//! Cuts lines and filled areas at the edge of a box: a viewport's window
//! on the paper. What lies on the edge is inside.

use super::Point2;

/// How close to a side of the box a segment along it lies and is left out,
/// on the paper, as AutoCAD 2026 left out of a plot a line along a
/// viewport's edge. A box in model space takes it over the viewport's
/// scale, so that it is as close on the paper at any scale.
pub(super) const ALONG_AN_EDGE: f64 = 1e-9;

/// The parts of the run of points `run` inside the box from `min` to `max`,
/// each a run of its own: a line that leaves the box and comes back is two.
/// A run of one point, a dot, is kept when it is inside. A segment that
/// lies along a side of the box, within `along` of it, is left out.
pub(super) fn clip_run(run: &[Point2], min: Point2, max: Point2, along: f64) -> Vec<Vec<Point2>> {
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
        match clip_segment(a, b, min, max, along) {
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
    along: f64,
) -> Option<(Point2, Point2, bool, bool)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    if !(d[0].is_finite() && d[1].is_finite()) {
        // Points so far apart that their difference overflows are cut at
        // a quarter of their size, exactly, and scaled back.
        let quarter = |p: Point2| [p[0] / 4.0, p[1] / 4.0];
        let whole = |p: Point2| [p[0] * 4.0, p[1] * 4.0];
        let (q, r) = (quarter(a), quarter(b));
        if !(r[0] - q[0]).is_finite() || !(r[1] - q[1]).is_finite() {
            return None;
        }
        return clip_segment(q, r, quarter(min), quarter(max), along / 4.0)
            .map(|(p, q, s, e)| (whole(p), whole(q), s, e));
    }
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    // The side of the box that cut each end, by its axis and where it is.
    let (mut cut0, mut cut1): (Option<Side>, Option<Side>) = (None, None);
    for (p, q, side) in [
        (-d[0], a[0] - min[0], (0, min[0])),
        (d[0], max[0] - a[0], (0, max[0])),
        (-d[1], a[1] - min[1], (1, min[1])),
        (d[1], max[1] - a[1], (1, max[1])),
    ] {
        if p == 0.0 {
            if q <= along {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return None;
                }
                if r > t0 {
                    t0 = r;
                    cut0 = Some(side);
                }
            } else {
                if r < t0 {
                    return None;
                }
                if r < t1 {
                    t1 = r;
                    cut1 = Some(side);
                }
            }
        }
    }
    // An end the box cut lies on the side that cut it, and inside the box,
    // however coarse the numbers it is worked out from.
    let at = |t: f64, cut: Option<Side>| match cut {
        None if t == 0.0 => a,
        None if t == 1.0 => b,
        _ => {
            let mut p = [a[0] + t * d[0], a[1] + t * d[1]];
            if let Some((axis, side)) = cut {
                p[axis] = side;
            }
            [p[0].clamp(min[0], max[0]), p[1].clamp(min[1], max[1])]
        }
    };
    Some((at(t0, cut0), at(t1, cut1), cut0.is_none(), cut1.is_none()))
}

/// A side of the box: its axis, 0 for x and 1 for y, and where it is.
type Side = (usize, f64);

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
            // Halved first, exactly, so that points far apart do not
            // overflow their difference.
            let (h, k) = ([a[0] / 2.0, a[1] / 2.0], [b[0] / 2.0, b[1] / 2.0]);
            let t = (at / 2.0 - h[axis]) / (k[axis] - h[axis]);
            let mut p = [
                (h[0] + t * (k[0] - h[0])) * 2.0,
                (h[1] + t * (k[1] - h[1])) * 2.0,
            ];
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

/// Whether `p` is inside the closed outline `outline`, by the even-odd
/// rule, its edge included.
pub(crate) fn inside_outline(p: Point2, outline: &[Point2]) -> bool {
    if distance_to_outline(p, outline) == 0.0 {
        return true;
    }
    let mut inside = false;
    for i in 0..outline.len() {
        let a = outline[i];
        let b = outline[(i + 1) % outline.len()];
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// How far `p` is from the nearest edge of the closed outline `outline`.
pub(super) fn distance_to_outline(p: Point2, outline: &[Point2]) -> f64 {
    (0..outline.len())
        .map(|i| {
            let a = outline[i];
            let b = outline[(i + 1) % outline.len()];
            let d = [b[0] - a[0], b[1] - a[1]];
            let length = d[0] * d[0] + d[1] * d[1];
            let t = if length > 0.0 {
                (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (p[0] - (a[0] + t * d[0])).hypot(p[1] - (a[1] + t * d[1]))
        })
        .fold(f64::INFINITY, f64::min)
}

/// Whether `p` lies within `along` of one of the edges of `outline` that
/// `square` marks.
fn along_a_square_edge(p: Point2, outline: &[Point2], square: &[bool], along: f64) -> bool {
    (0..outline.len()).any(|i| {
        if !square.get(i).copied().unwrap_or(false) {
            return false;
        }
        let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
        distance_to_outline(p, &[a, b]) <= along
    })
}

/// Where along the segment from `a` to `b`, from 0 to 1, it meets the
/// segment from `c` to `d`; none when they are parallel or do not meet.
fn meets(a: Point2, b: Point2, c: Point2, d: Point2) -> Option<f64> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let denominator = r[0] * s[1] - r[1] * s[0];
    if denominator == 0.0 {
        return None;
    }
    let q = [c[0] - a[0], c[1] - a[1]];
    let t = (q[0] * s[1] - q[1] * s[0]) / denominator;
    let u = (q[0] * r[1] - q[1] * r[0]) / denominator;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

/// The parts of the run of points `run` inside the closed outline
/// `outline`, each a run of its own, as [`clip_run`] cuts a run at a box.
/// A part that lies along one of the outline's edges that `square` marks,
/// within `along` of it, is left out: AutoCAD 2026 left out of its plot a
/// line along a clipped viewport's straight edge that runs across or up
/// the paper, as along a rectangular viewport's, and drew one along a
/// slanting edge or an arc. `square` holds a mark for each edge, from
/// point `i` to the next. A part within `near` of the outline counts as on
/// it, and is kept: AutoCAD 2026 drew a circle lying on a clipping circle,
/// whose chords and the outline's differ by up to the drawing's tolerance.
/// The run lies within the outline's box, so that nothing overflows.
pub(super) fn clip_run_to_outline(
    run: &[Point2],
    outline: &[Point2],
    square: &[bool],
    along: f64,
    near: f64,
) -> Vec<Vec<Point2>> {
    if let [only] = run {
        return if inside_outline(*only, outline) {
            vec![vec![*only]]
        } else {
            Vec::new()
        };
    }
    let mut out: Vec<Vec<Point2>> = Vec::new();
    let mut current: Vec<Point2> = Vec::new();
    for pair in run.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        // The segment cut where it meets the outline's edges.
        let mut cuts = vec![0.0, 1.0];
        for i in 0..outline.len() {
            let (c, d) = (outline[i], outline[(i + 1) % outline.len()]);
            if let Some(t) = meets(a, b, c, d) {
                cuts.push(t);
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let at = |t: f64| {
            if t == 0.0 {
                a
            } else if t == 1.0 {
                b
            } else {
                [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
            }
        };
        for piece in cuts.windows(2) {
            let (p, q) = (at(piece[0]), at(piece[1]));
            let middle = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
            let kept = (p == q || !along_a_square_edge(middle, outline, square, along))
                && (inside_outline(middle, outline)
                    || distance_to_outline(middle, outline) <= near);
            if kept {
                if current.last() != Some(&p) {
                    if current.len() > 1 {
                        out.push(std::mem::take(&mut current));
                    }
                    current.clear();
                    current.push(p);
                }
                current.push(q);
            } else if current.len() > 1 {
                out.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
    }
    if current.len() > 1 {
        out.push(current);
    }
    out
}

/// The closed contour `subject` cut at the convex contour `convex`, by
/// Sutherland and Hodgman's clipping, whichever way either winds: empty
/// when nothing of it is inside.
pub(crate) fn clip_to_convex(subject: &[Point2], convex: &[Point2]) -> Vec<Point2> {
    // Which side of each edge is in: the side the convex contour's area is.
    let area: f64 = (0..convex.len())
        .map(|i| {
            let (a, b) = (convex[i], convex[(i + 1) % convex.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let sign = if area < 0.0 { -1.0 } else { 1.0 };
    let mut out = subject.to_vec();
    for i in 0..convex.len() {
        if out.is_empty() {
            break;
        }
        let (c, d) = (convex[i], convex[(i + 1) % convex.len()]);
        let side =
            |p: Point2| sign * ((d[0] - c[0]) * (p[1] - c[1]) - (d[1] - c[1]) * (p[0] - c[0]));
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let current = input[j];
            let previous = input[(j + input.len() - 1) % input.len()];
            let (sp, sc) = (side(previous), side(current));
            let cross = || {
                let t = sp / (sp - sc);
                [
                    previous[0] + t * (current[0] - previous[0]),
                    previous[1] + t * (current[1] - previous[1]),
                ]
            };
            match (sp >= 0.0, sc >= 0.0) {
                (true, true) => out.push(current),
                (true, false) => out.push(cross()),
                (false, true) => {
                    out.push(cross());
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

/// Twice the area the closed outline `outline` encloses: more than 0 when
/// it runs anticlockwise.
fn twice_area(outline: &[Point2]) -> f64 {
    (0..outline.len())
        .map(|i| {
            let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum()
}

/// How far `c` turns left of the line from `a` through `b`: more than 0 to
/// the left, less to the right.
fn turn(a: Point2, b: Point2, c: Point2) -> f64 {
    (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
}

/// Whether the closed outline `outline` is convex: every corner turns the
/// same way, or not at all.
pub(crate) fn is_convex(outline: &[Point2]) -> bool {
    let n = outline.len();
    let turns: Vec<f64> = (0..n)
        .map(|i| turn(outline[i], outline[(i + 1) % n], outline[(i + 2) % n]))
        .collect();
    turns.iter().all(|&t| t >= 0.0) || turns.iter().all(|&t| t <= 0.0)
}

/// The closed outline `outline`, a simple polygon, cut into triangles that
/// cover it, each anticlockwise, by clipping its ears off one by one.
pub(crate) fn triangles(outline: &[Point2]) -> Vec<[Point2; 3]> {
    let mut left: Vec<Point2> = outline.to_vec();
    if twice_area(&left) < 0.0 {
        left.reverse();
    }
    let inside = |p: Point2, [a, b, c]: [Point2; 3]| {
        turn(a, b, p) >= 0.0 && turn(b, c, p) >= 0.0 && turn(c, a, p) >= 0.0
    };
    let mut out = Vec::new();
    while left.len() > 3 {
        let n = left.len();
        let ear = (0..n).find(|&i| {
            let t = [left[(i + n - 1) % n], left[i], left[(i + 1) % n]];
            turn(t[0], t[1], t[2]) > 0.0
                && !left
                    .iter()
                    .any(|&p| !t.contains(&p) && inside(p, t))
        });
        match ear {
            Some(i) => {
                out.push([left[(i + n - 1) % n], left[i], left[(i + 1) % n]]);
                left.remove(i);
            }
            None => {
                // A corner that does not turn encloses nothing: it goes.
                let Some(i) = (0..n)
                    .find(|&i| turn(left[(i + n - 1) % n], left[i], left[(i + 1) % n]) == 0.0)
                else {
                    break;
                };
                left.remove(i);
            }
        }
    }
    if left.len() == 3 && turn(left[0], left[1], left[2]) > 0.0 {
        out.push([left[0], left[1], left[2]]);
    }
    out
}

/// Whether the box from `min` to `max` and the closed outline `outline`
/// share any point.
pub(super) fn box_meets_outline(min: Point2, max: Point2, outline: &[Point2]) -> bool {
    let corners = [min, [max[0], min[1]], max, [min[0], max[1]]];
    corners.iter().any(|&p| inside_outline(p, outline))
        || outline.iter().any(|&p| inside(p, min, max))
        || (0..4).any(|i| {
            let (a, b) = (corners[i], corners[(i + 1) % 4]);
            (0..outline.len())
                .any(|j| meets(a, b, outline[j], outline[(j + 1) % outline.len()]).is_some())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_across_a_triangle_is_cut_at_its_sides() {
        let triangle = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
        let square = [true, false, true];
        let runs = clip_run_to_outline(
            &[[-5.0, 2.0], [15.0, 2.0]],
            &triangle,
            &square,
            ALONG_AN_EDGE,
            ALONG_AN_EDGE,
        );
        assert_eq!(runs, vec![vec![[0.0, 2.0], [8.0, 2.0]]]);
        // Along the slanting side: drawn, as AutoCAD 2026 drew it.
        let runs = clip_run_to_outline(
            &[[0.0, 10.0], [10.0, 0.0]],
            &triangle,
            &square,
            ALONG_AN_EDGE,
            ALONG_AN_EDGE,
        );
        assert_eq!(runs, vec![vec![[0.0, 10.0], [10.0, 0.0]]]);
        // Along the side across the paper: left out, as along a rectangle's.
        let runs = clip_run_to_outline(
            &[[0.0, 0.0], [10.0, 0.0]],
            &triangle,
            &square,
            ALONG_AN_EDGE,
            ALONG_AN_EDGE,
        );
        assert_eq!(runs, Vec::<Vec<Point2>>::new());
        // A run that leaves a notch and comes back is two.
        let notch = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [5.0, 5.0],
            [0.0, 10.0],
        ];
        let runs = clip_run_to_outline(
            &[[0.0, 8.0], [10.0, 8.0]],
            &notch,
            &[true, true, false, false, true],
            ALONG_AN_EDGE,
            ALONG_AN_EDGE,
        );
        assert_eq!(
            runs,
            vec![vec![[0.0, 8.0], [2.0, 8.0]], vec![[8.0, 8.0], [10.0, 8.0]]]
        );
    }

    #[test]
    fn a_concave_outline_is_cut_into_triangles_that_cover_it() {
        // A U, as a notched clipping boundary is.
        let u = [
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 100.0],
            [60.0, 100.0],
            [60.0, 20.0],
            [40.0, 20.0],
            [40.0, 100.0],
            [0.0, 100.0],
        ];
        assert!(!is_convex(&u));
        let cut = triangles(&u);
        assert_eq!(cut.len(), u.len() - 2);
        let area: f64 = cut.iter().map(|t| twice_area(t) / 2.0).sum();
        assert!((area - 10000.0 + 1600.0).abs() < 1e-9, "{area}");
        // None reaches into the notch.
        assert!(cut
            .iter()
            .all(|t| !inside_outline([50.0, 50.0], t) || t.contains(&[50.0, 50.0])));
        assert!(is_convex(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]));
    }

    #[test]
    fn a_contour_is_cut_at_a_convex_outline_either_way_round() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        // A diamond that cuts each of the square's corners off.
        let diamond = [[5.0, -2.0], [12.0, 5.0], [5.0, 12.0], [-2.0, 5.0]];
        let cut = clip_to_convex(&square, &diamond);
        let reversed: Vec<Point2> = diamond.iter().rev().copied().collect();
        assert_eq!(cut.len(), clip_to_convex(&square, &reversed).len());
        assert!(cut.iter().all(|&p| inside_outline(p, &diamond)), "{cut:?}");
        assert_eq!(cut.len(), 8, "{cut:?}");
    }

    #[test]
    fn a_line_through_the_box_is_cut_at_its_edges() {
        let runs = clip_run(
            &[[-5.0, 5.0], [15.0, 5.0]],
            [0.0, 0.0],
            [10.0, 10.0],
            ALONG_AN_EDGE,
        );
        assert_eq!(runs, vec![vec![[0.0, 5.0], [10.0, 5.0]]]);
    }

    #[test]
    fn a_line_that_leaves_and_comes_back_is_two() {
        let runs = clip_run(
            &[[2.0, 2.0], [2.0, 20.0], [8.0, 20.0], [8.0, 2.0]],
            [0.0, 0.0],
            [10.0, 10.0],
            ALONG_AN_EDGE,
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
            ALONG_AN_EDGE,
        );
        assert_eq!(runs, vec![vec![[10.0, 5.0], [5.0, 5.0]]]);
    }

    #[test]
    fn points_far_apart_are_cut_without_overflowing() {
        let runs = clip_run(
            &[[-1e308, 5.0], [1e308, 5.0]],
            [0.0, 0.0],
            [10.0, 10.0],
            ALONG_AN_EDGE,
        );
        assert_eq!(runs, vec![vec![[0.0, 5.0], [10.0, 5.0]]]);
        let cut = clip_polygon(
            &[
                [-1e308, -1e308],
                [1e308, -1e308],
                [1e308, 1e308],
                [-1e308, 1e308],
            ],
            [0.0, 0.0],
            [10.0, 10.0],
        );
        assert!(cut.iter().flatten().all(|v| v.is_finite()), "{cut:?}");
        assert_eq!(cut.len(), 4, "{cut:?}");
    }

    #[test]
    fn a_run_inside_is_kept_whole() {
        let run = vec![[1.0, 1.0], [2.0, 3.0], [4.0, 1.0]];
        assert_eq!(
            clip_run(&run, [0.0, 0.0], [10.0, 10.0], ALONG_AN_EDGE),
            vec![run]
        );
        assert_eq!(
            clip_run(&[[11.0, 1.0]], [0.0, 0.0], [10.0, 10.0], ALONG_AN_EDGE),
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
