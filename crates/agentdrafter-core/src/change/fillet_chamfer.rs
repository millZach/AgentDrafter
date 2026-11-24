//! Geometric fillet (`fillet` / `FILLET` / `F`) and chamfer (`chamfer` /
//! `CHAMFER` / `CHA`) between two objects or at every corner of a flat
//! polyline.

use std::f64::consts::{PI, TAU};

use crate::error::{Error, ErrorCode};
use crate::model::{Arc, Line, Object, Point3, Polyline};

const EPS: f64 = 1e-7;
const Z_TOL: f64 = 1e-6;

/// Which end of an object to keep when two objects cross or meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CornerEnd {
    Start,
    End,
}

impl CornerEnd {
    pub(crate) const ALL: [&'static str; 2] = ["start", "end"];

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "end" => Some(Self::End),
            _ => None,
        }
    }
}

/// How a chamfer's cutback is specified, matching AutoCAD's `CHAMFER` Distance
/// and Angle methods.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ChamferSpec {
    /// Cutback `d1` along the first object (or incoming polyline segment) and
    /// `d2` along the second object (or outgoing polyline segment). Both are
    /// finite and non-negative; `(0, 0)` brings two lines to a sharp corner.
    Distances(f64, f64),
    /// Cutback `length` along the first object (or incoming segment) and angle
    /// `angle_deg` in `(0, 180)` from the first line to the chamfer line.
    LengthAngle { length: f64, angle_deg: f64 },
}

/// Output of a two-object fillet or chamfer.
#[derive(Clone, Debug)]
pub(crate) struct TwoObjectResult {
    /// Updated `first` object, if trimmed/extended.
    pub(crate) first: Option<Object>,
    /// Updated `second` object, if trimmed/extended.
    pub(crate) second: Option<Object>,
    /// Newly created connecting arc (`fillet`) or line (`chamfer`), if any.
    pub(crate) added: Option<Object>,
}

/// Output of a polyline-wide fillet or chamfer.
#[derive(Clone, Debug)]
pub(crate) struct PolylineCornerResult {
    pub(crate) polyline: Object,
    pub(crate) corners_modified: usize,
    pub(crate) corners_skipped: usize,
}

#[derive(Clone, Copy, Debug)]
struct V2(f64, f64);

impl V2 {
    fn from_p3(p: Point3) -> Self {
        Self(p.x, p.y)
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

    fn perp_left(self) -> Self {
        Self(-self.1, self.0)
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

fn angle_deg(center: V2, p: V2) -> f64 {
    norm_deg((p.1 - center.1).atan2(p.0 - center.0).to_degrees())
}

fn arc_point(center: V2, radius: f64, deg: f64) -> V2 {
    let (s, c) = deg.to_radians().sin_cos();
    V2(center.0 + radius * c, center.1 + radius * s)
}

fn arc_sweep_deg(start_deg: f64, end_deg: f64) -> f64 {
    let d = (end_deg - start_deg).rem_euclid(360.0);
    if d <= 1e-9 {
        360.0
    } else {
        d
    }
}

fn angle_on_arc(deg: f64, start_deg: f64, end_deg: f64) -> bool {
    let sweep = arc_sweep_deg(start_deg, end_deg);
    let rel = (deg - start_deg).rem_euclid(360.0);
    rel <= sweep + 1e-4 || (360.0 - rel) <= 1e-4
}

/// Shortest distance from point `p` to finite segment `[a, b]`.
fn dist_point_to_segment(p: V2, a: V2, b: V2) -> f64 {
    let ab = b.sub(a);
    let len2 = ab.dot(ab);
    if len2 <= EPS * EPS {
        return p.dist(a);
    }
    let t = (p.sub(a).dot(ab) / len2).clamp(0.0, 1.0);
    p.dist(a.add(ab.scale(t)))
}

/// Shortest distance from point `p` to an arc.
fn dist_point_to_arc(p: V2, arc: &Arc) -> f64 {
    let c = V2::from_p3(arc.center);
    let deg = angle_deg(c, p);
    if angle_on_arc(deg, arc.start_angle, arc.end_angle) {
        (p.dist(c) - arc.radius).abs()
    } else {
        let p0 = arc_point(c, arc.radius, arc.start_angle);
        let p1 = arc_point(c, arc.radius, arc.end_angle);
        p.dist(p0).min(p.dist(p1))
    }
}

/// Intersection of infinite lines `(p1, d1)` and `(p2, d2)`.
fn line_line_intersection(p1: V2, d1: V2, p2: V2, d2: V2) -> Option<V2> {
    let denom = d1.cross(d2);
    if denom.abs() <= EPS * d1.len() * d2.len() {
        return None;
    }
    let t = p2.sub(p1).cross(d2) / denom;
    Some(p1.add(d1.scale(t)))
}

/// Intersection(s) of infinite line `(p, u)` (`|u| = 1`) with circle `(c, r)`.
fn line_circle_intersections(p: V2, u: V2, c: V2, r: f64) -> Vec<V2> {
    if r <= EPS {
        return Vec::new();
    }
    let pc = p.sub(c);
    let b = pc.dot(u);
    let c_term = pc.dot(pc) - r * r;
    let disc = b * b - c_term;
    if disc < -1e-6 * r * r {
        return Vec::new();
    }
    if disc <= 1e-10 * r * r {
        vec![p.add(u.scale(-b))]
    } else {
        let sd = disc.sqrt();
        vec![p.add(u.scale(-b - sd)), p.add(u.scale(-b + sd))]
    }
}

/// Intersection(s) of two circles `(c1, r1)` and `(c2, r2)`.
fn circle_circle_intersections(c1: V2, r1: f64, c2: V2, r2: f64) -> Vec<V2> {
    if r1 <= EPS || r2 <= EPS {
        return Vec::new();
    }
    let d_vec = c2.sub(c1);
    let d = d_vec.len();
    if d <= EPS {
        return Vec::new();
    }
    if d > r1 + r2 + 1e-5 || d < (r1 - r2).abs() - 1e-5 {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h2 = r1 * r1 - a * a;
    let u = d_vec.scale(1.0 / d);
    let mid = c1.add(u.scale(a));
    if h2 <= 1e-10 * r1 * r1 {
        vec![mid]
    } else {
        let h = h2.sqrt();
        let n = u.perp_left();
        vec![mid.add(n.scale(h)), mid.sub(n.scale(h))]
    }
}

/// Checks that a line is flat in Z and returns its `(from, to, unit_dir, z)`.
fn line_2d(line: &Line, label: &str) -> Result<(V2, V2, V2, f64), Error> {
    if (line.from.z - line.to.z).abs() > Z_TOL {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("{label} is not parallel to the XY plane"),
        ));
    }
    let a = V2::from_p3(line.from);
    let b = V2::from_p3(line.to);
    let Some(u) = b.sub(a).unit() else {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("{label} has zero length in the XY plane"),
        ));
    };
    Ok((a, b, u, line.from.z))
}

fn object_z(obj: &Object, label: &str) -> Result<f64, Error> {
    match obj {
        Object::Line(l) => {
            if (l.from.z - l.to.z).abs() > Z_TOL {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("{label} is not parallel to the XY plane"),
                ));
            }
            Ok(l.from.z)
        }
        Object::Arc(a) => Ok(a.center.z),
        Object::Circle(c) => Ok(c.center.z),
        Object::Polyline(p) => {
            if !p.is_flat() {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("{label} is not flat in Z"),
                ));
            }
            Ok(p.points.first().map_or(0.0, |pt| pt.z))
        }
        _ => Err(Error::new(
            ErrorCode::BadValue,
            format!("{label} is not a supported curve"),
        )),
    }
}

/// Chooses which endpoint of a line `[a, b]` stays fixed and which moves to the
/// corner/tangent point near `corner`.
/// Returns `(kept_endpoint, leg_unit_from_corner, keep_is_start, leg_length)`.
fn choose_line_leg(
    a: V2,
    b: V2,
    corner: V2,
    explicit_end: Option<CornerEnd>,
    label: &str,
) -> Result<(V2, V2, bool, f64), Error> {
    let da = a.dist(corner);
    let db = b.dist(corner);
    let keep_start = match explicit_end {
        Some(CornerEnd::Start) => true,
        Some(CornerEnd::End) => false,
        None => da > db + EPS,
    };
    let kept = if keep_start { a } else { b };
    let dist = kept.dist(corner);
    let Some(dir) = kept.sub(corner).unit() else {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "the kept end of `{label}` already lies at the corner intersection; \
                 choose the other end with `{label}_end`"
            ),
        ));
    };
    Ok((kept, dir, keep_start, dist))
}

fn updated_line(line: &Line, kept: V2, moved: V2, keep_is_start: bool, z: f64) -> Line {
    let (from, to) = if keep_is_start {
        (kept.to_p3(z), moved.to_p3(z))
    } else {
        (moved.to_p3(z), kept.to_p3(z))
    };
    Line {
        from,
        to,
        layer: line.layer.clone(),
        properties: line.properties.clone(),
    }
}

/// Builds the minor arc between `t1` and `t2` on circle `(center, radius)`.
fn minor_arc_between(
    center: V2,
    radius: f64,
    t1: V2,
    t2: V2,
    z: f64,
    layer: String,
    properties: crate::model::Properties,
) -> Arc {
    let a1 = angle_deg(center, t1);
    let a2 = angle_deg(center, t2);
    let ccw = (a2 - a1).rem_euclid(360.0);
    let (start_angle, end_angle) = if ccw <= 180.0 { (a1, a2) } else { (a2, a1) };
    Arc {
        center: center.to_p3(z),
        radius,
        start_angle,
        end_angle,
        layer,
        properties,
    }
}

// ---------------------------------------------------------------------------
// Two-object FILLET
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_fillet_two(
    first: &Object,
    second: &Object,
    radius: f64,
    trim: bool,
    first_end: Option<CornerEnd>,
    second_end: Option<CornerEnd>,
    layer_override: Option<&str>,
) -> Result<TwoObjectResult, Error> {
    if !radius.is_finite() || radius < 0.0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`radius` must be a non-negative length".to_owned(),
        ));
    }
    let z1 = object_z(first, "first")?;
    let z2 = object_z(second, "second")?;
    if (z1 - z2).abs() > Z_TOL {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`first` and `second` do not lie in the same Z plane".to_owned(),
        ));
    }
    let layer = layer_override.unwrap_or_else(|| first.layer()).to_owned();
    let props = first.properties().clone();

    match (first, second) {
        (Object::Line(l1), Object::Line(l2)) => fillet_line_line(
            l1, l2, radius, trim, first_end, second_end, z1, layer, props,
        ),
        (Object::Line(l), Object::Arc(a)) => fillet_line_arc_or_circle(
            l,
            V2::from_p3(a.center),
            a.radius,
            Some(a),
            false,
            radius,
            trim,
            first_end,
            second_end,
            z1,
            layer,
            props,
        ),
        (Object::Arc(a), Object::Line(l)) => fillet_line_arc_or_circle(
            l,
            V2::from_p3(a.center),
            a.radius,
            Some(a),
            true,
            radius,
            trim,
            second_end,
            first_end,
            z1,
            layer,
            props,
        ),
        (Object::Line(l), Object::Circle(c)) => fillet_line_arc_or_circle(
            l,
            V2::from_p3(c.center),
            c.radius,
            None,
            false,
            radius,
            trim,
            first_end,
            second_end,
            z1,
            layer,
            props,
        ),
        (Object::Circle(c), Object::Line(l)) => fillet_line_arc_or_circle(
            l,
            V2::from_p3(c.center),
            c.radius,
            None,
            true,
            radius,
            trim,
            second_end,
            first_end,
            z1,
            layer,
            props,
        ),
        (Object::Arc(a1), Object::Arc(a2)) => fillet_curve_curve(
            V2::from_p3(a1.center),
            a1.radius,
            Some(a1),
            V2::from_p3(a2.center),
            a2.radius,
            Some(a2),
            radius,
            trim,
            first_end,
            second_end,
            z1,
            layer,
            props,
        ),
        (Object::Arc(a1), Object::Circle(c2)) => fillet_curve_curve(
            V2::from_p3(a1.center),
            a1.radius,
            Some(a1),
            V2::from_p3(c2.center),
            c2.radius,
            None,
            radius,
            trim,
            first_end,
            second_end,
            z1,
            layer,
            props,
        ),
        (Object::Circle(c1), Object::Arc(a2)) => fillet_curve_curve(
            V2::from_p3(c1.center),
            c1.radius,
            None,
            V2::from_p3(a2.center),
            a2.radius,
            Some(a2),
            radius,
            trim,
            first_end,
            second_end,
            z1,
            layer,
            props,
        ),
        (Object::Circle(c1), Object::Circle(c2)) => fillet_curve_curve(
            V2::from_p3(c1.center),
            c1.radius,
            None,
            V2::from_p3(c2.center),
            c2.radius,
            None,
            radius,
            trim,
            first_end,
            second_end,
            z1,
            layer,
            props,
        ),
        (a, b) => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`fillet` between two objects works on lines, arcs and circles, not `{}` and `{}` \
                 (for a single polyline, pass it under `target`)",
                a.type_name(),
                b.type_name()
            ),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn fillet_line_line(
    l1: &Line,
    l2: &Line,
    radius: f64,
    trim: bool,
    first_end: Option<CornerEnd>,
    second_end: Option<CornerEnd>,
    z: f64,
    layer: String,
    properties: crate::model::Properties,
) -> Result<TwoObjectResult, Error> {
    let (a1, b1, u1, _) = line_2d(l1, "first")?;
    let (a2, b2, u2, _) = line_2d(l2, "second")?;

    let Some(corner) = line_line_intersection(a1, u1, a2, u2) else {
        // Parallel lines: AutoCAD creates a semicircular cap connecting them,
        // ignoring the specified radius (radius = half perpendicular distance).
        let n1 = u1.perp_left();
        let signed_sep = a2.sub(a1).dot(n1);
        let sep = signed_sep.abs();
        if sep <= EPS {
            return Err(Error::new(
                ErrorCode::BadValue,
                "lines are collinear; cannot fillet".to_owned(),
            ));
        }
        let use_start = matches!(first_end, Some(CornerEnd::Start));
        let e1 = if use_start { a1 } else { b1 };
        let outward = if use_start { u1.scale(-1.0) } else { u1 };
        let e2 = e1.add(n1.scale(signed_sep));
        let center = e1.add(e2).scale(0.5);
        let half_r = sep * 0.5;
        let mid_pt = center.add(outward.scale(half_r));
        let a_e1 = angle_deg(center, e1);
        let a_e2 = angle_deg(center, e2);
        let a_mid = angle_deg(center, mid_pt);
        let (start_angle, end_angle) = if angle_on_arc(a_mid, a_e1, a_e2) {
            (a_e1, a_e2)
        } else {
            (a_e2, a_e1)
        };
        let arc = Arc {
            center: center.to_p3(z),
            radius: half_r,
            start_angle,
            end_angle,
            layer,
            properties,
        };
        let second_out = if trim {
            let keep_s2 = match second_end {
                Some(CornerEnd::Start) => true,
                Some(CornerEnd::End) => false,
                None => a2.dist(e2) > b2.dist(e2) + EPS,
            };
            let kept2 = if keep_s2 { a2 } else { b2 };
            if kept2.dist(e2) <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "parallel fillet would collapse `second` to zero length".to_owned(),
                ));
            }
            Some(Object::Line(updated_line(l2, kept2, e2, keep_s2, z)))
        } else {
            None
        };
        return Ok(TwoObjectResult {
            first: None,
            second: second_out,
            added: Some(Object::Arc(arc)),
        });
    };

    let (kept1, dir1, keep_s1, len1) = choose_line_leg(a1, b1, corner, first_end, "first")?;
    let (kept2, dir2, keep_s2, len2) = choose_line_leg(a2, b2, corner, second_end, "second")?;

    if radius == 0.0 {
        if !trim {
            return Err(Error::new(
                ErrorCode::BadValue,
                "`fillet` with `radius: 0` and `trim: false` changes nothing".to_owned(),
            ));
        }
        return Ok(TwoObjectResult {
            first: Some(Object::Line(updated_line(l1, kept1, corner, keep_s1, z))),
            second: Some(Object::Line(updated_line(l2, kept2, corner, keep_s2, z))),
            added: None,
        });
    }

    let cos_theta = dir1.dot(dir2).clamp(-1.0, 1.0);
    let theta = cos_theta.acos();
    if theta <= EPS || (PI - theta).abs() <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "the two line legs are collinear; cannot fillet".to_owned(),
        ));
    }
    let half = theta * 0.5;
    let trim_dist = radius / half.tan();
    if trim && trim_dist >= len1 - EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "fillet radius {radius} requires trim distance {trim_dist:.4}, which exceeds \
                 `first`'s leg length {len1:.4}"
            ),
        ));
    }
    if trim && trim_dist >= len2 - EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "fillet radius {radius} requires trim distance {trim_dist:.4}, which exceeds \
                 `second`'s leg length {len2:.4}"
            ),
        ));
    }

    let t1 = corner.add(dir1.scale(trim_dist));
    let t2 = corner.add(dir2.scale(trim_dist));
    let bisector = dir1.add(dir2).unit().ok_or_else(|| {
        Error::new(
            ErrorCode::BadValue,
            "the two line legs are opposite; cannot fillet".to_owned(),
        )
    })?;
    let center = corner.add(bisector.scale(radius / half.sin()));
    let arc = minor_arc_between(center, radius, t1, t2, z, layer, properties);

    let (first_out, second_out) = if trim {
        (
            Some(Object::Line(updated_line(l1, kept1, t1, keep_s1, z))),
            Some(Object::Line(updated_line(l2, kept2, t2, keep_s2, z))),
        )
    } else {
        (None, None)
    };
    Ok(TwoObjectResult {
        first: first_out,
        second: second_out,
        added: Some(Object::Arc(arc)),
    })
}

/// Trims or extends an `Arc` so its nearer (or explicitly chosen) endpoint lands
/// at `target_pt` (which lies on the arc's circle).
fn trim_arc_to_point(
    arc: &Arc,
    target_pt: V2,
    explicit_end: Option<CornerEnd>,
) -> Result<Arc, Error> {
    let c = V2::from_p3(arc.center);
    let new_deg = angle_deg(c, target_pt);
    let p_start = arc_point(c, arc.radius, arc.start_angle);
    let p_end = arc_point(c, arc.radius, arc.end_angle);
    let keep_start = match explicit_end {
        Some(CornerEnd::Start) => true,
        Some(CornerEnd::End) => false,
        None => p_start.dist(target_pt) > p_end.dist(target_pt) + EPS,
    };
    let (start_angle, end_angle) = if keep_start {
        (arc.start_angle, new_deg)
    } else {
        (new_deg, arc.end_angle)
    };
    if (end_angle - start_angle).rem_euclid(360.0) <= 1e-4
        || (start_angle - end_angle).rem_euclid(360.0) <= 1e-4
    {
        return Err(Error::new(
            ErrorCode::BadValue,
            "fillet would collapse the arc to zero length".to_owned(),
        ));
    }
    Ok(Arc {
        center: arc.center,
        radius: arc.radius,
        start_angle,
        end_angle,
        layer: arc.layer.clone(),
        properties: arc.properties.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn fillet_line_arc_or_circle(
    line: &Line,
    c0: V2,
    r0: f64,
    arc_opt: Option<&Arc>,
    swapped: bool,
    radius: f64,
    trim: bool,
    line_end: Option<CornerEnd>,
    arc_end: Option<CornerEnd>,
    z: f64,
    layer: String,
    properties: crate::model::Properties,
) -> Result<TwoObjectResult, Error> {
    let (a, b, u, _) = line_2d(line, if swapped { "second" } else { "first" })?;
    let n = u.perp_left();

    if radius == 0.0 {
        if !trim {
            return Err(Error::new(
                ErrorCode::BadValue,
                "`fillet` with `radius: 0` and `trim: false` changes nothing".to_owned(),
            ));
        }
        let hits = line_circle_intersections(a, u, c0, r0);
        if hits.is_empty() {
            return Err(Error::new(
                ErrorCode::BadValue,
                "line and arc/circle do not intersect; cannot fillet with radius 0".to_owned(),
            ));
        }
        let best = hits
            .into_iter()
            .min_by(|&p1, &p2| {
                let s1 = dist_point_to_segment(p1, a, b)
                    + arc_opt.map_or(0.0, |arc| dist_point_to_arc(p1, arc));
                let s2 = dist_point_to_segment(p2, a, b)
                    + arc_opt.map_or(0.0, |arc| dist_point_to_arc(p2, arc));
                s1.total_cmp(&s2)
            })
            .unwrap();
        let (kept, _, keep_s, _) = choose_line_leg(
            a,
            b,
            best,
            line_end,
            if swapped { "second" } else { "first" },
        )?;
        let new_line = Some(Object::Line(updated_line(line, kept, best, keep_s, z)));
        let new_arc = match arc_opt {
            Some(arc) => Some(Object::Arc(trim_arc_to_point(arc, best, arc_end)?)),
            None => None,
        };
        return Ok(if swapped {
            TwoObjectResult {
                first: new_arc,
                second: new_line,
                added: None,
            }
        } else {
            TwoObjectResult {
                first: new_line,
                second: new_arc,
                added: None,
            }
        });
    }

    // Offset line by +/- radius and intersect with offset circles r0 + radius
    // and |r0 - radius|.
    let mut candidates: Vec<(V2, V2, V2, f64)> = Vec::new();
    let offset_radii: Vec<f64> = [r0 + radius, (r0 - radius).abs()]
        .into_iter()
        .filter(|&rr| rr > EPS)
        .collect();
    for sign in [1.0, -1.0] {
        let p_off = a.add(n.scale(sign * radius));
        for &rr in &offset_radii {
            for center in line_circle_intersections(p_off, u, c0, rr) {
                let t_line = a.add(u.scale(center.sub(a).dot(u)));
                let Some(dir_c) = center.sub(c0).unit() else {
                    continue;
                };
                // Pick whichever point on circle (c0, r0) along c0->center is
                // at distance `radius` from `center`.
                let t_plus = c0.add(dir_c.scale(r0));
                let t_minus = c0.sub(dir_c.scale(r0));
                let t_arc = if (t_plus.dist(center) - radius).abs()
                    <= (t_minus.dist(center) - radius).abs()
                {
                    t_plus
                } else {
                    t_minus
                };
                if (t_arc.dist(center) - radius).abs() > 1e-4 {
                    continue;
                }
                if t_line.dist(t_arc) <= EPS {
                    continue;
                }
                let score = dist_point_to_segment(t_line, a, b)
                    + arc_opt.map_or(0.0, |arc| dist_point_to_arc(t_arc, arc))
                    + 0.01 * t_line.dist(t_arc);
                candidates.push((center, t_line, t_arc, score));
            }
        }
    }

    let Some((center, t_line, t_arc, _)) =
        candidates.into_iter().min_by(|a, b| a.3.total_cmp(&b.3))
    else {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("no fillet arc of radius {radius} is tangent to both objects"),
        ));
    };

    let fillet_arc = minor_arc_between(center, radius, t_line, t_arc, z, layer, properties);
    let (new_line, new_arc) = if trim {
        let (kept, _, keep_s, _) = choose_line_leg(
            a,
            b,
            t_line,
            line_end,
            if swapped { "second" } else { "first" },
        )?;
        let nl = Some(Object::Line(updated_line(line, kept, t_line, keep_s, z)));
        let na = match arc_opt {
            Some(arc) => Some(Object::Arc(trim_arc_to_point(arc, t_arc, arc_end)?)),
            None => None,
        };
        (nl, na)
    } else {
        (None, None)
    };
    Ok(if swapped {
        TwoObjectResult {
            first: new_arc,
            second: new_line,
            added: Some(Object::Arc(fillet_arc)),
        }
    } else {
        TwoObjectResult {
            first: new_line,
            second: new_arc,
            added: Some(Object::Arc(fillet_arc)),
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn fillet_curve_curve(
    c1: V2,
    r1: f64,
    a1_opt: Option<&Arc>,
    c2: V2,
    r2: f64,
    a2_opt: Option<&Arc>,
    radius: f64,
    trim: bool,
    first_end: Option<CornerEnd>,
    second_end: Option<CornerEnd>,
    z: f64,
    layer: String,
    properties: crate::model::Properties,
) -> Result<TwoObjectResult, Error> {
    if radius == 0.0 {
        if !trim {
            return Err(Error::new(
                ErrorCode::BadValue,
                "`fillet` with `radius: 0` and `trim: false` changes nothing".to_owned(),
            ));
        }
        let hits = circle_circle_intersections(c1, r1, c2, r2);
        if hits.is_empty() {
            return Err(Error::new(
                ErrorCode::BadValue,
                "arcs/circles do not intersect; cannot fillet with radius 0".to_owned(),
            ));
        }
        let best = hits
            .into_iter()
            .min_by(|&p1, &p2| {
                let s1 = a1_opt.map_or(0.0, |a| dist_point_to_arc(p1, a))
                    + a2_opt.map_or(0.0, |a| dist_point_to_arc(p1, a));
                let s2 = a1_opt.map_or(0.0, |a| dist_point_to_arc(p2, a))
                    + a2_opt.map_or(0.0, |a| dist_point_to_arc(p2, a));
                s1.total_cmp(&s2)
            })
            .unwrap();
        let first_out = match a1_opt {
            Some(a) => Some(Object::Arc(trim_arc_to_point(a, best, first_end)?)),
            None => None,
        };
        let second_out = match a2_opt {
            Some(a) => Some(Object::Arc(trim_arc_to_point(a, best, second_end)?)),
            None => None,
        };
        return Ok(TwoObjectResult {
            first: first_out,
            second: second_out,
            added: None,
        });
    }

    let r1s: Vec<f64> = [r1 + radius, (r1 - radius).abs()]
        .into_iter()
        .filter(|&rr| rr > EPS)
        .collect();
    let r2s: Vec<f64> = [r2 + radius, (r2 - radius).abs()]
        .into_iter()
        .filter(|&rr| rr > EPS)
        .collect();
    let mut candidates: Vec<(V2, V2, V2, f64)> = Vec::new();
    for &rr1 in &r1s {
        for &rr2 in &r2s {
            for center in circle_circle_intersections(c1, rr1, c2, rr2) {
                let (Some(d1), Some(d2)) = (center.sub(c1).unit(), center.sub(c2).unit()) else {
                    continue;
                };
                let pick_t = |c: V2, r: f64, d: V2| -> Option<V2> {
                    let tp = c.add(d.scale(r));
                    let tm = c.sub(d.scale(r));
                    let t = if (tp.dist(center) - radius).abs() <= (tm.dist(center) - radius).abs()
                    {
                        tp
                    } else {
                        tm
                    };
                    ((t.dist(center) - radius).abs() <= 1e-4).then_some(t)
                };
                let (Some(t1), Some(t2)) = (pick_t(c1, r1, d1), pick_t(c2, r2, d2)) else {
                    continue;
                };
                if t1.dist(t2) <= EPS {
                    continue;
                }
                let score = a1_opt.map_or(0.0, |a| dist_point_to_arc(t1, a))
                    + a2_opt.map_or(0.0, |a| dist_point_to_arc(t2, a))
                    + 0.01 * t1.dist(t2);
                candidates.push((center, t1, t2, score));
            }
        }
    }
    let Some((center, t1, t2, _)) = candidates.into_iter().min_by(|a, b| a.3.total_cmp(&b.3))
    else {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("no fillet arc of radius {radius} is tangent to both curves"),
        ));
    };
    let fillet_arc = minor_arc_between(center, radius, t1, t2, z, layer, properties);
    let (first_out, second_out) = if trim {
        let f = match a1_opt {
            Some(a) => Some(Object::Arc(trim_arc_to_point(a, t1, first_end)?)),
            None => None,
        };
        let s = match a2_opt {
            Some(a) => Some(Object::Arc(trim_arc_to_point(a, t2, second_end)?)),
            None => None,
        };
        (f, s)
    } else {
        (None, None)
    };
    Ok(TwoObjectResult {
        first: first_out,
        second: second_out,
        added: Some(Object::Arc(fillet_arc)),
    })
}

// ---------------------------------------------------------------------------
// Two-object CHAMFER
// ---------------------------------------------------------------------------

fn chamfer_distances_for_angle(spec: ChamferSpec, theta: f64) -> Result<(f64, f64), Error> {
    match spec {
        ChamferSpec::Distances(d1, d2) => Ok((d1, d2)),
        ChamferSpec::LengthAngle { length, angle_deg } => {
            let alpha = angle_deg.to_radians();
            let gamma = PI - theta - alpha;
            if gamma <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "chamfer angle {angle_deg} deg does not meet the second line at a \
                         {:.2} deg corner (corner angle + chamfer angle must be less than 180 deg)",
                        theta.to_degrees()
                    ),
                ));
            }
            let d2 = length * alpha.sin() / gamma.sin();
            Ok((length, d2))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_chamfer_two(
    first: &Object,
    second: &Object,
    spec: ChamferSpec,
    trim: bool,
    first_end: Option<CornerEnd>,
    second_end: Option<CornerEnd>,
    layer_override: Option<&str>,
) -> Result<TwoObjectResult, Error> {
    let (Object::Line(l1), Object::Line(l2)) = (first, second) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`chamfer` between two objects works on two lines, not `{}` and `{}` \
                 (for a single polyline, pass it under `target`)",
                first.type_name(),
                second.type_name()
            ),
        ));
    };
    let (a1, b1, u1, z1) = line_2d(l1, "first")?;
    let (a2, b2, u2, z2) = line_2d(l2, "second")?;
    if (z1 - z2).abs() > Z_TOL {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`first` and `second` do not lie in the same Z plane".to_owned(),
        ));
    }
    let Some(corner) = line_line_intersection(a1, u1, a2, u2) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            "lines are parallel; cannot chamfer".to_owned(),
        ));
    };

    let (kept1, dir1, keep_s1, len1) = choose_line_leg(a1, b1, corner, first_end, "first")?;
    let (kept2, dir2, keep_s2, len2) = choose_line_leg(a2, b2, corner, second_end, "second")?;

    let theta = dir1.dot(dir2).clamp(-1.0, 1.0).acos();
    if theta <= EPS || (PI - theta).abs() <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "the two line legs are collinear; cannot chamfer".to_owned(),
        ));
    }

    let (d1, d2) = chamfer_distances_for_angle(spec, theta)?;
    if d1 == 0.0 && d2 == 0.0 {
        if !trim {
            return Err(Error::new(
                ErrorCode::BadValue,
                "`chamfer` with zero distances and `trim: false` changes nothing".to_owned(),
            ));
        }
        return Ok(TwoObjectResult {
            first: Some(Object::Line(updated_line(l1, kept1, corner, keep_s1, z1))),
            second: Some(Object::Line(updated_line(l2, kept2, corner, keep_s2, z1))),
            added: None,
        });
    }

    if trim && d1 >= len1 - EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("chamfer distance {d1:.4} exceeds `first`'s leg length {len1:.4}"),
        ));
    }
    if trim && d2 >= len2 - EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("chamfer distance {d2:.4} exceeds `second`'s leg length {len2:.4}"),
        ));
    }

    let c1 = corner.add(dir1.scale(d1));
    let c2 = corner.add(dir2.scale(d2));
    if c1.dist(c2) <= EPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            "chamfer produces a zero-length bevel".to_owned(),
        ));
    }

    let layer = layer_override.unwrap_or(&l1.layer).to_owned();
    let bevel = Line {
        from: c1.to_p3(z1),
        to: c2.to_p3(z1),
        layer,
        properties: l1.properties.clone(),
    };
    let (first_out, second_out) = if trim {
        (
            Some(Object::Line(updated_line(l1, kept1, c1, keep_s1, z1))),
            Some(Object::Line(updated_line(l2, kept2, c2, keep_s2, z1))),
        )
    } else {
        (None, None)
    };
    Ok(TwoObjectResult {
        first: first_out,
        second: second_out,
        added: Some(Object::Line(bevel)),
    })
}

// ---------------------------------------------------------------------------
// Polyline FILLET & CHAMFER
// ---------------------------------------------------------------------------

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Removes existing arc (fillet) segments that sit between two straight
/// segments by extending those straight segments to their sharp corner
/// intersection (AutoCAD's `FILLET Polyline` with radius 0).
fn square_polyline_arc_corners(poly: &Polyline) -> Result<PolylineCornerResult, Error> {
    let n = poly.points.len();
    let seg_count = if poly.closed { n } else { n.saturating_sub(1) };
    if seg_count < 3 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "polyline has no arc corners between straight segments to square up".to_owned(),
        ));
    }
    let z = poly.points[0].z;
    let pts: Vec<V2> = poly.points.iter().copied().map(V2::from_p3).collect();
    let has_widths = !poly.widths.is_empty();

    // Represent segments as (start_pt, end_pt, bulge, width_pair).
    #[derive(Clone, Copy)]
    struct Seg {
        a: V2,
        b: V2,
        bulge: f64,
        w: [f64; 2],
    }
    let mut segs: Vec<Seg> = (0..seg_count)
        .map(|i| Seg {
            a: pts[i],
            b: pts[(i + 1) % n],
            bulge: poly.bulge(i),
            w: poly.widths_at(i),
        })
        .collect();

    let mut remove = vec![false; seg_count];
    let mut modified = 0usize;
    let mut skipped = 0usize;
    for i in 0..seg_count {
        if segs[i].bulge.abs() <= 1e-12 {
            continue;
        }
        if !poly.closed && (i == 0 || i + 1 >= seg_count) {
            skipped += 1;
            continue;
        }
        let prev = (i + seg_count - 1) % seg_count;
        let next = (i + 1) % seg_count;
        if segs[prev].bulge.abs() > 1e-12 || segs[next].bulge.abs() > 1e-12 {
            skipped += 1;
            continue;
        }
        let (Some(u1), Some(u2)) = (
            segs[prev].b.sub(segs[prev].a).unit(),
            segs[next].b.sub(segs[next].a).unit(),
        ) else {
            skipped += 1;
            continue;
        };
        let Some(corner) = line_line_intersection(segs[prev].a, u1, segs[next].a, u2) else {
            skipped += 1;
            continue;
        };
        segs[prev].b = corner;
        segs[next].a = corner;
        remove[i] = true;
        modified += 1;
    }

    if modified == 0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "polyline has no arc corners between straight segments to square up".to_owned(),
        ));
    }

    let kept: Vec<Seg> = segs
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !remove[*i])
        .map(|(_, s)| s)
        .collect();
    let mut out_pts: Vec<Point3> = kept.iter().map(|s| s.a.to_p3(z)).collect();
    if !poly.closed {
        out_pts.push(kept.last().unwrap().b.to_p3(z));
    }
    let out_bulges: Vec<f64> = if kept.iter().any(|s| s.bulge.abs() > 1e-12) {
        let mut bs: Vec<f64> = kept.iter().map(|s| s.bulge).collect();
        if !poly.closed {
            bs.push(0.0);
        }
        bs
    } else {
        Vec::new()
    };
    let out_widths: Vec<[f64; 2]> = if has_widths {
        kept.iter().map(|s| s.w).collect()
    } else {
        Vec::new()
    };

    Ok(PolylineCornerResult {
        polyline: Object::Polyline(Polyline {
            points: out_pts,
            bulges: out_bulges,
            widths: out_widths,
            width: poly.width,
            closed: poly.closed,
            layer: poly.layer.clone(),
            properties: poly.properties.clone(),
        }),
        corners_modified: modified,
        corners_skipped: skipped,
    })
}

/// Applies a corner modification (fillet arc or chamfer bevel) to every
/// eligible straight-to-straight corner of a flat polyline.
fn modify_polyline_corners(
    target: &Object,
    op_name: &str,
    corner_fn: impl Fn(f64, f64) -> Result<Option<(f64, f64, f64)>, Error>,
) -> Result<PolylineCornerResult, Error> {
    let Object::Polyline(poly) = target else {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "`{op_name}` with `target` expects a polyline, not a {}",
                target.type_name()
            ),
        ));
    };
    if !poly.is_flat() {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("`{op_name}` requires a flat polyline (constant Z)"),
        ));
    }
    let n = poly.points.len();
    let seg_count = if poly.closed { n } else { n.saturating_sub(1) };
    if seg_count < 2 {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("polyline has no corners to {op_name}"),
        ));
    }
    let z = poly.points[0].z;
    let pts: Vec<V2> = poly.points.iter().copied().map(V2::from_p3).collect();
    let has_widths = !poly.widths.is_empty();

    // Candidate corner edits indexed by vertex index `v` (0..n).
    // At vertex `v`, incoming segment is `prev_seg`, outgoing segment is `next_seg`.
    #[derive(Clone, Copy)]
    struct CornerCut {
        d_in: f64,
        d_out: f64,
        /// Bulge for the inserted corner segment (0.0 for chamfer, +/-tan(turn/4) for fillet).
        bulge: f64,
    }

    let seg_lengths: Vec<f64> = (0..seg_count)
        .map(|i| pts[i].dist(pts[(i + 1) % n]))
        .collect();

    let mut cuts: Vec<Option<CornerCut>> = vec![None; n];
    let mut skipped = 0usize;

    let corner_indices: Vec<usize> = if poly.closed {
        (0..n).collect()
    } else {
        (1..n - 1).collect()
    };

    for &v in &corner_indices {
        let prev_seg = (v + n - 1) % n;
        let next_seg = v;
        if poly.bulge(prev_seg).abs() > 1e-12 || poly.bulge(next_seg).abs() > 1e-12 {
            // Skip corners adjacent to arc segments.
            skipped += 1;
            continue;
        }
        let p_prev = pts[prev_seg];
        let p_v = pts[v];
        let p_next = pts[(v + 1) % n];
        let (Some(u_in), Some(u_out)) = (p_v.sub(p_prev).unit(), p_next.sub(p_v).unit()) else {
            skipped += 1;
            continue;
        };
        let a = u_in.scale(-1.0); // away from v back along incoming segment
        let b = u_out; // away from v forward along outgoing segment
        let cos_theta = a.dot(b).clamp(-1.0, 1.0);
        let theta = cos_theta.acos();
        if theta <= EPS || (PI - theta).abs() <= EPS {
            // Collinear or 180-degree reversal: nothing to fillet/chamfer.
            continue;
        }
        let cross = u_in.cross(u_out);
        let Some((d_in, d_out, bulge)) = corner_fn(theta, cross)? else {
            continue;
        };
        if d_in <= 0.0 && d_out <= 0.0 {
            continue;
        }
        if d_in > seg_lengths[prev_seg] + EPS || d_out > seg_lengths[next_seg] + EPS {
            skipped += 1;
            continue;
        }
        cuts[v] = Some(CornerCut { d_in, d_out, bulge });
    }

    // Check segment budgets: for every segment `i` from vertex `i` to `(i+1)%n`,
    // `cut[i].d_out + cut[(i+1)%n].d_in` must not exceed `seg_lengths[i]`.
    // If a segment is oversubscribed, drop the larger-demand end (or both) just
    // as AutoCAD skips corners whose segment is too short.
    for (i, &seg_len) in seg_lengths.iter().enumerate() {
        let v_start = i;
        let v_end = (i + 1) % n;
        let d_start = cuts[v_start].map_or(0.0, |c| c.d_out);
        let d_end = cuts[v_end].map_or(0.0, |c| c.d_in);
        if d_start + d_end > seg_len + EPS {
            if cuts[v_end].is_some() {
                cuts[v_end] = None;
                skipped += 1;
            }
            let d_start_now = cuts[v_start].map_or(0.0, |c| c.d_out);
            if d_start_now > seg_len + EPS && cuts[v_start].is_some() {
                cuts[v_start] = None;
                skipped += 1;
            }
        }
    }

    let modified = cuts.iter().filter(|c| c.is_some()).count();
    if modified == 0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "no corners of the polyline could be {op_name}ed \
                 (segments may be too short, curved, or collinear)"
            ),
        ));
    }

    // Reconstruct vertices, bulges, and widths in order.
    let mut raw_pts: Vec<V2> = Vec::with_capacity(n + modified);
    let mut raw_bulges: Vec<f64> = Vec::with_capacity(n + modified);
    let mut raw_widths: Vec<[f64; 2]> = Vec::new();

    for i in 0..seg_count {
        let v_start = i;
        let v_end = (i + 1) % n;
        let p_start = pts[v_start];
        let p_end = pts[v_end];
        let len = seg_lengths[i];
        let w_orig = poly.widths_at(i);

        let t0 = cuts[v_start].map_or(0.0, |c| (c.d_out / len).clamp(0.0, 1.0));
        let t1 = cuts[v_end].map_or(1.0, |c| (1.0 - c.d_in / len).clamp(0.0, 1.0));

        let seg_a = p_start.add(p_end.sub(p_start).scale(t0));
        let seg_b = p_start.add(p_end.sub(p_start).scale(t1));
        let w0 = lerp(w_orig[0], w_orig[1], t0);
        let w1 = lerp(w_orig[0], w_orig[1], t1);

        // Emit the (possibly shortened) original segment if non-degenerate.
        if seg_a.dist(seg_b) > EPS {
            raw_pts.push(seg_a);
            raw_bulges.push(poly.bulge(i));
            if has_widths {
                raw_widths.push([w0, w1]);
            }
        }

        // Emit the corner arc/bevel at `v_end` if cut.
        if let Some(cut) = cuts[v_end] {
            let next_seg = v_end;
            let next_len = seg_lengths[next_seg];
            let next_w = poly.widths_at(next_seg);
            let t_next = (cut.d_out / next_len).clamp(0.0, 1.0);
            let w_end = lerp(next_w[0], next_w[1], t_next);
            raw_pts.push(seg_b);
            raw_bulges.push(cut.bulge);
            if has_widths {
                raw_widths.push([w1, w_end]);
            }
        }
    }

    if !poly.closed {
        raw_pts.push(*pts.last().unwrap());
        raw_bulges.push(0.0);
    }

    let out_pts: Vec<Point3> = raw_pts.into_iter().map(|p| p.to_p3(z)).collect();
    let out_bulges = if raw_bulges.iter().any(|b| b.abs() > 1e-12) {
        raw_bulges
    } else {
        Vec::new()
    };
    let out_widths = if has_widths { raw_widths } else { Vec::new() };

    Ok(PolylineCornerResult {
        polyline: Object::Polyline(Polyline {
            points: out_pts,
            bulges: out_bulges,
            widths: out_widths,
            width: poly.width,
            closed: poly.closed,
            layer: poly.layer.clone(),
            properties: poly.properties.clone(),
        }),
        corners_modified: modified,
        corners_skipped: skipped,
    })
}

pub(crate) fn compute_fillet_polyline(
    target: &Object,
    radius: f64,
) -> Result<PolylineCornerResult, Error> {
    if !radius.is_finite() || radius < 0.0 {
        return Err(Error::new(
            ErrorCode::BadValue,
            "`radius` must be a non-negative length".to_owned(),
        ));
    }
    if radius == 0.0 {
        let Object::Polyline(poly) = target else {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`fillet` with `target` expects a polyline, not a {}",
                    target.type_name()
                ),
            ));
        };
        if !poly.is_flat() {
            return Err(Error::new(
                ErrorCode::BadValue,
                "`fillet` requires a flat polyline (constant Z)".to_owned(),
            ));
        }
        return square_polyline_arc_corners(poly);
    }

    modify_polyline_corners(target, "fillet", |theta, cross| {
        let half = theta * 0.5;
        let trim_dist = radius / half.tan();
        let turn = PI - theta; // sweep angle of fillet arc in (0, PI)
        let bulge = (turn * 0.25).tan() * if cross >= 0.0 { 1.0 } else { -1.0 };
        // Guard against numerical drift past TAU.
        let _ = TAU;
        Ok(Some((trim_dist, trim_dist, bulge)))
    })
}

pub(crate) fn compute_chamfer_polyline(
    target: &Object,
    spec: ChamferSpec,
) -> Result<PolylineCornerResult, Error> {
    if matches!(spec, ChamferSpec::Distances(0.0, 0.0)) {
        let Object::Polyline(poly) = target else {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`chamfer` with `target` expects a polyline, not a {}",
                    target.type_name()
                ),
            ));
        };
        if !poly.is_flat() {
            return Err(Error::new(
                ErrorCode::BadValue,
                "`chamfer` requires a flat polyline (constant Z)".to_owned(),
            ));
        }
        return square_polyline_arc_corners(poly);
    }

    modify_polyline_corners(target, "chamfer", |theta, _cross| {
        let (d1, d2) = chamfer_distances_for_angle(spec, theta)?;
        Ok(Some((d1, d2, 0.0)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::polyline::arc_of;
    use crate::model::{Circle, Properties};

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
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
    fn fillet_two_lines_radius_positive_and_zero_and_no_trim() {
        // Two wires not yet meeting: (0,0)->(7,0) and (10,3)->(10,10).
        let l1 = line(0.0, 0.0, 7.0, 0.0);
        let l2 = line(10.0, 3.0, 10.0, 10.0);

        // Radius 0 squares them up cleanly at (10, 0).
        let sq = compute_fillet_two(&l1, &l2, 0.0, true, None, None, None).unwrap();
        assert!(sq.added.is_none());
        let Some(Object::Line(f1)) = sq.first else {
            panic!()
        };
        let Some(Object::Line(f2)) = sq.second else {
            panic!()
        };
        close(f1.to.x, 10.0);
        close(f1.to.y, 0.0);
        close(f2.from.x, 10.0);
        close(f2.from.y, 0.0);

        // Radius 2 creates tangent arc centered at (8, 2), trimming/extending to (8, 0) & (10, 2).
        let r2 = compute_fillet_two(&l1, &l2, 2.0, true, None, None, Some("bends")).unwrap();
        let Some(Object::Line(f1)) = r2.first else {
            panic!()
        };
        let Some(Object::Line(f2)) = r2.second else {
            panic!()
        };
        let Some(Object::Arc(arc)) = r2.added else {
            panic!()
        };
        close(f1.to.x, 8.0);
        close(f1.to.y, 0.0);
        close(f2.from.x, 10.0);
        close(f2.from.y, 2.0);
        close(arc.center.x, 8.0);
        close(arc.center.y, 2.0);
        close(arc.radius, 2.0);
        close(arc.start_angle, 270.0);
        close(arc.end_angle, 0.0);
        assert_eq!(arc.layer, "bends");

        // No-trim leaves first/second untouched and only returns the arc.
        let nt = compute_fillet_two(&l1, &l2, 2.0, false, None, None, None).unwrap();
        assert!(nt.first.is_none());
        assert!(nt.second.is_none());
        assert!(matches!(nt.added, Some(Object::Arc(_))));
    }

    #[test]
    fn fillet_parallel_lines_creates_semicircular_cap() {
        let l1 = line(0.0, 0.0, 10.0, 0.0);
        let l2 = line(0.0, 4.0, 8.0, 4.0);
        let res = compute_fillet_two(&l1, &l2, 1.0, true, None, None, None).unwrap();
        let Some(Object::Arc(arc)) = res.added else {
            panic!()
        };
        close(arc.center.x, 10.0);
        close(arc.center.y, 2.0);
        close(arc.radius, 2.0);
        close(arc.start_angle, 270.0);
        close(arc.end_angle, 90.0);
        let Some(Object::Line(l2_new)) = res.second else {
            panic!()
        };
        close(l2_new.to.x, 10.0);
        close(l2_new.to.y, 4.0);
    }

    #[test]
    fn fillet_line_and_arc_tangent() {
        // Horizontal line y = 0 from (0, 0) to (10, 0), arc centered at (10, 6) r = 5 (180..270 deg).
        // Line intersects the arc's circle; radius 2 fillet sits tangent to y=0 (center y=2)
        // and to the arc (distance 5+2=7 or 5-2=3 from (10, 6)).
        let l = line(0.0, 0.0, 10.0, 0.0);
        let a = Object::Arc(Arc {
            center: Point3::new(10.0, 6.0, 0.0),
            radius: 5.0,
            start_angle: 180.0,
            end_angle: 270.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let res = compute_fillet_two(&l, &a, 2.0, true, None, None, None).unwrap();
        let Some(Object::Arc(fillet)) = res.added else {
            panic!()
        };
        assert!(res.first.is_some());
        assert!(res.second.is_some());
        close(fillet.radius, 2.0);
        // Fillet arc center is at distance 2 from line y = 0.
        close(fillet.center.y.abs(), 2.0);
        // And tangent to circle (10, 6, r=5): distance is 5+2=7 or 5-2=3.
        let dc = (fillet.center.x - 10.0).hypot(fillet.center.y - 6.0);
        assert!(
            (dc - 7.0).abs() < 1e-5 || (dc - 3.0).abs() < 1e-5,
            "unexpected center distance {dc}"
        );
    }

    #[test]
    fn fillet_line_and_circle_exact_tangency() {
        // Horizontal line y = 0, circle centered at (10, 6) r = 4 (lowest point y = 2).
        // Fillet of radius 3: center at y = 3, distance to (10, 6) = 4 + 3 = 7 => dx = sqrt(49 - 9) = sqrt(40).
        let l = line(0.0, 0.0, 10.0, 0.0);
        let c = Object::Circle(Circle {
            center: Point3::new(10.0, 6.0, 0.0),
            radius: 4.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let res = compute_fillet_two(&l, &c, 3.0, true, None, None, None).unwrap();
        let Some(Object::Arc(fillet)) = res.added else {
            panic!()
        };
        assert!(res.second.is_none(), "circle is not trimmed by fillet");
        close(fillet.radius, 3.0);
        close(fillet.center.y, 3.0);
        let d_centers = (fillet.center.x - 10.0).hypot(fillet.center.y - 6.0);
        close(d_centers, 7.0);
    }

    #[test]
    fn chamfer_two_lines_distances_angle_and_zero() {
        let l1 = line(0.0, 0.0, 10.0, 0.0);
        let l2 = line(10.0, 0.0, 10.0, 10.0);

        // Asymmetric distances (3 on first, 2 on second).
        let res = compute_chamfer_two(
            &l1,
            &l2,
            ChamferSpec::Distances(3.0, 2.0),
            true,
            None,
            None,
            None,
        )
        .unwrap();
        let Some(Object::Line(f1)) = res.first else {
            panic!()
        };
        let Some(Object::Line(f2)) = res.second else {
            panic!()
        };
        let Some(Object::Line(bevel)) = res.added else {
            panic!()
        };
        close(f1.to.x, 7.0);
        close(f1.to.y, 0.0);
        close(f2.from.x, 10.0);
        close(f2.from.y, 2.0);
        close(bevel.from.x, 7.0);
        close(bevel.from.y, 0.0);
        close(bevel.to.x, 10.0);
        close(bevel.to.y, 2.0);

        // Angle method: length = 4 along first, angle = 30 deg -> d2 = 4 * tan(30 deg).
        let ang = compute_chamfer_two(
            &l1,
            &l2,
            ChamferSpec::LengthAngle {
                length: 4.0,
                angle_deg: 30.0,
            },
            true,
            None,
            None,
            None,
        )
        .unwrap();
        let Some(Object::Line(bevel2)) = ang.added else {
            panic!()
        };
        close(bevel2.from.x, 6.0);
        close(bevel2.to.y, 4.0 * 30.0_f64.to_radians().tan());

        // Zero distances bring disjoint lines to corner.
        let s1 = line(0.0, 0.0, 6.0, 0.0);
        let s2 = line(10.0, 4.0, 10.0, 10.0);
        let z = compute_chamfer_two(
            &s1,
            &s2,
            ChamferSpec::Distances(0.0, 0.0),
            true,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(z.added.is_none());
        let Some(Object::Line(zf1)) = z.first else {
            panic!()
        };
        close(zf1.to.x, 10.0);
        close(zf1.to.y, 0.0);
    }

    #[test]
    fn polyline_fillet_all_corners_and_square_back_with_radius_zero() {
        // Closed 40x20 enclosure rectangle.
        let rect = Object::Polyline(Polyline::through(
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(40.0, 0.0, 0.0),
                Point3::new(40.0, 20.0, 0.0),
                Point3::new(0.0, 20.0, 0.0),
            ],
            true,
            "enclosure",
        ));
        let filleted = compute_fillet_polyline(&rect, 5.0).unwrap();
        assert_eq!(filleted.corners_modified, 4);
        assert_eq!(filleted.corners_skipped, 0);
        let Object::Polyline(ref p) = filleted.polyline else {
            panic!()
        };
        assert!(p.closed);
        assert_eq!(p.points.len(), 8);
        // Check every arc segment has radius 5.
        let mut arc_count = 0;
        for i in 0..p.points.len() {
            let b = p.bulge(i);
            if b.abs() > 1e-12 {
                let a = p.points[i];
                let q = p.points[(i + 1) % p.points.len()];
                let sa = arc_of([a.x, a.y], [q.x, q.y], b).unwrap();
                close(sa.radius, 5.0);
                arc_count += 1;
            }
        }
        assert_eq!(arc_count, 4);

        // Filleting the rounded rectangle with radius 0 squares all 4 corners back!
        let squared = compute_fillet_polyline(&filleted.polyline, 0.0).unwrap();
        assert_eq!(squared.corners_modified, 4);
        let Object::Polyline(ref sp) = squared.polyline else {
            panic!()
        };
        assert_eq!(sp.points.len(), 4);
        assert!(sp.bulges.is_empty());
    }

    #[test]
    fn polyline_chamfer_open_wire_and_closed_enclosure() {
        let rect = Object::Polyline(Polyline::through(
            vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(40.0, 0.0, 0.0),
                Point3::new(40.0, 20.0, 0.0),
                Point3::new(0.0, 20.0, 0.0),
            ],
            true,
            "enclosure",
        ));
        let ch = compute_chamfer_polyline(&rect, ChamferSpec::Distances(3.0, 3.0)).unwrap();
        assert_eq!(ch.corners_modified, 4);
        let Object::Polyline(ref p) = ch.polyline else {
            panic!()
        };
        assert_eq!(p.points.len(), 8);
        assert!(p.bulges.is_empty());
        // First straight segment runs (3, 0) -> (37, 0).
        close(p.points[0].x, 3.0);
        close(p.points[0].y, 0.0);
        close(p.points[1].x, 37.0);
        close(p.points[1].y, 0.0);
        // Bevel runs (37, 0) -> (40, 3).
        close(p.points[2].x, 40.0);
        close(p.points[2].y, 3.0);
    }
}
