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

    /// The curve placed by `place`, which moves, scales evenly by `scale`,
    /// may mirror, when `mirrored`, and turns about the Z axis: `vector`
    /// transforms a 2D direction directly without translating through the
    /// origin, and `angle` gives the direction, in radians, a direction of the
    /// curve's own takes. A mirror reverses the way an arc runs.
    pub(crate) fn placed(
        &self,
        place: &dyn Fn(P3) -> P3,
        vector: &dyn Fn([f64; 2]) -> [f64; 2],
        scale: f64,
        angle: &dyn Fn(f64) -> f64,
        mirrored: bool,
    ) -> Curve {
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
                start: angle(*start),
                sweep: if mirrored { -*sweep } else { *sweep },
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
                let f = |t: f64| {
                    let d = self.tangent(t);
                    (d[axis], length_of(d))
                };
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
    pub(crate) fn parameter_of(&self, p: P3) -> Option<f64> {
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

/// The parameters within `breaks` where `f` is 0. `f` gives its value and
/// how large its terms are: a value within a billionth of that is 0. Each
/// piece between breaks is looked at in `steps` steps, each halved; where
/// the value changes sign, the crossing is halved down to the last digit,
/// and where its size dips between two neighbours of the same sign, the dip
/// is followed to its bottom, which holds two roots when the sign changes
/// there and one, where the curve touches, when the value is 0 there. So
/// roots closer together than a step are not lost.
pub(crate) fn roots(f: &dyn Fn(f64) -> (f64, f64), breaks: &[f64], steps: usize) -> Vec<f64> {
    let value = |t: f64| f(t).0;
    let mut found: Vec<f64> = Vec::new();
    for w in breaks.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = 2 * steps;
        let ts: Vec<f64> = (0..=n).map(|i| a + (b - a) * i as f64 / n as f64).collect();
        let fs: Vec<f64> = ts.iter().map(|&t| value(t)).collect();
        for i in 0..=n {
            if fs[i] == 0.0 {
                found.push(ts[i]);
            }
            if i < n && fs[i] != 0.0 && fs[i + 1] != 0.0 && fs[i].signum() != fs[i + 1].signum() {
                found.push(bisect(&value, ts[i], ts[i + 1], fs[i]));
            }
            // A dip of the value's size that keeps its sign between its
            // neighbours, or one that reaches 0 at a sample.
            if i == 0 || i == n {
                continue;
            }
            let (l, m, r) = (fs[i - 1], fs[i], fs[i + 1]);
            let same = l != 0.0
                && m != 0.0
                && r != 0.0
                && l.signum() == m.signum()
                && m.signum() == r.signum();
            if !same || m.abs() > l.abs() || m.abs() > r.abs() {
                continue;
            }
            let bottom = least(&|t| value(t).abs(), ts[i - 1], ts[i + 1]);
            let (v, size) = f(bottom);
            if v != 0.0 && v.signum() != m.signum() {
                found.push(bisect(&value, ts[i - 1], bottom, l));
                found.push(bisect(&value, bottom, ts[i + 1], v));
            } else if v.abs() <= 1e-9 * size {
                found.push(bottom);
            }
        }
    }
    found.sort_by(f64::total_cmp);
    let span = breaks.last().unwrap_or(&1.0) - breaks.first().unwrap_or(&0.0);
    found.dedup_by(|a, b| (*a - *b).abs() <= 1e-9 * span.abs().max(f64::MIN_POSITIVE));
    found
}

/// Where `g` is least between `a` and `b`, for a `g` with one dip there
/// (golden-section search).
fn least(g: &dyn Fn(f64) -> f64, mut a: f64, mut b: f64) -> f64 {
    let ratio = (5f64.sqrt() - 1.0) / 2.0;
    let mut c = b - ratio * (b - a);
    let mut d = a + ratio * (b - a);
    let (mut gc, mut gd) = (g(c), g(d));
    for _ in 0..200 {
        if (b - a).abs() <= 1e-15 * (1.0 + a.abs()) {
            break;
        }
        if gc < gd {
            b = d;
            d = c;
            gd = gc;
            c = b - ratio * (b - a);
            gc = g(c);
        } else {
            a = c;
            c = d;
            gc = gd;
            d = a + ratio * (b - a);
            gd = g(d);
        }
    }
    (a + b) / 2.0
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

/// Where curves `c` and `d` cross or touch seen from above, in the order
/// they lie along `c`.
///
/// Where one of the two is a line, a circle or an arc, or an ellipse, it is
/// taken as a signed function of a point that is 0 on it, its [`Shape`],
/// and that function is followed along the other: see [`meetings`]. A
/// crossing is a change of its sign and a touch a dip of its size to 0 that
/// does not change it, so each point is found once, however close another
/// is, and no point is found that is not on both. The shape is the simpler
/// of the two, a line before a circle or an arc, before an ellipse. Two
/// splines have no such function, and meet as [`splines_meet`] finds.
pub(crate) fn crossings(c: &Curve, d: &Curve) -> Vec<Crossing> {
    let size = scale_of(c).max(scale_of(d));
    let swap = |found: Vec<Crossing>| found.into_iter().map(|(u, t)| (t, u)).collect();
    let mut found: Vec<Crossing> = if end_on(c).is_some() {
        seen_end_on(c, d, size)
    } else if end_on(d).is_some() {
        swap(seen_end_on(d, c, size))
    } else if matches!((c, d), (Curve::Spline(_), Curve::Spline(_))) {
        splines_meet(c, d, size)
    } else if rank(c) < rank(d) {
        swap(meet(d, c, size))
    } else {
        meet(c, d, size)
    };
    found.retain(|&(t, u)| c.holds(t) && d.holds(u));
    let (c0, c1) = c.domain();
    let (d0, d1) = d.domain();
    for (t, u) in &mut found {
        *t = t.clamp(c0, c1);
        *u = u.clamp(d0, d1);
    }
    // One curve can pass one point of the other twice, at two heights: each
    // parameter pair is a point of its own.
    found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    found.dedup();
    found
}

/// How a curve ranks as a shape: the simpler, the earlier.
fn rank(c: &Curve) -> u8 {
    match c {
        Curve::Segment { .. } => 0,
        Curve::Arc { .. } => 1,
        Curve::Ellipse { .. } => 2,
        Curve::Spline(_) => 3,
    }
}

/// How large a curve's coordinates are, for tolerances: at least 1.
pub(crate) fn scale_of(c: &Curve) -> f64 {
    let (min, max) = c.extents();
    min.iter()
        .chain(max.iter())
        .fold(1.0_f64, |m, v| m.max(v.abs()))
}

/// A curve as a signed function of a point seen from above, 0 on the line,
/// circle or ellipse it runs along.
enum Shape {
    /// The signed distance from the line through `a` square to `n`, a unit
    /// vector.
    Line { a: [f64; 2], n: [f64; 2] },
    /// `|p − c|² − r²`.
    Circle { c: [f64; 2], r: f64 },
    /// `α² + β² − 1`, where `α` and `β` are the point's coordinates along
    /// the ellipse's axes, each as a share of its axis: its unit-circle
    /// form. `m` and `k` are the axes over their lengths squared.
    Ellipse {
        c: [f64; 2],
        m: [f64; 2],
        k: [f64; 2],
        start: f64,
        sweep: f64,
    },
}

impl Shape {
    /// The shape of `d`; none for a spline, or for a segment seen end on.
    fn of(d: &Curve) -> Option<Shape> {
        Some(match d {
            Curve::Segment { a, b } => {
                let v = [b[0] - a[0], b[1] - a[1]];
                let l = v[0].hypot(v[1]);
                if l == 0.0 {
                    return None;
                }
                Shape::Line {
                    a: [a[0], a[1]],
                    n: [-v[1] / l, v[0] / l],
                }
            }
            Curve::Arc { center, radius, .. } => Shape::Circle {
                c: [center[0], center[1]],
                r: *radius,
            },
            Curve::Ellipse {
                center,
                major,
                minor,
                start,
                sweep,
            } => {
                let over = |v: &[f64; 2]| {
                    let l2 = v[0] * v[0] + v[1] * v[1];
                    [v[0] / l2, v[1] / l2]
                };
                Shape::Ellipse {
                    c: [center[0], center[1]],
                    m: over(major),
                    k: over(minor),
                    start: *start,
                    sweep: *sweep,
                }
            }
            Curve::Spline(_) => return None,
        })
    }

    /// The function's value at `p`, its gradient there, and how far `p`
    /// is from the curve.
    fn at(&self, p: P3) -> (f64, [f64; 2], f64) {
        match self {
            Shape::Line { a, n } => {
                let v = (p[0] - a[0]) * n[0] + (p[1] - a[1]) * n[1];
                (v, *n, v.abs())
            }
            Shape::Circle { c, r } => {
                let w = [p[0] - c[0], p[1] - c[1]];
                (
                    w[0] * w[0] + w[1] * w[1] - r * r,
                    [2.0 * w[0], 2.0 * w[1]],
                    (w[0].hypot(w[1]) - r).abs(),
                )
            }
            Shape::Ellipse { c, m, k, .. } => {
                let w = [p[0] - c[0], p[1] - c[1]];
                let (alpha, beta) = (w[0] * m[0] + w[1] * m[1], w[0] * k[0] + w[1] * k[1]);
                let v = alpha * alpha + beta * beta - 1.0;
                let g = [
                    2.0 * (alpha * m[0] + beta * k[0]),
                    2.0 * (alpha * m[1] + beta * k[1]),
                ];
                let slope = g[0].hypot(g[1]);
                let far = if slope > 0.0 {
                    v.abs() / slope
                } else {
                    f64::INFINITY
                };
                (v, g, far)
            }
        }
    }

    /// The parameter along `d`, the curve this is the shape of, of `p`, a
    /// point on it.
    fn parameter(&self, d: &Curve, p: P3) -> Option<f64> {
        match self {
            Shape::Line { .. } | Shape::Circle { .. } => d.parameter_of(p),
            Shape::Ellipse {
                c,
                m,
                k,
                start,
                sweep,
            } => {
                let w = [p[0] - c[0], p[1] - c[1]];
                let (alpha, beta) = (w[0] * m[0] + w[1] * m[1], w[0] * k[0] + w[1] * k[1]);
                let share = (beta.atan2(alpha) - start).rem_euclid(TAU) / sweep;
                // A point a hair before the start is at the start.
                let turn = TAU / sweep;
                Some(if share > 1.0 && turn - share < 1e-9 * turn {
                    share - turn
                } else {
                    share
                })
            }
        }
    }
}

/// Whether a curve closes on itself: a whole circle or ellipse.
fn closed(c: &Curve) -> bool {
    match c {
        Curve::Arc { sweep, .. } | Curve::Ellipse { sweep, .. } => sweep.abs() >= TAU - 1e-12,
        _ => false,
    }
}

/// Where `c` meets `d`, seen from above: the parameter along each.
fn meet(c: &Curve, d: &Curve, size: f64) -> Vec<Crossing> {
    let Some(shape) = Shape::of(d) else {
        return Vec::new();
    };
    meetings(c, &shape, size)
        .into_iter()
        .filter_map(|t| Some((t, shape.parameter(d, c.point(t))?)))
        .collect()
}

/// Where two splines `c` and `d` cross or touch seen from above: the
/// parameter along each.
///
/// A spline has no signed function to follow along the other, since its
/// nearest foot to a point jumps. So both are cut into [`straight_pieces`],
/// and every place they may meet is a start for [`refined`], which solves
/// `c(t) = d(u)` itself: where `c` crosses one of `d`'s chords within it,
/// found by following the chord's line along `c` with [`meetings`], and
/// where a chord of each comes within the two pieces' bends of the other,
/// from the nearest points of the two, which is where splines that touch
/// are found. A start gives a point only where it converges to one on both
/// curves, within the tolerance and their domains. Two starts give one
/// point only when they converge to the same parameters on both, which is
/// identity, not closeness; so one spline passing a point of the other
/// twice, at two heights, gives two, each then checked in height on its
/// own.
fn splines_meet(c: &Curve, d: &Curve, size: f64) -> Vec<Crossing> {
    let tolerance = 1e-9 * (1.0 + size) + 8.0 * f64::EPSILON * size;
    let (on_c, on_d) = (straight_pieces(c), straight_pieces(d));
    let mut starts: Vec<Crossing> = Vec::new();
    for piece in &on_d {
        let chord = Curve::Segment {
            a: piece.a,
            b: piece.b,
        };
        let Some(line) = Shape::of(&chord) else {
            continue;
        };
        for t in meetings(c, &line, size) {
            let Some(s) = chord.parameter_of(c.point(t)) else {
                continue;
            };
            if (-0.25..=1.25).contains(&s) {
                starts.push((t, piece.at(s)));
            }
        }
    }
    for p in &on_c {
        for q in &on_d {
            let (s, r, apart) = nearest_on_chords(p, q);
            if apart <= 2.0 * (p.bend + q.bend) + tolerance {
                starts.push((p.at(s), q.at(r)));
            }
        }
    }
    let (c0, c1) = c.domain();
    let (d0, d1) = d.domain();
    let (t_slack, u_slack) = ((c1 - c0) * 1e-9, (d1 - d0) * 1e-9);
    let mut found: Vec<Crossing> = Vec::new();
    for start in starts {
        let Some((t, u)) = refined(c, d, start, tolerance) else {
            continue;
        };
        let seen = found
            .iter()
            .any(|&(a, b)| (a - t).abs() <= t_slack && (b - u).abs() <= u_slack);
        if !seen {
            found.push((t, u));
        }
    }
    found
}

/// A nearly straight piece of a curve, from parameter `u0` to `u1`: its
/// ends, and how far the curve bends from the chord between them, seen
/// from above.
struct Piece {
    u0: f64,
    u1: f64,
    a: P3,
    b: P3,
    bend: f64,
}

impl Piece {
    /// The parameter along the curve at the share `s` of the chord.
    fn at(&self, s: f64) -> f64 {
        self.u0 + (self.u1 - self.u0) * s.clamp(0.0, 1.0)
    }
}

/// The curve cut at its breaks into pieces, each halved until it bends
/// from its chord by no more than a thousandth of the chord's length.
fn straight_pieces(c: &Curve) -> Vec<Piece> {
    fn cut(c: &Curve, u0: f64, u1: f64, depth: u32, out: &mut Vec<Piece>) {
        let (a, b) = (c.point(u0), c.point(u1));
        let bend = (1..8)
            .map(|i| off_segment(c.point(u0 + (u1 - u0) * f64::from(i) / 8.0), a, b).0)
            .fold(0.0_f64, f64::max);
        if depth >= 16 || bend <= 1e-3 * plan_distance(a, b) {
            out.push(Piece { u0, u1, a, b, bend });
        } else {
            let m = (u0 + u1) / 2.0;
            cut(c, u0, m, depth + 1, out);
            cut(c, m, u1, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    for w in c.breaks().windows(2) {
        let u = |k: u32| w[0] + (w[1] - w[0]) * f64::from(k) / 4.0;
        for i in 0..4 {
            cut(c, u(i), u(i + 1), 0, &mut out);
        }
    }
    out
}

/// How far `p` is from the segment `a` to `b`, seen from above, and the
/// share of the segment its nearest point there lies at.
fn off_segment(p: P3, a: P3, b: P3) -> (f64, f64) {
    let v = [b[0] - a[0], b[1] - a[1]];
    let l2 = v[0] * v[0] + v[1] * v[1];
    let s = if l2 > 0.0 {
        (((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = [a[0] + v[0] * s, a[1] + v[1] * s, 0.0];
    (plan_distance(p, q), s)
}

/// The nearest points of two pieces' chords seen from above: the share of
/// each chord they lie at, and how far apart they are, 0 where they cross.
fn nearest_on_chords(p: &Piece, q: &Piece) -> (f64, f64, f64) {
    let (v, w) = (sub(p.b, p.a), sub(q.b, q.a));
    let x = sub(q.a, p.a);
    let cross = v[0] * w[1] - v[1] * w[0];
    if cross != 0.0 {
        let s = (x[0] * w[1] - x[1] * w[0]) / cross;
        let r = (x[0] * v[1] - x[1] * v[0]) / cross;
        if (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&r) {
            return (s, r, 0.0);
        }
    }
    let mut best = (0.0, 0.0, f64::INFINITY);
    for (s, end) in [(0.0, p.a), (1.0, p.b)] {
        let (apart, r) = off_segment(end, q.a, q.b);
        if apart < best.2 {
            best = (s, r, apart);
        }
    }
    for (r, end) in [(0.0, q.a), (1.0, q.b)] {
        let (apart, s) = off_segment(end, p.a, p.b);
        if apart < best.2 {
            best = (s, r, apart);
        }
    }
    best
}

/// The sine of the angle between `c` at `t` and `d` at `u`, seen from
/// above.
fn sine_between(c: &Curve, t: f64, d: &Curve, u: f64) -> f64 {
    let (v, w) = (c.tangent(t), d.tangent(u));
    (v[0] * w[1] - v[1] * w[0]) / (v[0].hypot(v[1]) * w[0].hypot(w[1]))
}

/// The parameters where `c(t) = d(u)` seen from above, refined from
/// `start`, if they converge to a point on both curves within `tolerance`
/// and within their domains.
///
/// Newton's method on the two coordinates, with the curves' tangents for
/// its Jacobian, fixes a crossing to the last digit. Where the curves run
/// parallel there, or it finds none, they may touch, where the parameters
/// are fixed only to the square root of the last digit; so the touch is
/// taken where the two run parallel, [`parallel`], which a touch fixes to
/// the last digit, and kept only where the curves are within the tolerance
/// of each other there. Where they run parallel but apart, a crossing
/// found is one of two at a shallow angle, and kept.
fn refined(c: &Curve, d: &Curve, start: Crossing, tolerance: f64) -> Option<Crossing> {
    let on_both = |(t, u): Crossing| {
        c.holds(t) && d.holds(u) && plan_distance(c.point(t), d.point(u)) <= tolerance
    };
    let crossing = newton(c, d, start).filter(|&p| on_both(p));
    if let Some((t, u)) = crossing {
        if sine_between(c, t, d, u).abs() > 1e-6 {
            return crossing;
        }
    }
    parallel(c, d, crossing.unwrap_or(start))
        .filter(|&p| on_both(p))
        .or(crossing)
}

/// Newton's method on `c(t) − d(u) = 0` seen from above, from `start`;
/// none where it runs off; where no step can be taken it stops there.
fn newton(c: &Curve, d: &Curve, start: Crossing) -> Option<Crossing> {
    let (c0, c1) = c.domain();
    let (d0, d1) = d.domain();
    let (mut t, mut u) = start;
    for _ in 0..100 {
        let (p, q) = (c.point(t), d.point(u));
        let (v, w) = (c.tangent(t), d.tangent(u));
        let r = [p[0] - q[0], p[1] - q[1]];
        // v·dt − w·du = −r, by Cramer's rule.
        let det = w[0] * v[1] - v[0] * w[1];
        // Where a tangent is zero, at a cusp, or the curves run parallel,
        // no step can be taken: the point stands as it is, for `refined`
        // to keep only when it lies on both.
        if det == 0.0 || !det.is_finite() {
            break;
        }
        let dt = (r[0] * w[1] - r[1] * w[0]) / det;
        let du = (r[0] * v[1] - r[1] * v[0]) / det;
        t += dt;
        u += du;
        let off = |x: f64, a: f64, b: f64| !x.is_finite() || x < a - (b - a) || x > b + (b - a);
        if off(t, c0, c1) || off(u, d0, d1) {
            return None;
        }
        if dt.abs() <= 1e-15 * (1.0 + t.abs()) && du.abs() <= 1e-15 * (1.0 + u.abs()) {
            break;
        }
    }
    Some((t, u))
}

/// Where `c` runs parallel to `d` at its foot on it, seen from above, near
/// `start`: the parameters of the point on `c` and of its foot on `d`.
fn parallel(c: &Curve, d: &Curve, start: Crossing) -> Option<Crossing> {
    let (c0, c1) = c.domain();
    let (mut t, mut u) = start;
    u = foot_on(d, c.point(t), u);
    let h = (c1 - c0) * 1e-7;
    for _ in 0..100 {
        let g = sine_between(c, t, d, u);
        if g == 0.0 {
            break;
        }
        let ahead = foot_on(d, c.point(t + h), u);
        let slope = (sine_between(c, t + h, d, ahead) - g) / h;
        if slope == 0.0 || !slope.is_finite() {
            return None;
        }
        let step = g / slope;
        t -= step;
        if !t.is_finite() || t < c0 - (c1 - c0) || t > c1 + (c1 - c0) {
            return None;
        }
        u = foot_on(d, c.point(t), u);
        if step.abs() <= 1e-15 * (1.0 + t.abs()) {
            break;
        }
    }
    u.is_finite().then_some((t, u))
}

/// The parameters along `c` where it meets `shape`.
///
/// The shape's function, `f`, is followed along `c`, cut into pieces at
/// `c`'s breaks and wherever `f`'s slope along it changes sign, found from
/// fine steps and halved down to the last digit. Within a piece `f` only
/// rises or only falls, so it is 0 there at most once:
///
/// - A piece whose ends have opposite signs holds one crossing, halved down
///   to the last digit. One piece gives one point, so two crossings
///   however close are two, and a crossing where `c` runs flat is one.
/// - A touch is where `f`'s size dips between two pieces that do not change
///   sign, kept only when the point is on both curves within the
///   tolerance, `1e-9 · (1 + size)` and a few units of rounding. An end of
///   an open curve is a touch so too, as where a line ends on another.
/// - Where the dip does change sign, on both sides, but by no more than
///   rounding can, the two crossings beside it are the one touch it is.
/// - Where `c` lies along the shape within the tolerance all the way, they
///   meet nowhere: collinear lines do not intersect, as AutoCAD's
///   intersection snap finds none between them.
///
/// A whole circle or ellipse is followed from where it is furthest from the
/// shape, so that its start, which is its end, is no point it meets.
fn meetings(c: &Curve, shape: &Shape, size: f64) -> Vec<f64> {
    let tolerance = 1e-9 * (1.0 + size) + 8.0 * f64::EPSILON * size;
    let rounding = 64.0 * f64::EPSILON * size;
    let value = |t: f64| shape.at(c.point(t)).0;
    let distance = |t: f64| shape.at(c.point(t)).2;
    let slope = |t: f64| {
        let (_, g, _) = shape.at(c.point(t));
        let v = c.tangent(t);
        g[0] * v[0] + g[1] * v[1]
    };
    let round = closed(c);
    let breaks = if round {
        let from = (0..64)
            .map(|i| f64::from(i) / 64.0)
            .max_by(|a, b| value(*a).abs().total_cmp(&value(*b).abs()))
            .unwrap_or(0.0);
        vec![from, from + 1.0]
    } else {
        c.breaks()
    };
    let steps = match c {
        Curve::Segment { .. } => 8,
        Curve::Arc { sweep, .. } | Curve::Ellipse { sweep, .. } => {
            ((sweep.abs() / TAU) * 128.0).ceil().max(16.0) as usize
        }
        Curve::Spline(_) => 64,
    };
    // The pieces: the breaks, and where the slope changes sign, marked.
    let mut splits: Vec<(f64, bool)> = Vec::new();
    let mut farthest: f64 = 0.0;
    for w in breaks.windows(2) {
        splits.push((w[0], false));
        let mut last: Option<(f64, f64)> = None;
        for i in 0..=steps {
            let t = w[0] + (w[1] - w[0]) * i as f64 / steps as f64;
            farthest = farthest.max(distance(t));
            let s = slope(t);
            if s == 0.0 {
                continue;
            }
            if let Some((t0, s0)) = last {
                if s0.signum() != s.signum() {
                    splits.push((bisect(&slope, t0, t, s0), true));
                }
            }
            last = Some((t, s));
        }
    }
    splits.push((*breaks.last().unwrap_or(&1.0), false));
    if farthest <= tolerance {
        return Vec::new();
    }
    let ts: Vec<f64> = splits.iter().map(|s| s.0).collect();
    let fs: Vec<f64> = ts.iter().map(|&t| value(t)).collect();
    let n = ts.len();
    let changes =
        |i: usize| fs[i] != 0.0 && fs[i + 1] != 0.0 && fs[i].signum() != fs[i + 1].signum();
    let mut crossing: Vec<bool> = (0..n - 1).map(changes).collect();
    // A zero of the function is a meeting only where the point lies on the
    // other curve.
    let mut found: Vec<f64> = (0..n)
        .filter(|&i| fs[i] == 0.0 && distance(ts[i]) <= tolerance)
        .map(|i| ts[i])
        .collect();
    for k in 1..n - 1 {
        if !splits[k].1 || fs[k] == 0.0 {
            continue;
        }
        let (before, after) = (crossing[k - 1], crossing[k]);
        if before && after && distance(ts[k]) <= rounding {
            crossing[k - 1] = false;
            crossing[k] = false;
            found.push(ts[k]);
        } else if !before
            && !after
            && fs[k].abs() <= fs[k - 1].abs()
            && fs[k].abs() <= fs[k + 1].abs()
            && distance(ts[k]) <= tolerance
        {
            found.push(ts[k]);
        }
    }
    if !round {
        for (k, next, piece) in [(0, 1, 0), (n - 1, n - 2, n - 2)] {
            if fs[k] != 0.0
                && !crossing[piece]
                && fs[k].abs() <= fs[next].abs()
                && distance(ts[k]) <= tolerance
            {
                found.push(ts[k]);
            }
        }
    }
    for i in 0..n - 1 {
        if crossing[i] {
            let t = bisect(&value, ts[i], ts[i + 1], fs[i]);
            if distance(t) <= tolerance {
                found.push(t);
            }
        }
    }
    if round {
        for t in &mut found {
            if *t >= 1.0 {
                *t -= 1.0;
            }
        }
    }
    found.sort_by(f64::total_cmp);
    found.dedup();
    found
}

/// The parameter of the foot on `d`, near `u`, of the perpendicular from
/// `q`, seen from above.
fn foot_on(d: &Curve, q: P3, mut u: f64) -> f64 {
    for _ in 0..64 {
        let (p, v) = (d.point(u), d.tangent(u));
        let speed = v[0] * v[0] + v[1] * v[1];
        if speed == 0.0 {
            break;
        }
        let step = ((p[0] - q[0]) * v[0] + (p[1] - q[1]) * v[1]) / speed;
        u -= step;
        if step.abs() <= 1e-15 * (1.0 + u.abs()) {
            break;
        }
    }
    u
}

/// A straight segment seen end on from above, a line along Z: the point
/// it is seen as.
fn end_on(c: &Curve) -> Option<P3> {
    match c {
        Curve::Segment { a, b } if a[0] == b[0] && a[1] == b[1] => Some(*a),
        _ => None,
    }
}

/// Where `d` passes through the point a segment seen end on, `c`, is seen
/// as from above: the parameter along each, the segment's at the height
/// `d` has there, or its nearer end. Two segments seen end on at one point
/// run along one line, and collinear lines do not intersect, as AutoCAD's
/// intersection snap finds none between them.
fn seen_end_on(c: &Curve, d: &Curve, size: f64) -> Vec<Crossing> {
    let (Curve::Segment { a, b }, Some(q)) = (c, end_on(c)) else {
        return Vec::new();
    };
    let on_d: Vec<f64> = match d {
        _ if end_on(d).is_some() => Vec::new(),
        Curve::Segment { .. } | Curve::Arc { .. } => d.parameter_of(q).into_iter().collect(),
        _ => where_on(d, &|p, v| {
            let w = [p[0] - q[0], p[1] - q[1]];
            (
                w[0] * v[0] + w[1] * v[1],
                w[0].hypot(w[1]) * v[0].hypot(v[1]),
            )
        }),
    };
    on_d.into_iter()
        .filter(|&u| plan_distance(d.point(u), q) <= 1e-9 * size)
        .map(|u| {
            let z = d.point(u)[2];
            let t = if b[2] == a[2] {
                0.0
            } else {
                ((z - a[2]) / (b[2] - a[2])).clamp(0.0, 1.0)
            };
            (t, u)
        })
        .collect()
}

/// The parameters at which the curve meets `wanted`, a condition on the
/// point and the derivative there that is 0, such as being perpendicular
/// to the line from a point: it gives its value and how large its terms
/// are, as [`roots`] takes them. A segment or an arc is solved
/// exactly by its caller; this is for ellipses and splines.
pub(crate) fn where_on(c: &Curve, wanted: &dyn Fn(P3, P3) -> (f64, f64)) -> Vec<f64> {
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

    /// The quadratic spline through `[-1, y(-1)]`, `[1, y(1)]` with its
    /// middle control point at `[0, m]`.
    fn parabola(ends: f64, m: f64) -> Curve {
        Curve::Spline(BSpline::new(
            2,
            vec![[-1.0, ends, 0.0], [0.0, m, 0.0], [1.0, ends, 0.0]],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        ))
    }

    #[test]
    fn two_splines_that_touch_meet_once_and_two_that_nearly_touch_do_not() {
        // y = x² touches y = −x² from outside and y = 2x² from inside, at
        // the origin; y = −x² − 0.001 passes it by.
        let cup = parabola(1.0, -1.0);
        for other in [parabola(-1.0, 1.0), parabola(2.0, -2.0)] {
            for (c, d) in [(&cup, &other), (&other, &cup)] {
                let found = crossings(c, d);
                assert_eq!(found.len(), 1, "{found:?}");
                let (t, u) = found[0];
                close(t, 0.5);
                close(u, 0.5);
            }
        }
        let apart = parabola(-1.001, 0.999);
        assert!(crossings(&cup, &apart).is_empty());
        assert!(crossings(&apart, &cup).is_empty());
    }

    #[test]
    fn ellipse_placed_through_insert_near_1e16_keeps_its_axes() {
        let insert = crate::model::Insert {
            block: "E".into(),
            position: crate::model::Point3::new(1e16, 1e16, 0.0),
            scale: 2.0,
            rotation: 90.0,
            mirrored: true,
            readable_attributes: false,
            attributes: Vec::new(),
            state: std::collections::BTreeMap::new(),
            evaluated_block: None,
            name: None,
            layer: "0".into(),
            properties: Default::default(),
            annotative: false,
            annotation_scales: Vec::new(),
        };
        let placement =
            crate::model::block::Placement::of(&insert, crate::model::Point3::new(0.0, 0.0, 0.0));
        let ellipse = Curve::Ellipse {
            center: [0.0, 0.0, 0.0],
            major: [10.0, 0.0],
            minor: [0.0, 4.0],
            start: 0.0,
            sweep: TAU,
        };
        let placed = ellipse.placed(
            &|p| {
                let q = placement.point(crate::model::Point3::new(p[0], p[1], p[2]));
                [q.x, q.y, q.z]
            },
            &|v| placement.vector(v),
            placement.scale(),
            &|a| placement.radians(a),
            placement.mirrored(),
        );
        match placed {
            Curve::Ellipse { major, minor, .. } => {
                // Mirrored (x -> -x), scaled by 2, rotated +90°:
                // major [10, 0] -> [-20, 0] -> [0, -20]
                // minor [0, 4]  -> [0, 8]   -> [-8, 0]
                close(major[0], 0.0);
                close(major[1], -20.0);
                close(minor[0], -8.0);
                close(minor[1], 0.0);
            }
            other => panic!("expected ellipse, got {other:?}"),
        }
    }
}
