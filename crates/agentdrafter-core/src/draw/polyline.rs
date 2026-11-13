//! The geometry of a flat polyline's segments: a straight segment or an
//! arc by its bulge, and the band a wide segment fills, as AutoCAD draws
//! them.
//!
//! A segment's bulge is the tangent of a quarter of its arc's included
//! angle, positive anticlockwise. A wide segment fills a band centred on
//! its path, its width changing evenly from its start to its end, with
//! square ends; where two wide segments meet, their edges are carried on
//! to where they cross, so the corner is mitred, as AutoCAD joins them. A
//! wide segment in a linetype fills a piece of its band under each dash,
//! with square ends and no mitre. The bands are filled together, by the
//! non-zero rule, so where two overlap, at a corner that turns back on
//! itself, the overlap is filled; AutoCAD 2026's plot of a made-up probe
//! filled such an overlap only in alternate triangles of its own.

use super::{curves, linetype, Point2};

/// A segment's arc: its centre, radius, the angle it starts at and the
/// angle it sweeps, in radians, positive anticlockwise.
#[derive(Clone, Copy, Debug)]
struct SegmentArc {
    center: Point2,
    radius: f64,
    start: f64,
    sweep: f64,
}

/// The arc from `a` to `b` of `bulge`; none for a straight segment.
fn arc_of(a: Point2, b: Point2, bulge: f64) -> Option<SegmentArc> {
    let chord = [b[0] - a[0], b[1] - a[1]];
    let length = chord[0].hypot(chord[1]);
    if bulge == 0.0 || !bulge.is_finite() || length == 0.0 {
        return None;
    }
    let half = length / 2.0;
    let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    // The chord's left normal: an anticlockwise arc bulges to its right,
    // so its centre lies towards the left by the sagitta's complement.
    let left = [-chord[1] / length, chord[0] / length];
    let off = half * (1.0 - bulge * bulge) / (2.0 * bulge);
    let center = [middle[0] + left[0] * off, middle[1] + left[1] * off];
    let radius = half * (1.0 + bulge * bulge) / (2.0 * bulge.abs());
    let start = (a[1] - center[1]).atan2(a[0] - center[0]);
    let sweep = 4.0 * bulge.atan();
    Some(SegmentArc {
        center,
        radius,
        start,
        sweep,
    })
}

/// The path of the segment from `a` to `b` of `bulge`: its two ends, or
/// its arc as chords within `tolerance` of it, from `a` to `b`.
pub(super) fn segment(a: Point2, b: Point2, bulge: f64, tolerance: f64) -> Vec<Point2> {
    let Some(arc) = arc_of(a, b, bulge) else {
        return vec![a, b];
    };
    let steps = curves::arc_steps(arc.radius, arc.sweep, tolerance);
    let mut path: Vec<Point2> = (0..=steps)
        .map(|i| {
            let angle = arc.start + arc.sweep * f64::from(i) / f64::from(steps);
            let (sin, cos) = angle.sin_cos();
            [
                arc.center[0] + arc.radius * cos,
                arc.center[1] + arc.radius * sin,
            ]
        })
        .collect();
    // The ends exactly where the points are.
    path[0] = a;
    *path.last_mut().expect("an arc has two ends") = b;
    path
}

/// The band one wide segment fills: its left and right edges, from its
/// start to its end, and the direction it runs at each end.
#[derive(Clone, Debug)]
struct Band {
    left: Vec<Point2>,
    right: Vec<Point2>,
    start_direction: Point2,
    end_direction: Point2,
}

/// The band of the segment from `a` to `b` of `bulge`, `widths` wide at
/// its start and end; none when it has no length or no width.
fn band(a: Point2, b: Point2, bulge: f64, widths: [f64; 2], tolerance: f64) -> Option<Band> {
    if a == b || (widths[0] <= 0.0 && widths[1] <= 0.0) {
        return None;
    }
    let half = |t: f64| (widths[0] + (widths[1] - widths[0]) * t) / 2.0;
    match arc_of(a, b, bulge) {
        None => {
            let d = [b[0] - a[0], b[1] - a[1]];
            let length = d[0].hypot(d[1]);
            let unit = [d[0] / length, d[1] / length];
            let normal = [-unit[1], unit[0]];
            let at = |p: Point2, h: f64, side: f64| {
                [p[0] + normal[0] * h * side, p[1] + normal[1] * h * side]
            };
            Some(Band {
                left: vec![at(a, half(0.0), 1.0), at(b, half(1.0), 1.0)],
                right: vec![at(a, half(0.0), -1.0), at(b, half(1.0), -1.0)],
                start_direction: unit,
                end_direction: unit,
            })
        }
        Some(arc) => {
            let path = segment(a, b, bulge, tolerance);
            let steps = path.len() - 1;
            let turn = arc.sweep.signum();
            let mut left = Vec::with_capacity(path.len());
            let mut right = Vec::with_capacity(path.len());
            for (i, p) in path.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f64 / steps as f64;
                let angle = arc.start + arc.sweep * t;
                let (sin, cos) = angle.sin_cos();
                // Outward from the centre; the path's left is inward for
                // an anticlockwise arc.
                let inward = [-cos * turn, -sin * turn];
                let h = half(t);
                left.push([p[0] + inward[0] * h, p[1] + inward[1] * h]);
                right.push([p[0] - inward[0] * h, p[1] - inward[1] * h]);
            }
            let direction = |angle: f64| {
                let (sin, cos) = angle.sin_cos();
                [-sin * turn, cos * turn]
            };
            Some(Band {
                left,
                right,
                start_direction: direction(arc.start),
                end_direction: direction(arc.start + arc.sweep),
            })
        }
    }
}

/// Where the line through `p` along `u` crosses the line through `q`
/// along `v`; none when they are parallel.
fn crossing(p: Point2, u: Point2, q: Point2, v: Point2) -> Option<Point2> {
    let cross = u[0] * v[1] - u[1] * v[0];
    if cross.abs() < 1e-12 {
        return None;
    }
    let t = ((q[0] - p[0]) * v[1] - (q[1] - p[1]) * v[0]) / cross;
    Some([p[0] + u[0] * t, p[1] + u[1] * t])
}

/// Mitres the corner where `before` ends and `after` starts: each edge is
/// carried on to where it crosses the other's. A corner that turns back
/// on itself, or does not turn, is left square.
fn mitre(before: &mut Band, after: &mut Band) {
    let u = before.end_direction;
    let v = after.start_direction;
    let turn = u[0] * v[1] - u[1] * v[0];
    let along = u[0] * v[0] + u[1] * v[1];
    if turn.abs() < 1e-9 || along < -0.999 {
        return;
    }
    for (edge_before, edge_after) in [
        (&mut before.left, &mut after.left),
        (&mut before.right, &mut after.right),
    ] {
        let end = *edge_before.last().expect("an edge has two ends");
        let start = edge_after[0];
        if let Some(corner) = crossing(end, u, start, v) {
            *edge_before.last_mut().expect("an edge has two ends") = corner;
            edge_after[0] = corner;
        }
    }
}

/// The areas a flat polyline's wide segments fill, each a closed contour
/// running anticlockwise, to be filled by the non-zero rule, and the paths of its thin segments,
/// each from its start to its end.
pub(super) struct Drawn {
    pub fills: Vec<Vec<Point2>>,
    pub thin: Vec<Vec<Point2>>,
}

/// Draws a flat polyline through `points`, closed or not, the segment
/// starting at each point taking its `bulge` and `widths`, its arcs within
/// `tolerance`. A wide segment is cut into the dashes of `pattern`, when it
/// has one, laid out along it as along a thin one.
pub(super) fn draw(
    points: &[Point2],
    closed: bool,
    bulge: impl Fn(usize) -> f64,
    widths: impl Fn(usize) -> [f64; 2],
    pattern: Option<&[f64]>,
    tolerance: f64,
) -> Drawn {
    let count = points.len();
    let segments = if closed && count > 1 {
        count
    } else {
        count.saturating_sub(1)
    };
    let mut bands: Vec<Option<Band>> = Vec::with_capacity(segments);
    let mut thin = Vec::new();
    let mut dashes = Vec::new();
    for i in 0..segments {
        let (a, b) = (points[i], points[(i + 1) % count]);
        let w = widths(i);
        if (w[0] > 0.0 || w[1] > 0.0) && pattern.is_some() {
            bands.push(None);
            dashes.extend(dashed(a, b, bulge(i), w, pattern, tolerance));
        } else if w[0] > 0.0 || w[1] > 0.0 {
            bands.push(band(a, b, bulge(i), w, tolerance));
        } else {
            bands.push(None);
            thin.push(segment(a, b, bulge(i), tolerance));
        }
    }
    let joins = if closed {
        segments
    } else {
        segments.saturating_sub(1)
    };
    for i in 0..joins {
        let j = (i + 1) % segments;
        if i == j {
            continue;
        }
        let (first, second) = if i < j {
            let (head, tail) = bands.split_at_mut(j);
            (&mut head[i], &mut tail[0])
        } else {
            let (head, tail) = bands.split_at_mut(i);
            (&mut tail[0], &mut head[j])
        };
        if let (Some(before), Some(after)) = (first.as_mut(), second.as_mut()) {
            mitre(before, after);
        }
    }
    let mut fills: Vec<Vec<Point2>> = bands
        .into_iter()
        .flatten()
        .map(|band| {
            let mut contour = band.left;
            contour.extend(band.right.into_iter().rev());
            anticlockwise(contour)
        })
        .collect();
    fills.extend(dashes.into_iter().map(anticlockwise));
    Drawn { fills, thin }
}

/// The pieces of a wide segment's band under each dash `pattern` lays
/// along its path. A dot fills nothing.
fn dashed(
    a: Point2,
    b: Point2,
    bulge: f64,
    widths: [f64; 2],
    pattern: Option<&[f64]>,
    tolerance: f64,
) -> Vec<Vec<Point2>> {
    if a == b {
        return Vec::new();
    }
    let path = segment(a, b, bulge, tolerance);
    let arc = arc_of(a, b, bulge);
    let chord = [b[0] - a[0], b[1] - a[1]];
    let length = chord[0].hypot(chord[1]);
    let unit = [chord[0] / length, chord[1] / length];
    // How far along the segment `p` is, from 0 to 1, and the way its left
    // edge lies from it.
    let locate = |p: Point2| -> (f64, Point2) {
        match arc {
            None => {
                let t = ((p[0] - a[0]) * unit[0] + (p[1] - a[1]) * unit[1]) / length;
                (t, [-unit[1], unit[0]])
            }
            Some(arc) => {
                let angle = (p[1] - arc.center[1]).atan2(p[0] - arc.center[0]);
                let turned = if arc.sweep >= 0.0 {
                    (angle - arc.start).rem_euclid(std::f64::consts::TAU)
                } else {
                    -(arc.start - angle).rem_euclid(std::f64::consts::TAU)
                };
                let t = (turned / arc.sweep).clamp(0.0, 1.0);
                let turn = arc.sweep.signum();
                let (sin, cos) = angle.sin_cos();
                (t, [-cos * turn, -sin * turn])
            }
        }
    };
    linetype::along(&path, pattern, false)
        .into_iter()
        .filter(|run| run.len() > 1)
        .map(|run| {
            let mut left = Vec::with_capacity(run.len());
            let mut right = Vec::with_capacity(run.len());
            for p in run {
                let (t, side) = locate(p);
                let h = (widths[0] + (widths[1] - widths[0]) * t) / 2.0;
                left.push([p[0] + side[0] * h, p[1] + side[1] * h]);
                right.push([p[0] - side[0] * h, p[1] - side[1] * h]);
            }
            left.extend(right.into_iter().rev());
            left
        })
        .collect()
}

/// `contour` running anticlockwise, turned round if it runs clockwise.
pub(super) fn anticlockwise(mut contour: Vec<Point2>) -> Vec<Point2> {
    let n = contour.len();
    let area: f64 = (0..n)
        .map(|i| {
            let (p, q) = (contour[i], contour[(i + 1) % n]);
            p[0] * q[1] - q[0] * p[1]
        })
        .sum();
    if area < 0.0 {
        contour.reverse();
    }
    contour
}

/// The included angle, in degrees, of a segment of `bulge`.
#[cfg(test)]
pub(crate) fn arc_angle_of(bulge: f64) -> f64 {
    (4.0 * bulge.atan()).to_degrees()
}

/// The bulge of a segment whose arc's included angle is `degrees`.
pub(crate) fn bulge_of(degrees: f64) -> f64 {
    (degrees.to_radians() / 4.0).tan()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn near(a: Point2, b: Point2) {
        assert!(
            (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9,
            "{a:?} is not {b:?}"
        );
    }

    #[test]
    fn a_bulge_of_one_is_a_half_circle_anticlockwise() {
        let path = segment([1.0, 0.0], [-1.0, 0.0], 1.0, 1e-4);
        near(path[0], [1.0, 0.0]);
        near(*path.last().unwrap(), [-1.0, 0.0]);
        // Anticlockwise from (1, 0) to (-1, 0) round the origin passes (0, 1).
        assert!(path
            .iter()
            .any(|p| (p[0].abs() < 0.01) && (p[1] - 1.0).abs() < 1e-3));
        for p in &path {
            assert!((p[0].hypot(p[1]) - 1.0).abs() < 1e-9, "{p:?}");
        }
        let clockwise = segment([1.0, 0.0], [-1.0, 0.0], -1.0, 1e-4);
        assert!(clockwise.iter().all(|p| p[1] <= 1e-12));
    }

    #[test]
    fn a_bulge_and_an_arc_angle_are_one_another() {
        assert!((bulge_of(180.0) - 1.0).abs() < 1e-12);
        assert!((arc_angle_of(1.0) - 180.0).abs() < 1e-9);
        assert!((arc_angle_of(bulge_of(-90.0)) + 90.0).abs() < 1e-9);
    }

    #[test]
    fn a_wide_straight_segment_fills_a_band_with_square_ends() {
        let drawn = draw(
            &[[0.0, 0.0], [10.0, 0.0]],
            false,
            |_| 0.0,
            |_| [2.0, 2.0],
            None,
            1e-3,
        );
        assert!(drawn.thin.is_empty());
        let band = &drawn.fills[0];
        for corner in [[0.0, -1.0], [10.0, -1.0], [10.0, 1.0], [0.0, 1.0]] {
            assert!(
                band.iter()
                    .any(|p| (p[0] - corner[0]).abs() < 1e-12 && (p[1] - corner[1]).abs() < 1e-12),
                "{band:?}"
            );
        }
    }

    #[test]
    fn a_tapered_segment_narrows_to_its_end_width() {
        let drawn = draw(
            &[[0.0, 0.0], [10.0, 0.0]],
            false,
            |_| 0.0,
            |_| [4.0, 0.0],
            None,
            1e-3,
        );
        let band = &drawn.fills[0];
        assert!(band
            .iter()
            .any(|p| (p[0] - 10.0).abs() < 1e-12 && p[1].abs() < 1e-12));
        assert!(band
            .iter()
            .any(|p| p[0].abs() < 1e-12 && (p[1] - 2.0).abs() < 1e-12));
    }

    #[test]
    fn two_wide_segments_meet_in_a_mitred_corner() {
        let drawn = draw(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            false,
            |_| 0.0,
            |_| [2.0, 2.0],
            None,
            1e-3,
        );
        // The outer corner is carried out to (11, -1), the inner in to (9, 1).
        for band in &drawn.fills {
            assert!(
                band.iter()
                    .any(|p| (p[0] - 11.0).abs() < 1e-9 && (p[1] + 1.0).abs() < 1e-9),
                "{band:?}"
            );
            assert!(
                band.iter()
                    .any(|p| (p[0] - 9.0).abs() < 1e-9 && (p[1] - 1.0).abs() < 1e-9),
                "{band:?}"
            );
        }
    }

    #[test]
    fn a_thin_segment_beside_a_wide_one_is_a_path() {
        let widths = |i: usize| if i == 0 { [1.0, 1.0] } else { [0.0, 0.0] };
        let drawn = draw(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            false,
            |_| 0.0,
            widths,
            None,
            1e-3,
        );
        assert_eq!(drawn.fills.len(), 1);
        assert_eq!(drawn.thin, vec![vec![[10.0, 0.0], [10.0, 10.0]]]);
    }

    #[test]
    fn a_wide_dashed_segment_fills_a_piece_of_its_band_under_each_dash() {
        let drawn = draw(
            &[[0.0, 0.0], [20.0, 0.0]],
            false,
            |_| 0.0,
            |_| [2.0, 2.0],
            Some(&[5.0, -2.5]),
            1e-3,
        );
        let dashes = linetype::along(&[[0.0, 0.0], [20.0, 0.0]], Some(&[5.0, -2.5]), false);
        assert_eq!(drawn.fills.len(), dashes.len());
        for (piece, dash) in drawn.fills.iter().zip(&dashes) {
            let xs: Vec<f64> = piece.iter().map(|p| p[0]).collect();
            let low = xs.iter().copied().fold(f64::MAX, f64::min);
            let high = xs.iter().copied().fold(f64::MIN, f64::max);
            assert!(
                (low - dash[0][0]).abs() < 1e-9 && (high - dash.last().unwrap()[0]).abs() < 1e-9
            );
            assert!(
                piece.iter().all(|p| (p[1].abs() - 1.0).abs() < 1e-9),
                "{piece:?}"
            );
        }
    }

    #[test]
    fn a_wide_arc_fills_a_ring_sector() {
        let drawn = draw(
            &[[1.0, 0.0], [-1.0, 0.0]],
            false,
            |_| 1.0,
            |_| [0.5, 0.5],
            None,
            1e-4,
        );
        for p in &drawn.fills[0] {
            let r = p[0].hypot(p[1]);
            assert!(
                (r - 0.75).abs() < 1e-9 || (r - 1.25).abs() < 1e-9,
                "{p:?} at {r}"
            );
        }
    }
}
