//! Curves as chords: arcs, circles, ellipses and splines turned into runs
//! of points that stray at most a tolerance from the curve.
//!
//! A spline through fit points is fitted as AutoCAD fits it: a cubic with
//! its knots spaced by the chord lengths between the points and no bend at
//! either end (natural end conditions). On AutoCAD 2026's reading of a
//! built DWG, its control points match AutoCAD's to the last digit printed.

use std::f64::consts::{FRAC_PI_4, TAU};

use super::Point2;
use crate::model::{Ellipse, Spline, FIT_DEGREE};

/// The most chords one curve is drawn with.
const MOST_CHORDS: f64 = 16_384.0;

/// How many chords an arc of `radius` sweeping `sweep` radians takes so that
/// none strays more than `tolerance` from it: at least one.
pub(crate) fn arc_steps(radius: f64, sweep: f64, tolerance: f64) -> u32 {
    let radius = radius.abs();
    let sweep = sweep.abs();
    if tolerance.is_nan() || tolerance <= 0.0 || radius <= tolerance {
        return (sweep / FRAC_PI_4).ceil().clamp(1.0, MOST_CHORDS) as u32;
    }
    let step = 2.0 * (1.0 - tolerance / radius).acos();
    (sweep / step).ceil().clamp(1.0, MOST_CHORDS) as u32
}

/// Part of a circle, anticlockwise from `start` to `end` degrees.
pub(crate) fn arc(
    center: Point2,
    radius: f64,
    start: f64,
    end: f64,
    tolerance: f64,
) -> Vec<Point2> {
    let from = start.to_radians();
    let mut sweep = (end - start).to_radians().rem_euclid(TAU);
    if sweep == 0.0 {
        sweep = TAU;
    }
    swept(center, radius, from, sweep, tolerance)
}

/// A whole circle, from and back to its point at 0 degrees, anticlockwise.
pub(crate) fn circle(center: Point2, radius: f64, tolerance: f64) -> Vec<Point2> {
    swept(center, radius, 0.0, TAU, tolerance)
}

fn swept(center: Point2, radius: f64, from: f64, sweep: f64, tolerance: f64) -> Vec<Point2> {
    let steps = arc_steps(radius, sweep, tolerance);
    (0..=steps)
        .map(|i| {
            let angle = from + sweep * f64::from(i) / f64::from(steps);
            let (sin, cos) = angle.sin_cos();
            [center[0] + radius * cos, center[1] + radius * sin]
        })
        .collect()
}

/// An ellipse, or the part of it from its start angle anticlockwise to its
/// end angle.
pub(crate) fn ellipse(e: &Ellipse, tolerance: f64) -> Vec<Point2> {
    let major = [e.major_axis.x, e.major_axis.y];
    let minor = [-major[1] * e.ratio, major[0] * e.ratio];
    let (from, sweep) = if e.is_whole() {
        (0.0, TAU)
    } else {
        let start = parameter(e.start_angle, e.ratio);
        let end = parameter(e.end_angle, e.ratio);
        let sweep = (end - start).rem_euclid(TAU);
        (start, if sweep == 0.0 { TAU } else { sweep })
    };
    // Even steps of the parameter stray from the ellipse no more than the
    // same steps of angle stray from the circle round its major axis: where
    // it bends tightest, at the ends of the major axis, it moves slowest.
    let steps = arc_steps(major[0].hypot(major[1]), sweep, tolerance);
    (0..=steps)
        .map(|i| {
            let t = from + sweep * f64::from(i) / f64::from(steps);
            let (sin, cos) = t.sin_cos();
            [
                e.center.x + cos * major[0] + sin * minor[0],
                e.center.y + cos * major[1] + sin * minor[1],
            ]
        })
        .collect()
}

/// The parameter of an ellipse's point at `degrees` from its major axis, as
/// the DWG keeps it: the point at parameter `t` is `centre + cos t · major +
/// sin t · minor`, so its angle `θ` has `tan θ = ratio · tan t`.
fn parameter(degrees: f64, ratio: f64) -> f64 {
    let theta = degrees.to_radians();
    theta.sin().atan2(ratio * theta.cos()).rem_euclid(TAU)
}

/// A spline: through its fit points, or by its control points.
pub(crate) fn spline(s: &Spline, tolerance: f64) -> Vec<Point2> {
    let (degree, control, knots) = if s.fit_points.is_empty() {
        let control: Vec<Point2> = s.control_points.iter().map(|p| [p.x, p.y]).collect();
        let degree = usize::from(s.degree).clamp(1, control.len().saturating_sub(1).max(1));
        let knots = clamped_uniform_knots(degree, control.len());
        (degree, control, knots)
    } else {
        // The curve is fitted in space, through the fit points as the DWG
        // holds them, and then seen from above: projecting first would
        // space the knots by the wrong distances.
        let fit: Vec<Point3d> = s.fit_points.iter().map(|p| [p.x, p.y, p.z]).collect();
        let (control, knots) = fit_cubic(&fit);
        let control = control.iter().map(|p| [p[0], p[1]]).collect();
        (usize::from(FIT_DEGREE), control, knots)
    };
    if control.len() <= degree {
        return control;
    }
    let curve = BSpline {
        degree,
        control: &control,
        knots: &knots,
    };
    let mut points = vec![curve.at(knots[degree])];
    for span in degree..control.len() {
        let (a, b) = (knots[span], knots[span + 1]);
        if b > a {
            curve.sample(a, b, curve.at(a), curve.at(b), tolerance, 0, &mut points);
        }
    }
    points
}

/// A clamped B-spline's knots, evenly spaced from 0 to 1 between `degree +
/// 1` at each end, as acadrust writes them and the core reads them.
fn clamped_uniform_knots(degree: usize, count: usize) -> Vec<f64> {
    let inner = count - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..inner).map(|i| i as f64 / inner as f64));
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    knots
}

/// A point in space: `[x, y, z]`.
type Point3d = [f64; 3];

/// The control points and knots of the cubic AutoCAD fits through `fit`:
/// knots spaced by chord length in space, natural end conditions.
fn fit_cubic(fit: &[Point3d]) -> (Vec<Point3d>, Vec<f64>) {
    let n = fit.len();
    let mut at = vec![0.0];
    for pair in fit.windows(2) {
        let last = at[at.len() - 1];
        let [dx, dy, dz] = [0, 1, 2].map(|i| pair[1][i] - pair[0][i]);
        at.push(last + dx.hypot(dy).hypot(dz));
    }
    let end = at[n - 1];
    let mut knots = vec![0.0; 4];
    knots.extend(&at[1..n - 1]);
    knots.extend([end; 4]);
    let count = n + 2;
    // The rows: the first fit point, no bend at the start, the inner fit
    // points, no bend at the end, the last fit point. Each touches at most
    // four neighbouring control points.
    let mut rows: Vec<(Vec<f64>, Point3d)> = Vec::with_capacity(count);
    let row = |entries: &[(usize, f64)]| {
        let mut r = vec![0.0; count];
        for &(i, v) in entries {
            r[i] = v;
        }
        r
    };
    rows.push((row(&[(0, 1.0)]), fit[0]));
    let (a, b) = (knots[4], knots[5]);
    rows.push((
        row(&[(0, 1.0 / a), (1, -1.0 / a - 1.0 / b), (2, 1.0 / b)]),
        [0.0; 3],
    ));
    for (k, &u) in at.iter().enumerate().take(n - 1).skip(1) {
        // Fit point k sits at knot k + 3, where three basis functions are not 0.
        let span = k + 3;
        let basis = basis_functions(span, u, 3, &knots);
        let entries: Vec<(usize, f64)> = (0..4).map(|j| (span - 3 + j, basis[j])).collect();
        rows.push((row(&entries), fit[k]));
    }
    let m = count - 1;
    let (c, d) = (knots[m + 3] - knots[m], knots[m + 2] - knots[m - 1]);
    rows.push((
        row(&[(m, 1.0 / c), (m - 1, -1.0 / c - 1.0 / d), (m - 2, 1.0 / d)]),
        [0.0; 3],
    ));
    rows.push((row(&[(m, 1.0)]), fit[n - 1]));
    (solve(rows), knots)
}

/// Solves a square system whose rows each give a point, by elimination
/// with partial pivoting. The systems here are banded, so most of each
/// row is skipped.
fn solve(mut rows: Vec<(Vec<f64>, Point3d)>) -> Vec<Point3d> {
    let n = rows.len();
    for col in 0..n {
        let pivot = (col..n.min(col + 8))
            .max_by(|&i, &j| rows[i].0[col].abs().total_cmp(&rows[j].0[col].abs()))
            .unwrap_or(col);
        rows.swap(col, pivot);
        let p = rows[col].0[col];
        if p == 0.0 {
            continue;
        }
        let reach = n.min(col + 16);
        for r in col + 1..n.min(col + 8) {
            let f = rows[r].0[col] / p;
            if f == 0.0 {
                continue;
            }
            let (head, tail) = rows.split_at_mut(r);
            let (src, dst) = (&head[col], &mut tail[0]);
            for c in col..reach {
                dst.0[c] -= f * src.0[c];
            }
            for (d, s) in dst.1.iter_mut().zip(src.1) {
                *d -= f * s;
            }
        }
    }
    let mut x = vec![[0.0; 3]; n];
    for r in (0..n).rev() {
        let mut v = rows[r].1;
        let reach = n.min(r + 16);
        for (coefficient, solved) in rows[r].0[r + 1..reach].iter().zip(&x[r + 1..reach]) {
            for (v, s) in v.iter_mut().zip(solved) {
                *v -= coefficient * s;
            }
        }
        let p = rows[r].0[r];
        x[r] = if p == 0.0 { v } else { v.map(|v| v / p) };
    }
    x
}

/// The `degree + 1` basis functions that are not 0 at `u`, in knot span
/// `span` (The NURBS Book, algorithm A2.2).
fn basis_functions(span: usize, u: f64, degree: usize, knots: &[f64]) -> Vec<f64> {
    let mut n = vec![0.0; degree + 1];
    let mut left = vec![0.0; degree + 1];
    let mut right = vec![0.0; degree + 1];
    n[0] = 1.0;
    for j in 1..=degree {
        left[j] = u - knots[span + 1 - j];
        right[j] = knots[span + j] - u;
        let mut saved = 0.0;
        for r in 0..j {
            let denominator = right[r + 1] + left[j - r];
            let temp = if denominator == 0.0 {
                0.0
            } else {
                n[r] / denominator
            };
            n[r] = saved + right[r + 1] * temp;
            saved = left[j - r] * temp;
        }
        n[j] = saved;
    }
    n
}

struct BSpline<'a> {
    degree: usize,
    control: &'a [Point2],
    knots: &'a [f64],
}

impl BSpline<'_> {
    /// The point at parameter `u` (de Boor's algorithm).
    fn at(&self, u: f64) -> Point2 {
        let p = self.degree;
        let last = self.control.len() - 1;
        let span = (p..=last)
            .rev()
            .find(|&i| self.knots[i] <= u && (u < self.knots[i + 1] || i == last))
            .unwrap_or(p);
        let mut d: Vec<Point2> = (0..=p).map(|j| self.control[span - p + j]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = span - p + j;
                let denominator = self.knots[i + p + 1 - r] - self.knots[i];
                let alpha = if denominator == 0.0 {
                    0.0
                } else {
                    (u - self.knots[i]) / denominator
                };
                d[j] = [
                    (1.0 - alpha) * d[j - 1][0] + alpha * d[j][0],
                    (1.0 - alpha) * d[j - 1][1] + alpha * d[j][1],
                ];
            }
        }
        d[p]
    }

    /// Adds points from `a` (already added) to `b`, halving each piece until
    /// its middle sits within `tolerance` of its chord.
    #[allow(clippy::too_many_arguments)]
    fn sample(
        &self,
        a: f64,
        b: f64,
        pa: Point2,
        pb: Point2,
        tolerance: f64,
        depth: u32,
        out: &mut Vec<Point2>,
    ) {
        let m = (a + b) / 2.0;
        let pm = self.at(m);
        let off = distance_to_chord(pm, pa, pb);
        // Halve at least three times, so a curve that comes back on itself
        // within one piece is not taken for its chord.
        if depth < 3 || (off > tolerance && depth < 16) {
            self.sample(a, m, pa, pm, tolerance, depth + 1, out);
            self.sample(m, b, pm, pb, tolerance, depth + 1, out);
        } else {
            out.push(pb);
        }
    }
}

fn distance_to_chord(p: Point2, a: Point2, b: Point2) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    if length == 0.0 {
        (p[0] - a[0]).hypot(p[1] - a[1])
    } else {
        ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / length
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Point3, LAYER_ZERO};

    #[track_caller]
    fn close(a: Point2, b: Point2, within: f64) {
        assert!(
            (a[0] - b[0]).abs() <= within && (a[1] - b[1]).abs() <= within,
            "{a:?} is not {b:?}"
        );
    }

    /// AutoCAD 2026's control points for a spline through four fit points,
    /// read from its DXF of a DWG the build wrote.
    #[test]
    fn a_spline_through_fit_points_has_autocads_control_points() {
        let fit = [
            [180.0, 0.0, 0.0],
            [200.0, 30.0, 0.0],
            [220.0, 10.0, 0.0],
            [240.0, 40.0, 0.0],
        ];
        let (control, knots) = fit_cubic(&fit);
        let autocad = [
            [180.0, 0.0],
            [186.0088385052214, 16.64341943902647],
            [196.7313977476969, 46.34301126151533],
            [223.2686022523031, -6.343011261515334],
            [233.9911614947786, 23.35658056097352],
            [240.0, 40.0],
        ];
        assert_eq!(control.len(), autocad.len());
        for (mine, theirs) in control.iter().zip(autocad) {
            close([mine[0], mine[1]], theirs, 1e-9);
            assert_eq!(mine[2], 0.0);
        }
        assert!((knots[4] - 36.055_512_754_64).abs() < 1e-9);
        assert!((knots[5] - 64.339_784_002_1).abs() < 1e-9);
    }

    #[test]
    fn a_spline_passes_through_its_fit_points_and_ends_at_its_control_points() {
        let fit: Vec<Point3> = [[0.0, 95.0], [30.0, 120.0], [60.0, 100.0], [90.0, 125.0]]
            .iter()
            .map(|p| Point3::new(p[0], p[1], 0.0))
            .collect();
        let through = Spline {
            fit_points: fit.clone(),
            control_points: Vec::new(),
            degree: 3,
            layer: LAYER_ZERO.into(),
        };
        let points = spline(&through, 1e-4);
        for p in &fit {
            let nearest = points
                .iter()
                .map(|q| (q[0] - p.x).hypot(q[1] - p.y))
                .fold(f64::MAX, f64::min);
            assert!(nearest < 0.05, "{p:?} is {nearest} from the curve");
        }
        let by_control = Spline {
            fit_points: Vec::new(),
            control_points: fit.clone(),
            degree: 2,
            layer: LAYER_ZERO.into(),
        };
        let points = spline(&by_control, 1e-4);
        close(points[0], [0.0, 95.0], 1e-12);
        close(*points.last().unwrap(), [90.0, 125.0], 1e-12);
    }

    /// A spline that rises out of the plane is fitted by its distances in
    /// space, as the DWG defines it, and only then seen from above. Seen
    /// from above first, the last two of these fit points would coincide.
    #[test]
    fn a_spline_out_of_the_plane_is_fitted_in_space_then_seen_from_above() {
        let fit = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 0.0, 10.0]];
        let (control, knots) = fit_cubic(&fit);
        assert_eq!(knots, [0.0, 0.0, 0.0, 0.0, 10.0, 20.0, 20.0, 20.0, 20.0]);
        assert!(
            control.iter().flatten().all(|c| c.is_finite()),
            "{control:?}"
        );
        let s = Spline {
            fit_points: fit.iter().map(|p| Point3::new(p[0], p[1], p[2])).collect(),
            control_points: Vec::new(),
            degree: 3,
            layer: LAYER_ZERO.into(),
        };
        let points = spline(&s, 1e-4);
        assert!(points.len() > 2, "{points:?}");
        assert!(points.iter().flatten().all(|c| c.is_finite()), "{points:?}");
        close(points[0], [0.0, 0.0], 1e-12);
        close(*points.last().unwrap(), [10.0, 0.0], 1e-9);
        // The curve passes through the middle fit point.
        let nearest = points
            .iter()
            .map(|q| (q[0] - 10.0).hypot(q[1]))
            .fold(f64::MAX, f64::min);
        assert!(nearest < 1e-3, "{nearest}");
    }

    #[test]
    fn chords_stay_within_the_tolerance() {
        let points = circle([0.0, 0.0], 10.0, 0.01);
        close(points[0], [10.0, 0.0], 1e-12);
        close(*points.last().unwrap(), [10.0, 0.0], 1e-9);
        for pair in points.windows(2) {
            let mid = [
                (pair[0][0] + pair[1][0]) / 2.0,
                (pair[0][1] + pair[1][1]) / 2.0,
            ];
            assert!(10.0 - mid[0].hypot(mid[1]) <= 0.01 + 1e-12);
        }
        // An arc from 270 to 90 runs anticlockwise through 0.
        let half = arc([0.0, 0.0], 1.0, 270.0, 90.0, 0.001);
        assert!(half.iter().all(|p| p[0] >= -1e-12), "{half:?}");
    }

    #[test]
    fn an_elliptical_arc_runs_from_its_start_angle_to_its_end_angle() {
        let e = Ellipse {
            center: Point3::new(0.0, 0.0, 0.0),
            major_axis: Point3::new(40.0, 0.0, 0.0),
            ratio: 0.6,
            start_angle: 0.0,
            end_angle: 180.0,
            layer: LAYER_ZERO.into(),
        };
        let points = ellipse(&e, 0.01);
        close(points[0], [40.0, 0.0], 1e-9);
        close(*points.last().unwrap(), [-40.0, 0.0], 1e-9);
        assert!(points.iter().all(|p| p[1] >= -1e-9));
        let top = points.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
        assert!((top - 24.0).abs() < 0.01, "{top}");
    }
}
