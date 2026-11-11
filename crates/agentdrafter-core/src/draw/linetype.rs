//! Linetypes: a pattern of dashes, gaps and dots laid along a path.
//!
//! AutoCAD lays a linetype out along each object, and along each segment of
//! a flat polyline, so that both ends draw alike and draw something: each
//! end takes the pattern's first element, a dash or a dot, and the length
//! left over after whole repeats is shared between the two ends, so a dash
//! at an end may be shorter or longer than the pattern's. Whole repeats run
//! between them. Round a closed curve, a circle or a whole ellipse, the
//! pattern instead stretches or shrinks to the nearest whole number of
//! repeats, from the curve's start. An open path shorter than one repeat of
//! the pattern draws continuous.
//! (Seen in AutoCAD 2026's plots of lines, polyline segments, arcs,
//! circles, ellipses and splines of many lengths.)

use super::Point2;

/// The most dashes one path is drawn with. A path that would take more is
/// drawn continuous, as AutoCAD shows a linetype too fine to see.
const MOST_DASHES: f64 = 100_000.0;

/// The runs a path draws in a linetype with `pattern`, or as one continuous
/// run when there is none. `closed` is for a path that comes back to its
/// start as a circle does. A run of one point is a dot. A path with no
/// length is a dot.
pub(crate) fn along(path: &[Point2], pattern: Option<&[f64]>, closed: bool) -> Vec<Vec<Point2>> {
    let path: Vec<Point2> = dedup(path);
    if path.len() < 2 {
        return path.first().map(|&p| vec![vec![p]]).unwrap_or_default();
    }
    let lengths = cumulative(&path);
    let total = lengths[lengths.len() - 1];
    let Some(pattern) = pattern else {
        return vec![path];
    };
    let repeat: f64 = pattern.iter().map(|l| l.abs()).sum();
    // Round a closed curve the pattern stretches or shrinks to the nearest
    // whole number of repeats; along an open path, whole repeats fit inside.
    let repeats = if closed {
        (total / repeat).round()
    } else {
        (total / repeat).floor()
    };
    if repeat.is_nan() || repeat <= 0.0 || repeats < 1.0 || total / repeat > MOST_DASHES {
        return vec![path];
    }
    let mut runs = Vec::new();
    let mut mark = |element: f64, from: f64, to: f64| {
        if element > 0.0 && to > from {
            runs.push(piece(&path, &lengths, from, to));
        }
    };
    let dot = |at: f64| vec![point_at(&path, &lengths, at.clamp(0.0, total))];
    if closed {
        let stretch = total / (repeats * repeat);
        let mut at = 0.0;
        let mut dots = Vec::new();
        for _ in 0..repeats as usize {
            for &element in pattern {
                let length = element.abs() * stretch;
                if element == 0.0 {
                    dots.push(dot(at));
                }
                mark(element, at, at + length);
                at += length;
            }
        }
        runs.extend(dots);
        return runs;
    }
    // Both ends take the pattern's first element, each with half of what
    // is left over after whole repeats; whole repeats run between them,
    // from the pattern's second element on.
    let first = pattern[0];
    let end = (first.abs() + total - repeats * repeat) / 2.0;
    let mut dots = Vec::new();
    if first == 0.0 {
        dots.push(dot(0.0));
    }
    mark(first, 0.0, end);
    let middle = pattern[1..].iter().chain(
        pattern
            .iter()
            .cycle()
            .take(pattern.len() * (repeats as usize - 1)),
    );
    let mut at = end;
    for &element in middle {
        let length = element.abs();
        if element == 0.0 {
            dots.push(dot(at));
        }
        mark(element, at, at + length);
        at += length;
    }
    mark(first, total - end, total);
    if first == 0.0 {
        dots.push(dot(total));
    }
    runs.extend(dots);
    runs
}

fn dedup(path: &[Point2]) -> Vec<Point2> {
    let mut out: Vec<Point2> = Vec::with_capacity(path.len());
    for &p in path {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

/// The distance along the path to each of its points.
fn cumulative(path: &[Point2]) -> Vec<f64> {
    let mut lengths = Vec::with_capacity(path.len());
    let mut total = 0.0;
    lengths.push(0.0);
    for pair in path.windows(2) {
        total += (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
        lengths.push(total);
    }
    lengths
}

/// The point `at` along the path.
fn point_at(path: &[Point2], lengths: &[f64], at: f64) -> Point2 {
    let i = lengths
        .partition_point(|&l| l <= at)
        .clamp(1, path.len() - 1);
    let (a, b) = (path[i - 1], path[i]);
    let span = lengths[i] - lengths[i - 1];
    let t = if span > 0.0 {
        ((at - lengths[i - 1]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// The part of the path from `from` to `to` along it, through the points
/// between.
fn piece(path: &[Point2], lengths: &[f64], from: f64, to: f64) -> Vec<Point2> {
    let mut run = vec![point_at(path, lengths, from)];
    for (i, &l) in lengths.iter().enumerate() {
        if l > from && l < to {
            run.push(path[i]);
        }
    }
    run.push(point_at(path, lengths, to));
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where each run starts and ends along a line from 0 along X.
    fn spans(length: f64, pattern: &[f64]) -> Vec<(f64, f64)> {
        let mut found = along(&[[0.0, 0.0], [length, 0.0]], Some(pattern), false)
            .iter()
            .map(|run| (run[0][0], run[run.len() - 1][0]))
            .collect::<Vec<_>>();
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found
    }

    #[track_caller]
    fn same(found: Vec<(f64, f64)>, expected: &[(f64, f64)]) {
        assert_eq!(found.len(), expected.len(), "{found:?}");
        for (f, e) in found.iter().zip(expected) {
            assert!(
                (f.0 - e.0).abs() < 1e-9 && (f.1 - e.1).abs() < 1e-9,
                "{found:?} is not {expected:?}"
            );
        }
    }

    /// The dashes AutoCAD 2026 plotted for lines of these lengths in the
    /// pattern 10 dash, 5 gap.
    #[test]
    fn both_ends_take_half_of_what_is_left_over() {
        let dash = [10.0, -5.0];
        same(spans(12.0, &dash), &[(0.0, 12.0)]);
        same(spans(20.0, &dash), &[(0.0, 7.5), (12.5, 20.0)]);
        same(
            spans(37.0, &dash),
            &[(0.0, 8.5), (13.5, 23.5), (28.5, 37.0)],
        );
        same(
            spans(50.0, &dash),
            &[(0.0, 7.5), (12.5, 22.5), (27.5, 37.5), (42.5, 50.0)],
        );
    }

    /// AutoCAD 2026's plot of a line 37 long in the pattern 10 dash, 3 gap,
    /// dot, 3 gap.
    #[test]
    fn dots_are_runs_of_one_point() {
        same(
            spans(37.0, &[10.0, -3.0, 0.0, -3.0]),
            &[
                (0.0, 7.5),
                (10.5, 10.5),
                (13.5, 23.5),
                (26.5, 26.5),
                (29.5, 37.0),
            ],
        );
    }

    /// AutoCAD 2026's plot of an arc 44.5 long in the pattern 10 dash, 5
    /// gap: what is left over is longer than a dash, and both end dashes
    /// grow to take it.
    #[test]
    fn end_dashes_grow_when_more_than_a_dash_is_left_over() {
        same(
            spans(44.5, &[10.0, -5.0]),
            &[(0.0, 12.25), (17.25, 27.25), (32.25, 44.5)],
        );
    }

    /// AutoCAD 2026's plot of a polyline segment 20 long in the pattern
    /// dot, 3.175 gap: a dot at each end, and the dots between spaced by
    /// the pattern from half of what is left over.
    #[test]
    fn a_dotted_path_has_a_dot_at_each_end() {
        let dots: Vec<f64> = spans(20.0, &[0.0, -3.175]).iter().map(|s| s.0).collect();
        let expected = [0.0, 3.65, 6.825, 10.0, 13.175, 16.35, 20.0];
        assert_eq!(dots.len(), expected.len(), "{dots:?}");
        for (d, e) in dots.iter().zip(expected) {
            assert!((d - e).abs() < 1e-9, "{dots:?}");
        }
    }

    /// AutoCAD 2026's plots of circles 94.25 and 100.53 round in the
    /// pattern 10 dash, 5 gap: 6.28 repeats fit as six, stretched, and 6.70
    /// as seven, shrunk, from the circle's start.
    #[test]
    fn round_a_closed_curve_the_pattern_fits_the_nearest_whole_repeats() {
        let angle = |p: Point2| p[1].atan2(p[0]).to_degrees().rem_euclid(360.0);
        for (radius, repeats) in [(15.0, 6), (16.0, 7)] {
            let circle = crate::draw::curves::circle([0.0, 0.0], radius, 1e-4);
            let runs = along(&circle, Some(&[10.0, -5.0]), true);
            assert_eq!(runs.len(), repeats);
            let period = 360.0 / repeats as f64;
            let first = &runs[0];
            assert!(angle(first[0]) < 1e-6, "{:?}", first[0]);
            assert!((angle(first[first.len() - 1]) - period * 2.0 / 3.0).abs() < 1e-3);
            assert!((angle(runs[1][0]) - period).abs() < 1e-3);
        }
    }

    #[test]
    fn a_pattern_follows_the_path_round_its_corners() {
        // Dashes from 0 to 5, 7 to 13 and 15 to 20 along it: the middle one
        // turns the corner at 10.
        let runs = along(
            &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            Some(&[6.0, -2.0]),
            false,
        );
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert_eq!(runs[1], vec![[7.0, 0.0], [10.0, 0.0], [10.0, 3.0]]);
    }

    #[test]
    fn a_continuous_path_is_one_run_and_a_point_is_a_dot() {
        assert_eq!(along(&[[0.0, 0.0], [5.0, 0.0]], None, false).len(), 1);
        assert_eq!(
            along(&[[1.0, 1.0], [1.0, 1.0]], Some(&[1.0, -1.0]), false),
            vec![vec![[1.0, 1.0]]]
        );
    }
}
