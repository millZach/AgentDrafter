//! Cuts lines and filled areas at the edge of a box: a viewport's window
//! on the paper. What lies on the edge is inside.

use super::Point2;

/// How close to a side of the box a segment along it lies and is left out,
/// as AutoCAD 2026 left out of a plot a line along a viewport's edge.
const ALONG_AN_EDGE: f64 = 1e-9;

/// The parts of the run of points `run` inside the box from `min` to `max`,
/// each a run of its own: a line that leaves the box and comes back is two.
/// A run of one point, a dot, is kept when it is inside. A segment that
/// lies along a side of the box is left out.
pub(super) fn clip_run(run: &[Point2], min: Point2, max: Point2) -> Vec<Vec<Point2>> {
    if let [only] = run {
        return if inside(*only, min, max) {
            vec![vec![*only]]
        } else {
            Vec::new()
        };
    }
    let mut out: Vec<Vec<Point2>> = Vec::new();
    let mut current: Vec<Point2> = Vec::new();
    for pair in run.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        match clip_segment(a, b, min, max) {
            Some((p, q, whole_start, whole_end)) => {
                if current.is_empty() || !whole_start || current.last() != Some(&p) {
                    if current.len() > 1 {
                        out.push(std::mem::take(&mut current));
                    }
                    current.clear();
                    current.push(p);
                }
                current.push(q);
                if !whole_end {
                    out.push(std::mem::take(&mut current));
                }
            }
            None => {
                if current.len() > 1 {
                    out.push(std::mem::take(&mut current));
                }
                current.clear();
            }
        }
    }
    if current.len() > 1 {
        out.push(current);
    }
    out
}

/// The part of the segment from `a` to `b` inside the box, by Liang and
/// Barsky's clipping: its ends, and whether each is the segment's own.
fn clip_segment(
    a: Point2,
    b: Point2,
    min: Point2,
    max: Point2,
) -> Option<(Point2, Point2, bool, bool)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-d[0], a[0] - min[0]),
        (d[0], max[0] - a[0]),
        (-d[1], a[1] - min[1]),
        (d[1], max[1] - a[1]),
    ] {
        if p == 0.0 {
            if q <= ALONG_AN_EDGE {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return None;
                }
                t1 = t1.min(r);
            }
        }
    }
    let at = |t: f64| {
        if t == 0.0 {
            a
        } else if t == 1.0 {
            b
        } else {
            [a[0] + t * d[0], a[1] + t * d[1]]
        }
    };
    Some((at(t0), at(t1), t0 == 0.0, t1 == 1.0))
}

/// Whether `p` is inside the box, its edge included.
fn inside(p: Point2, min: Point2, max: Point2) -> bool {
    p[0] >= min[0] && p[0] <= max[0] && p[1] >= min[1] && p[1] <= max[1]
}

/// The closed contour `contour` cut at the box, by Sutherland and
/// Hodgman's clipping, which keeps its winding: empty when nothing of it
/// is inside.
pub(crate) fn clip_polygon(contour: &[Point2], min: Point2, max: Point2) -> Vec<Point2> {
    if contour.iter().all(|&p| inside(p, min, max)) {
        return contour.to_vec();
    }
    let mut out = contour.to_vec();
    // Each edge of the box: which side is in, and where a segment crosses.
    let edges: [(usize, f64, bool); 4] = [
        (0, min[0], true),
        (0, max[0], false),
        (1, min[1], true),
        (1, max[1], false),
    ];
    for (axis, at, above) in edges {
        if out.is_empty() {
            break;
        }
        let keep = |p: Point2| if above { p[axis] >= at } else { p[axis] <= at };
        let cross = |a: Point2, b: Point2| {
            let t = (at - a[axis]) / (b[axis] - a[axis]);
            let mut p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            p[axis] = at;
            p
        };
        let input = std::mem::take(&mut out);
        for i in 0..input.len() {
            let current = input[i];
            let previous = input[(i + input.len() - 1) % input.len()];
            match (keep(previous), keep(current)) {
                (true, true) => out.push(current),
                (true, false) => out.push(cross(previous, current)),
                (false, true) => {
                    out.push(cross(previous, current));
                    out.push(current);
                }
                (false, false) => {}
            }
        }
    }
    if out.len() < 3 {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_through_the_box_is_cut_at_its_edges() {
        let runs = clip_run(&[[-5.0, 5.0], [15.0, 5.0]], [0.0, 0.0], [10.0, 10.0]);
        assert_eq!(runs, vec![vec![[0.0, 5.0], [10.0, 5.0]]]);
    }

    #[test]
    fn a_line_that_leaves_and_comes_back_is_two() {
        let runs = clip_run(
            &[[2.0, 2.0], [2.0, 20.0], [8.0, 20.0], [8.0, 2.0]],
            [0.0, 0.0],
            [10.0, 10.0],
        );
        assert_eq!(
            runs,
            vec![vec![[2.0, 2.0], [2.0, 10.0]], vec![[8.0, 10.0], [8.0, 2.0]]]
        );
    }

    #[test]
    fn a_line_along_an_edge_is_left_out() {
        let runs = clip_run(
            &[[0.0, 10.0], [10.0, 10.0], [10.0, 5.0], [5.0, 5.0]],
            [0.0, 0.0],
            [10.0, 10.0],
        );
        assert_eq!(runs, vec![vec![[10.0, 5.0], [5.0, 5.0]]]);
    }

    #[test]
    fn a_run_inside_is_kept_whole() {
        let run = vec![[1.0, 1.0], [2.0, 3.0], [4.0, 1.0]];
        assert_eq!(clip_run(&run, [0.0, 0.0], [10.0, 10.0]), vec![run]);
        assert_eq!(
            clip_run(&[[11.0, 1.0]], [0.0, 0.0], [10.0, 10.0]),
            Vec::<Vec<Point2>>::new()
        );
    }

    #[test]
    fn a_square_over_a_corner_keeps_its_part_inside() {
        let cut = clip_polygon(
            &[[5.0, 5.0], [15.0, 5.0], [15.0, 15.0], [5.0, 15.0]],
            [0.0, 0.0],
            [10.0, 10.0],
        );
        assert_eq!(
            cut,
            vec![[5.0, 10.0], [5.0, 5.0], [10.0, 5.0], [10.0, 10.0]]
        );
        assert!(clip_polygon(
            &[[20.0, 20.0], [30.0, 20.0], [30.0, 30.0]],
            [0.0, 0.0],
            [10.0, 10.0]
        )
        .is_empty());
    }
}
