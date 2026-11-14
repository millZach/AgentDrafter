//! The curves an object's geometry is made of, exactly: straight segments,
//! arcs of circles, arcs of ellipses and B-splines, with the points along
//! them, their lengths, the areas they enclose and their extents.
//!
//! Each curve runs along a parameter `t` over its [`Curve::domain`]: 0 to 1
//! for a segment, an arc and an ellipse, and the knots' span for a spline.
//! Lengths and areas that have no closed form, an ellipse's and a
//! spline's, are integrated to within a ten-billionth.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

/// A point or a vector in space: `[x, y, z]`.
pub(crate) type P3 = [f64; 3];

/// One curve of an object's geometry.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Curve {
    /// A straight segment from `a` to `b`.
    Segment { a: P3, b: P3 },
    /// An arc of the circle about `center`, in the plane parallel to XY at
    /// its `z`, from `start` radians round by `sweep` radians, positive
    /// anticlockwise; a whole circle sweeps a whole turn.
    Arc {
        center: P3,
        radius: f64,
        start: f64,
        sweep: f64,
    },
    /// An arc of the ellipse about `center` whose point at parameter `s` is
    /// `center + cos s · major + sin s · minor`, from `start` round by
    /// `sweep`, positive, in the plane parallel to XY.
    Ellipse {
        center: P3,
        major: [f64; 2],
        minor: [f64; 2],
        start: f64,
        sweep: f64,
    },
    /// A B-spline in space.
    Spline(BSpline),
}

/// A B-spline: its degree, control points and knots, and its derivative's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BSpline {
    degree: usize,
    control: Vec<P3>,
    knots: Vec<f64>,
    /// The derivative's control points, of degree `degree - 1` on the knots
    /// without their first and last.
    derivative: Vec<P3>,
}

impl BSpline {
    pub(crate) fn new(degree: usize, control: Vec<P3>, knots: Vec<f64>) -> BSpline {
        let derivative = (0..control.len().saturating_sub(1))
            .map(|i| {
                let span = knots[i + degree + 1] - knots[i + 1];
                let f = if span == 0.0 {
                    0.0
                } else {
                    degree as f64 / span
                };
                sub(control[i + 1], control[i]).map(|d| d * f)
            })
            .collect();
        BSpline {
            degree,
            control,
            knots,
            derivative,
        }
    }

    fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.control.len()])
    }

    /// The knots inside the domain where the curve may bend sharply, with
    /// the domain's ends: the pieces between them are smooth.
    fn breaks(&self) -> Vec<f64> {
        let (a, b) = self.domain();
        let mut breaks = vec![a];
        for &k in &self.knots[self.degree + 1..self.control.len()] {
            if k > *breaks.last().unwrap() && k < b {
                breaks.push(k);
            }
        }
        breaks.push(b);
        breaks
    }

    fn point(&self, u: f64) -> P3 {
        de_boor(self.degree, &self.control, &self.knots, u)
    }

    fn tangent(&self, u: f64) -> P3 {
        if self.degree == 0 || self.derivative.is_empty() {
            return [0.0; 3];
        }
        let knots = &self.knots[1..self.knots.len() - 1];
        de_boor(self.degree - 1, &self.derivative, knots, u)
    }

    /// The spline placed by `place`, which moves, scales and turns: a
    /// B-spline is placed by placing its control points.
    fn placed(&self, place: &dyn Fn(P3) -> P3) -> BSpline {
        BSpline::new(
            self.degree,
            self.control.iter().map(|&p| place(p)).collect(),
            self.knots.clone(),
        )
    }
}

/// The point at `u` of the B-spline of `degree` on `control` and `knots`
/// (de Boor's algorithm).
fn de_boor(degree: usize, control: &[P3], knots: &[f64], u: f64) -> P3 {
    let p = degree;
    let last = control.len() - 1;
    let span = (p..=last)
        .rev()
        .find(|&i| knots[i] <= u && (u < knots[i + 1] || i == last))
        .unwrap_or(p);
    let mut d: Vec<P3> = (0..=p).map(|j| control[span - p + j]).collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = span - p + j;
            let denominator = knots[i + p + 1 - r] - knots[i];
            let alpha = if denominator == 0.0 {
                0.0
            } else {
                (u - knots[i]) / denominator
            };
            d[j] = [0, 1, 2].map(|k| (1.0 - alpha) * d[j - 1][k] + alpha * d[j][k]);
        }
    }
    d[p]
}

pub(crate) fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn scaled(a: P3, f: f64) -> P3 {
    a.map(|v| v * f)
}

pub(crate) fn length_of(a: P3) -> f64 {
    a[0].hypot(a[1]).hypot(a[2])
}

/// The distance between two points seen from above.
pub(crate) fn plan_distance(a: P3, b: P3) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The angle `a` lies at along an arc starting at `start` and sweeping
/// `sweep`, as a share of the sweep: in `[0, 1]` when it is on the arc.
fn share_of_sweep(angle: f64, start: f64, sweep: f64) -> f64 {
    if sweep >= 0.0 {
        (angle - start).rem_euclid(TAU) / sweep
    } else {
        (start - angle).rem_euclid(TAU) / -sweep
    }
}

impl Curve {
    /// The parameters the curve runs over.
    pub(crate) fn domain(&self) -> (f64, f64) {
        match self {
            Curve::Spline(s) => s.domain(),
            _ => (0.0, 1.0),
        }
    }

    /// The parameters between which the curve is smooth, its ends among
    /// them.
    pub(crate) fn breaks(&self) -> Vec<f64> {
        match self {
            Curve::Spline(s) => s.breaks(),
            _ => vec![0.0, 1.0],
        }
    }

    pub(crate) fn point(&self, t: f64) -> P3 {
        match self {
            Curve::Segment { a, b } => add(*a, scaled(sub(*b, *a), t)),
            Curve::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                let (sin, cos) = (start + sweep * t).sin_cos();
                [
                    center[0] + radius * cos,
                    center[1] + radius * sin,
                    center[2],
                ]
            }
            Curve::Ellipse {
                center,
                major,
                minor,
                start,
                sweep,
            } => {
                let (sin, cos) = (start + sweep * t).sin_cos();
                [
                    center[0] + cos * major[0] + sin * minor[0],
                    center[1] + cos * major[1] + sin * minor[1],
                    center[2],
                ]
            }
            Curve::Spline(s) => s.point(t),
        }
    }

    /// The curve's derivative at `t`, along its parameter.
    pub(crate) fn tangent(&self, t: f64) -> P3 {
        match self {
            Curve::Segment { a, b } => sub(*b, *a),
            Curve::Arc {
                radius,
                start,
                sweep,
                ..
            } => {
                let (sin, cos) = (start + sweep * t).sin_cos();
                [-radius * sin * sweep, radius * cos * sweep, 0.0]
            }
            Curve::Ellipse {
                major,
                minor,
                start,
                sweep,
                ..
            } => {
                let (sin, cos) = (start + sweep * t).sin_cos();
                [
                    (-sin * major[0] + cos * minor[0]) * sweep,
                    (-sin * major[1] + cos * minor[1]) * sweep,
                    0.0,
                ]
            }
            Curve::Spline(s) => s.tangent(t),
        }
    }

    pub(crate) fn start(&self) -> P3 {
        self.point(self.domain().0)
    }

    pub(crate) fn end(&self) -> P3 {
        self.point(self.domain().1)
    }

    /// Whether the curve is straight.
    pub(crate) fn is_straight(&self) -> bool {
        matches!(self, Curve::Segment { .. })
    }

    /// The curve placed by `place`, which moves, scales evenly and turns
    /// about the Z axis by `turn` radians, enlarging by `scale`.
    pub(crate) fn placed(&self, place: &dyn Fn(P3) -> P3, scale: f64, turn: f64) -> Curve {
        let (sin, cos) = turn.sin_cos();
        let vector = |v: [f64; 2]| {
            [
                (v[0] * cos - v[1] * sin) * scale,
                (v[0] * sin + v[1] * cos) * scale,
            ]
        };
        match self {
            Curve::Segment { a, b } => Curve::Segment {
                a: place(*a),
                b: place(*b),
            },
            Curve::Arc {
                center,
                radius,
                start,
                sweep,
            } => Curve::Arc {
                center: place(*center),
                radius: radius * scale,
                start: start + turn,
                sweep: *sweep,
            },
            Curve::Ellipse {
                center,
                major,
                minor,
                start,
                sweep,
            } => Curve::Ellipse {
                center: place(*center),
                major: vector(*major),
                minor: vector(*minor),
                start: *start,
                sweep: *sweep,
            },
            Curve::Spline(s) => Curve::Spline(s.placed(place)),
        }
    }

    /// The curve's length, in space.
    pub(crate) fn length(&self) -> f64 {
        let (a, b) = self.domain();
        self.length_between(a, b)
    }

    /// The length along the curve from `a` to `b`, `a` before `b`.
    pub(crate) fn length_between(&self, a: f64, b: f64) -> f64 {
        match self {
            Curve::Segment { .. } | Curve::Arc { .. } => length_of(self.tangent(0.0)) * (b - a),
            _ => {
                let speed = |t: f64| length_of(self.tangent(t));
                pieces(&self.breaks(), a, b)
                    .map(|(a, b)| integrate(&speed, a, b))
                    .sum()
            }
        }
    }

    /// The parameter halfway along the curve's length.
    pub(crate) fn middle(&self) -> f64 {
        let (a, b) = self.domain();
        if matches!(self, Curve::Segment { .. } | Curve::Arc { .. }) {
            return (a + b) / 2.0;
        }
        let half = self.length() / 2.0;
        let (mut low, mut high) = (a, b);
        for _ in 0..200 {
            let mid = (low + high) / 2.0;
            if mid <= low || mid >= high {
                break;
            }
            if self.length_between(a, mid) < half {
                low = mid;
            } else {
                high = mid;
            }
        }
        (low + high) / 2.0
    }

    /// Half the integral of `x dy - y dx` along the curve, seen from above:
    /// summed round a closed loop, the area it encloses, positive when it
    /// runs anticlockwise.
    pub(crate) fn green(&self) -> f64 {
        match self {
            Curve::Segment { a, b } => 0.5 * (a[0] * b[1] - b[0] * a[1]),
            Curve::Arc {
                center,
                radius,
                start,
                sweep,
            } => {
                let (t0, t1) = (*start, start + sweep);
                0.5 * (radius * radius * sweep + radius * center[0] * (t1.sin() - t0.sin())
                    - radius * center[1] * (t1.cos() - t0.cos()))
            }
            _ => {
                let (a, b) = self.domain();
                let f = |t: f64| {
                    let p = self.point(t);
                    let d = self.tangent(t);
                    0.5 * (p[0] * d[1] - p[1] * d[0])
                };
                pieces(&self.breaks(), a, b)
                    .map(|(a, b)| integrate(&f, a, b))
                    .sum()
            }
        }
    }

    /// The parameters within the domain where the curve's coordinate `axis`
    /// is at its least or greatest, its ends among them.
    fn turning_points(&self, axis: usize) -> Vec<f64> {
        let (a, b) = self.domain();
        let mut at = vec![a, b];
        match self {
            Curve::Segment { .. } => {}
            Curve::Arc { start, sweep, .. } if axis < 2 => {
                let first = if axis == 0 { 0.0 } else { FRAC_PI_2 };
                for angle in [first, first + PI] {
                    let s = share_of_sweep(angle, *start, *sweep);
                    if s <= 1.0 {
                        at.push(s);
                    }
                }
            }
            Curve::Ellipse {
                major,
                minor,
                start,
                sweep,
                ..
            } if axis < 2 => {
                // x' = -sin s · major[axis] + cos s · minor[axis] = 0.
                let s0 = minor[axis].atan2(major[axis]);
                for s in [s0, s0 + PI] {
                    let share = (s - start).rem_euclid(TAU) / sweep;
                    if share <= 1.0 {
                        at.push(share);
                    }
                }
            }
            Curve::Spline(_) => {
                let f = |t: f64| self.tangent(t)[axis];
                at.extend(roots(&f, &self.breaks(), 64));
            }
            _ => {}
        }
        at
    }

    /// The box round the curve: its least and greatest corner.
    pub(crate) fn extents(&self) -> (P3, P3) {
        self.extents_from(self.start(), self.end())
    }

    /// The box round the curve, given its ends exactly, such as the points
    /// of the polyline a segment runs between: only where it turns between
    /// them is worked out.
    pub(crate) fn extents_from(&self, start: P3, end: P3) -> (P3, P3) {
        let (a, b) = self.domain();
        let ends = (b - a) * 1e-12;
        let mut min = [0, 1, 2].map(|i| start[i].min(end[i]));
        let mut max = [0, 1, 2].map(|i| start[i].max(end[i]));
        for axis in 0..3 {
            for t in self.turning_points(axis) {
                if t <= a + ends || t >= b - ends {
                    continue;
                }
                let v = self.point(t)[axis];
                min[axis] = min[axis].min(v);
                max[axis] = max[axis].max(v);
            }
        }
        (min, max)
    }

    /// Whether the parameter `t` lies on the curve, within a little.
    pub(crate) fn holds(&self, t: f64) -> bool {
        let (a, b) = self.domain();
        let slack = (b - a) * 1e-9;
        t >= a - slack && t <= b + slack
    }

    /// The parameter of the point `p` seen from above, for a point known to
    /// lie on the line or the circle the curve runs along; none for an
    /// ellipse or a spline.
    fn parameter_of(&self, p: P3) -> Option<f64> {
        match self {
            Curve::Segment { a, b } => {
                let d = sub(*b, *a);
                let l2 = d[0] * d[0] + d[1] * d[1];
                (l2 > 0.0).then(|| ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2)
            }
            Curve::Arc {
                center,
                start,
                sweep,
                ..
            } => {
                let angle = (p[1] - center[1]).atan2(p[0] - center[0]);
                let s = share_of_sweep(angle, *start, *sweep);
                // A point a hair before the start is at the start.
                let turn = TAU / sweep.abs();
                Some(if s > 1.0 && turn - s < 1e-9 * turn {
                    s - turn
                } else {
                    s
                })
            }
            _ => None,
        }
    }
}

/// The pieces of `[a, b]` between `breaks`.
fn pieces(breaks: &[f64], a: f64, b: f64) -> impl Iterator<Item = (f64, f64)> + '_ {
    breaks.windows(2).filter_map(move |w| {
        let (lo, hi) = (w[0].max(a), w[1].min(b));
        (hi > lo).then_some((lo, hi))
    })
}

/// The abscissae and weights of 10-point Gauss-Legendre quadrature on
/// `[-1, 1]`.
const GAUSS: [(f64, f64); 5] = [
    (0.148_874_338_981_631_2, 0.295_524_224_714_752_9),
    (0.433_395_394_129_247_2, 0.269_266_719_309_996_4),
    (0.679_409_568_299_024_4, 0.219_086_362_515_982_04),
    (0.865_063_366_688_984_5, 0.149_451_349_150_580_6),
    (0.973_906_528_517_171_7, 0.066_671_344_308_688_14),
];

fn gauss(f: &dyn Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    let (m, h) = ((a + b) / 2.0, (b - a) / 2.0);
    GAUSS
        .iter()
        .map(|&(x, w)| w * (f(m - h * x) + f(m + h * x)))
        .sum::<f64>()
        * h
}

/// The integral of `f` from `a` to `b`, halving each piece until two
/// halves agree with the whole to within a ten-billionth.
pub(crate) fn integrate(f: &dyn Fn(f64) -> f64, a: f64, b: f64) -> f64 {
    fn adapt(f: &dyn Fn(f64) -> f64, a: f64, b: f64, whole: f64, depth: u32) -> f64 {
        let m = (a + b) / 2.0;
        let (left, right) = (gauss(f, a, m), gauss(f, m, b));
        let halves = left + right;
        if depth >= 24 || (halves - whole).abs() <= 1e-13 * halves.abs().max(1e-300) {
            halves
        } else {
            adapt(f, a, m, left, depth + 1) + adapt(f, m, b, right, depth + 1)
        }
    }
    adapt(f, a, b, gauss(f, a, b), 0)
}

/// The parameters within `breaks` where `f` crosses 0, each piece between
/// breaks looked at in `steps` steps and each crossing then halved down to
/// the last digit.
pub(crate) fn roots(f: &dyn Fn(f64) -> f64, breaks: &[f64], steps: usize) -> Vec<f64> {
    let mut found: Vec<f64> = Vec::new();
    for w in breaks.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mut t0 = a;
        let mut f0 = f(a);
        for i in 1..=steps {
            let t1 = a + (b - a) * i as f64 / steps as f64;
            let f1 = f(t1);
            if f0 == 0.0 {
                found.push(t0);
            } else if f0.signum() != f1.signum() && f1 != 0.0 {
                found.push(bisect(f, t0, t1, f0));
            }
            t0 = t1;
            f0 = f1;
        }
        if f0 == 0.0 {
            found.push(t0);
        }
    }
    found.dedup_by(|a, b| (*a - *b).abs() <= 1e-12 * (1.0 + b.abs()));
    found
}

/// The root of `f` between `a` and `b`, where it changes sign, `fa` being
/// `f(a)`.
fn bisect(f: &dyn Fn(f64) -> f64, mut a: f64, mut b: f64, mut fa: f64) -> f64 {
    for _ in 0..200 {
        let m = (a + b) / 2.0;
        if m <= a || m >= b {
            break;
        }
        let fm = f(m);
        if fm == 0.0 {
            return m;
        }
        if fm.signum() == fa.signum() {
            a = m;
            fa = fm;
        } else {
            b = m;
        }
    }
    (a + b) / 2.0
}

/// A point where two curves cross, seen from above: the parameter along
/// each.
pub(crate) type Crossing = (f64, f64);

/// Where curves `c` and `d` cross seen from above, in the order they lie
/// along `c`. Lines and arcs are crossed exactly; anything with an ellipse
/// or a spline is found from fine chords and then refined.
pub(crate) fn crossings(c: &Curve, d: &Curve) -> Vec<Crossing> {
    let mut found = match (c, d) {
        (Curve::Segment { .. } | Curve::Arc { .. }, Curve::Segment { .. } | Curve::Arc { .. }) => {
            exact_crossings(c, d)
        }
        _ => numeric_crossings(c, d),
    };
    found.retain(|&(t, u)| c.holds(t) && d.holds(u));
    let (c0, c1) = c.domain();
    let (d0, d1) = d.domain();
    for (t, u) in &mut found {
        *t = t.clamp(c0, c1);
        *u = u.clamp(d0, d1);
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    let size = scale_of(c).max(scale_of(d));
    found.dedup_by(|a, b| plan_distance(c.point(a.0), c.point(b.0)) <= 1e-9 * size);
    found
}

/// How large a curve's coordinates are, for tolerances: at least 1.
pub(crate) fn scale_of(c: &Curve) -> f64 {
    let (min, max) = c.extents();
    min.iter()
        .chain(max.iter())
        .fold(1.0_f64, |m, v| m.max(v.abs()))
}

/// The line or circle a segment or an arc runs along: a point and a
/// direction, or a centre and a radius.
enum Carrier {
    Line { a: [f64; 2], d: [f64; 2] },
    Circle { c: [f64; 2], r: f64 },
}

fn carrier(curve: &Curve) -> Carrier {
    match curve {
        Curve::Segment { a, b } => Carrier::Line {
            a: [a[0], a[1]],
            d: [b[0] - a[0], b[1] - a[1]],
        },
        Curve::Arc { center, radius, .. } => Carrier::Circle {
            c: [center[0], center[1]],
            r: *radius,
        },
        _ => unreachable!("only a segment or an arc has a line or a circle"),
    }
}

/// The points where the lines or circles of two segments or arcs meet.
fn exact_crossings(c: &Curve, d: &Curve) -> Vec<Crossing> {
    let points: Vec<[f64; 2]> = match (carrier(c), carrier(d)) {
        (Carrier::Line { a, d: u }, Carrier::Line { a: b, d: v }) => {
            let cross = u[0] * v[1] - u[1] * v[0];
            if cross == 0.0 {
                Vec::new()
            } else {
                let t = ((b[0] - a[0]) * v[1] - (b[1] - a[1]) * v[0]) / cross;
                vec![[a[0] + u[0] * t, a[1] + u[1] * t]]
            }
        }
        (Carrier::Line { a, d: u }, Carrier::Circle { c, r })
        | (Carrier::Circle { c, r }, Carrier::Line { a, d: u }) => line_circle(a, u, c, r),
        (Carrier::Circle { c: c1, r: r1 }, Carrier::Circle { c: c2, r: r2 }) => {
            circle_circle(c1, r1, c2, r2)
        }
    };
    points
        .into_iter()
        .filter_map(|p| {
            let p = [p[0], p[1], 0.0];
            Some((c.parameter_of(p)?, d.parameter_of(p)?))
        })
        .collect()
}

fn line_circle(a: [f64; 2], u: [f64; 2], c: [f64; 2], r: f64) -> Vec<[f64; 2]> {
    let l2 = u[0] * u[0] + u[1] * u[1];
    if l2 == 0.0 {
        return Vec::new();
    }
    // The foot of the perpendicular from the centre, and half the chord.
    let t = ((c[0] - a[0]) * u[0] + (c[1] - a[1]) * u[1]) / l2;
    let foot = [a[0] + u[0] * t, a[1] + u[1] * t];
    let h2 = r * r - ((foot[0] - c[0]).powi(2) + (foot[1] - c[1]).powi(2));
    let tolerance = 1e-12 * r * r;
    if h2 < -tolerance {
        return Vec::new();
    }
    if h2 <= tolerance {
        return vec![foot];
    }
    let s = (h2 / l2).sqrt();
    vec![
        [foot[0] - u[0] * s, foot[1] - u[1] * s],
        [foot[0] + u[0] * s, foot[1] + u[1] * s],
    ]
}

fn circle_circle(c1: [f64; 2], r1: f64, c2: [f64; 2], r2: f64) -> Vec<[f64; 2]> {
    let d = (c2[0] - c1[0]).hypot(c2[1] - c1[1]);
    if d == 0.0 {
        return Vec::new();
    }
    let a = (d * d + r1 * r1 - r2 * r2) / (2.0 * d);
    let h2 = r1 * r1 - a * a;
    let tolerance = 1e-12 * r1.max(r2).powi(2);
    if h2 < -tolerance {
        return Vec::new();
    }
    let e = [(c2[0] - c1[0]) / d, (c2[1] - c1[1]) / d];
    let m = [c1[0] + e[0] * a, c1[1] + e[1] * a];
    if h2 <= tolerance {
        return vec![m];
    }
    let h = h2.sqrt();
    vec![
        [m[0] - e[1] * h, m[1] + e[0] * h],
        [m[0] + e[1] * h, m[1] - e[0] * h],
    ]
}

/// The curve's parameters at fine steps along it, its breaks among them.
fn samples(c: &Curve) -> Vec<f64> {
    let per_piece = match c {
        Curve::Segment { .. } => 1,
        Curve::Arc { sweep, .. } | Curve::Ellipse { sweep, .. } => {
            ((sweep.abs() / TAU) * 256.0).ceil().max(8.0) as usize
        }
        Curve::Spline(_) => 32,
    };
    let breaks = c.breaks();
    let mut at = vec![breaks[0]];
    for w in breaks.windows(2) {
        for i in 1..=per_piece {
            at.push(w[0] + (w[1] - w[0]) * i as f64 / per_piece as f64);
        }
    }
    at
}

/// Where two curves cross, found where their fine chords cross or nearly
/// do, then refined by Newton's method on both parameters.
fn numeric_crossings(c: &Curve, d: &Curve) -> Vec<Crossing> {
    let (ts, us) = (samples(c), samples(d));
    let (pc, pd): (Vec<P3>, Vec<P3>) = (
        ts.iter().map(|&t| c.point(t)).collect(),
        us.iter().map(|&u| d.point(u)).collect(),
    );
    let size = scale_of(c).max(scale_of(d));
    let mut found = Vec::new();
    for i in 0..ts.len() - 1 {
        let (a0, a1) = (pc[i], pc[i + 1]);
        for j in 0..us.len() - 1 {
            let (b0, b1) = (pd[j], pd[j + 1]);
            // Boxes that miss by more than a little cannot cross.
            let reach = 0.05 * (plan_distance(a0, a1) + plan_distance(b0, b1));
            if a0[0].min(a1[0]) > b0[0].max(b1[0]) + reach
                || b0[0].min(b1[0]) > a0[0].max(a1[0]) + reach
                || a0[1].min(a1[1]) > b0[1].max(b1[1]) + reach
                || b0[1].min(b1[1]) > a0[1].max(a1[1]) + reach
            {
                continue;
            }
            let (s, r) = chord_crossing(a0, a1, b0, b1);
            let t = ts[i] + (ts[i + 1] - ts[i]) * s.clamp(0.0, 1.0);
            let u = us[j] + (us[j + 1] - us[j]) * r.clamp(0.0, 1.0);
            if let Some((t, u)) = refine(c, d, t, u) {
                if plan_distance(c.point(t), d.point(u)) <= 1e-9 * size {
                    found.push((t, u));
                }
            }
        }
    }
    found
}

/// Where two chords' lines cross, as a share along each; the middle of each
/// when they are parallel.
fn chord_crossing(a0: P3, a1: P3, b0: P3, b1: P3) -> (f64, f64) {
    let u = [a1[0] - a0[0], a1[1] - a0[1]];
    let v = [b1[0] - b0[0], b1[1] - b0[1]];
    let cross = u[0] * v[1] - u[1] * v[0];
    if cross == 0.0 {
        return (0.5, 0.5);
    }
    let w = [b0[0] - a0[0], b0[1] - a0[1]];
    (
        (w[0] * v[1] - w[1] * v[0]) / cross,
        (w[0] * u[1] - w[1] * u[0]) / cross,
    )
}

/// Newton's method for `c(t) = d(u)` seen from above, from a guess, kept to
/// a little beyond each curve's ends; none when it does not settle.
fn refine(c: &Curve, d: &Curve, mut t: f64, mut u: f64) -> Option<(f64, f64)> {
    let (c0, c1) = c.domain();
    let (d0, d1) = d.domain();
    let (cs, ds) = ((c1 - c0) * 0.01, (d1 - d0) * 0.01);
    for _ in 0..64 {
        let (p, q) = (c.point(t), d.point(u));
        let f = [p[0] - q[0], p[1] - q[1]];
        let (dc, dd) = (c.tangent(t), d.tangent(u));
        // [dc, -dd] [Δt, Δu] = -f
        let det = dc[0] * -dd[1] - (-dd[0]) * dc[1];
        if det == 0.0 {
            return None;
        }
        let dt = (-f[0] * -dd[1] - (-dd[0]) * -f[1]) / det;
        let du = (dc[0] * -f[1] - (-f[0]) * dc[1]) / det;
        t = (t + dt).clamp(c0 - cs, c1 + cs);
        u = (u + du).clamp(d0 - ds, d1 + ds);
        if dt.abs() <= 1e-15 * (1.0 + t.abs()) && du.abs() <= 1e-15 * (1.0 + u.abs()) {
            break;
        }
    }
    Some((t, u))
}

/// The parameters at which the curve meets `wanted`, a condition on the
/// point and the derivative there that crosses 0, such as being
/// perpendicular to the line from a point. A segment or an arc is solved
/// exactly by its caller; this is for ellipses and splines.
pub(crate) fn where_on(c: &Curve, wanted: &dyn Fn(P3, P3) -> f64) -> Vec<f64> {
    let f = |t: f64| wanted(c.point(t), c.tangent(t));
    let steps = match c {
        Curve::Spline(_) => 64,
        _ => 512,
    };
    let mut found = roots(&f, &c.breaks(), steps);
    // A whole ellipse's start and end are one point.
    if let Curve::Ellipse { sweep, .. } = c {
        if (*sweep - TAU).abs() < 1e-12 && found.len() > 1 {
            let (a, b) = c.domain();
            if found.first() == Some(&a) && found.last() == Some(&b) {
                found.pop();
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() <= 1e-9 * (1.0 + b.abs()), "{a} is not {b}");
    }

    #[test]
    fn an_arc_s_length_area_and_extents_are_exact() {
        let quarter = Curve::Arc {
            center: [0.0, 0.0, 0.0],
            radius: 2.0,
            start: 0.0,
            sweep: FRAC_PI_2,
        };
        close(quarter.length(), PI);
        // The quarter and the two radii enclose a quarter of the disc.
        let radii = quarter.green()
            + Curve::Segment {
                a: [0.0, 2.0, 0.0],
                b: [0.0, 0.0, 0.0],
            }
            .green()
            + Curve::Segment {
                a: [0.0, 0.0, 0.0],
                b: [2.0, 0.0, 0.0],
            }
            .green();
        close(radii, PI);
        let (min, max) = quarter.extents();
        close(min[0], 0.0);
        close(min[1], 0.0);
        close(max[0], 2.0);
        close(max[1], 2.0);
    }

    #[test]
    fn an_ellipse_s_length_is_its_perimeter() {
        let e = Curve::Ellipse {
            center: [0.0; 3],
            major: [2.0, 0.0],
            minor: [0.0, 1.0],
            start: 0.0,
            sweep: TAU,
        };
        // Ramanujan's second approximation is good to far better than this.
        let (a, b) = (2.0_f64, 1.0_f64);
        let h = ((a - b) / (a + b)).powi(2);
        let perimeter = PI * (a + b) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
        assert!((e.length() - perimeter).abs() < 1e-6);
        close(e.green(), PI * 2.0);
    }

    #[test]
    fn a_line_crosses_a_circle_twice_and_a_spline_where_it_runs() {
        let line = Curve::Segment {
            a: [-5.0, 0.0, 0.0],
            b: [5.0, 0.0, 0.0],
        };
        let circle = Curve::Arc {
            center: [0.0; 3],
            radius: 3.0,
            start: 0.0,
            sweep: TAU,
        };
        let found = crossings(&line, &circle);
        assert_eq!(found.len(), 2);
        close(line.point(found[0].0)[0], -3.0);
        close(line.point(found[1].0)[0], 3.0);
        let spline = Curve::Spline(BSpline::new(
            2,
            vec![[0.0, -2.0, 0.0], [1.0, 4.0, 0.0], [2.0, -2.0, 0.0]],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        ));
        let found = crossings(&line, &spline);
        assert_eq!(found.len(), 2);
        for (t, u) in found {
            close(line.point(t)[1], 0.0);
            close(spline.point(u)[1], 0.0);
        }
    }
}
