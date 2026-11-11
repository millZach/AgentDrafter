//! Hatches: an area filled solid, or drawn with its pattern's lines.
//!
//! A point inside an odd number of the boundary's loops is filled, as
//! AutoCAD's normal island detection reads a hatch. A solid hatch is one
//! fill. A patterned hatch draws each family of parallel lines across the
//! boundary, clipped to the filled area, with its dashes counted from the
//! line's own origin, as AutoCAD draws a `.pat` pattern: the lines are
//! turned by the hatch's angle, their spacing and dashes enlarged by its
//! scale, and the pattern is anchored at the drawing's origin. Pattern
//! lines draw with the layer's pen; the layer's linetype does not apply.

use super::curves;
use super::{Bounds, FillRule, Point2, Shape};
use crate::model::hatch::walk;
use crate::model::{Hatch, HatchEdge, HatchLoop};

/// The most pattern lines one hatch draws, as AutoCAD's `HPMAXLINES`
/// allows by default. A pattern that would need more draws only the first.
const MOST_LINES: usize = 1_000_000;

/// The most dashes one hatch draws. A pattern that would need more draws
/// its lines continuous, as a linetype too fine to see does.
const MOST_DASHES: usize = 2_000_000;

/// The shapes a hatch draws, a fill or one stroke of all its pattern lines,
/// and the box round its boundary.
pub(super) fn draw(hatch: &Hatch, lineweight: f64, tolerance: f64) -> (Vec<Shape>, Option<Bounds>) {
    let loops: Vec<Vec<Point2>> = hatch
        .boundary
        .iter()
        .map(|l| boundary_loop(l, tolerance))
        .filter(|l| l.len() >= 3)
        .collect();
    let bounds = Bounds::of(loops.iter().flatten().copied());
    (shapes(hatch, loops, lineweight), bounds)
}

fn shapes(hatch: &Hatch, loops: Vec<Vec<Point2>>, lineweight: f64) -> Vec<Shape> {
    if loops.is_empty() {
        return Vec::new();
    }
    if hatch.is_solid() {
        return vec![Shape::Fill {
            rule: FillRule::EvenOdd,
            contours: loops,
        }];
    }
    // A boundary point this close to a pattern line is on it.
    let near = loops
        .iter()
        .flatten()
        .fold(1.0, |size: f64, p| size.max(p[0].abs()).max(p[1].abs()))
        * 1e-9;
    let mut runs = Vec::new();
    let mut lines = 0;
    for family in &hatch.pattern.lines {
        let (sin, cos) = sin_cos_degrees(family.angle + hatch.angle);
        let (hs, hc) = sin_cos_degrees(hatch.angle);
        let scale = hatch.scale;
        // The family's origin turns with the hatch; its step and dashes
        // are measured along and across its own direction.
        let origin = [
            (family.origin[0] * hc - family.origin[1] * hs) * scale,
            (family.origin[0] * hs + family.origin[1] * hc) * scale,
        ];
        let along = family.delta[0] * scale;
        let across = family.delta[1] * scale;
        if across == 0.0 {
            continue;
        }
        let direction = [cos, sin];
        let normal = [-sin, cos];
        let step = [
            along * direction[0] + across * normal[0],
            along * direction[1] + across * normal[1],
        ];
        let dashes: Vec<f64> = family.dashes.iter().map(|d| d * scale).collect();
        // Which lines cross the boundary: those whose distance across from
        // the origin falls within the boundary's.
        let (low, high) = loops
            .iter()
            .flatten()
            .map(|p| (p[0] - origin[0]) * normal[0] + (p[1] - origin[1]) * normal[1])
            .fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(s), hi.max(s)));
        let (first, last) = ((low / across).floor(), (high / across).ceil());
        let (first, last) = (first.min(last), first.max(last));
        for k in first as i64..=last as i64 {
            if lines >= MOST_LINES {
                break;
            }
            lines += 1;
            let k = k as f64;
            let base = [origin[0] + k * step[0], origin[1] + k * step[1]];
            for (from, to) in inside(&loops, base, direction, normal, near) {
                if dashes.is_empty() {
                    runs.push(vec![at(base, direction, from), at(base, direction, to)]);
                } else {
                    dash(&mut runs, &dashes, base, direction, from, to);
                }
            }
        }
    }
    if runs.len() > MOST_DASHES {
        runs.truncate(MOST_DASHES);
    }
    if runs.is_empty() {
        Vec::new()
    } else {
        vec![Shape::Stroke { lineweight, runs }]
    }
}

/// The sine and cosine of an angle in degrees, exact at quarter turns, so
/// that a pattern line meant to run along an edge does.
fn sin_cos_degrees(degrees: f64) -> (f64, f64) {
    let turned = degrees.rem_euclid(360.0);
    [
        (0.0, (0.0, 1.0)),
        (90.0, (1.0, 0.0)),
        (180.0, (0.0, -1.0)),
        (270.0, (-1.0, 0.0)),
    ]
    .into_iter()
    .find(|&(quarter, _)| turned == quarter)
    .map_or_else(|| turned.to_radians().sin_cos(), |(_, exact)| exact)
}

/// A loop's points, in order, curves as chords.
fn boundary_loop(l: &HatchLoop, tolerance: f64) -> Vec<Point2> {
    match l {
        HatchLoop::Polyline { points } => points.iter().map(|p| [p.x, p.y]).collect(),
        HatchLoop::Edges(edges) => {
            let backwards = walk(edges).unwrap_or_else(|_| vec![false; edges.len()]);
            let mut points: Vec<Point2> = Vec::new();
            for (edge, &back) in edges.iter().zip(&backwards) {
                let mut part = match *edge {
                    HatchEdge::Line { from, to } => vec![[from.x, from.y], [to.x, to.y]],
                    HatchEdge::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                    } => curves::arc(
                        [center.x, center.y],
                        radius,
                        start_angle,
                        end_angle,
                        tolerance,
                    ),
                };
                if back {
                    part.reverse();
                }
                for p in part {
                    if points.last() != Some(&p) {
                        points.push(p);
                    }
                }
            }
            if points.len() > 1 && points.first() == points.last() {
                points.pop();
            }
            points
        }
    }
}

/// The point `t` along a pattern line from its base.
fn at(base: Point2, direction: Point2, t: f64) -> Point2 {
    [base[0] + t * direction[0], base[1] + t * direction[1]]
}

/// Where the line through `base` along `direction` is inside the hatch:
/// spans from and to, along the line from its base, inside an odd number of
/// loops. A line that runs along the boundary is not inside, as AutoCAD
/// draws no pattern line on a hatch's edge.
fn inside(
    loops: &[Vec<Point2>],
    base: Point2,
    direction: Point2,
    normal: Point2,
    near: f64,
) -> Vec<(f64, f64)> {
    // A point on the line counts as on one side, then as on the other: a
    // line through a vertex crosses the boundary once either way, and a
    // line along an edge is inside one way only.
    let above = spans(loops, base, direction, normal, near, false);
    let below = spans(loops, base, direction, normal, near, true);
    let mut both = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < above.len() && j < below.len() {
        let (from, to) = (above[i].0.max(below[j].0), above[i].1.min(below[j].1));
        if to - from > near {
            both.push((from, to));
        }
        if above[i].1 < below[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    both
}

/// The spans of the line inside an odd number of loops, a point within
/// `near` of the line counted as on the side of the normal, or as off it
/// when `on_counts_off`.
fn spans(
    loops: &[Vec<Point2>],
    base: Point2,
    direction: Point2,
    normal: Point2,
    near: f64,
    on_counts_off: bool,
) -> Vec<(f64, f64)> {
    let side = |s: f64| {
        if s.abs() <= near {
            !on_counts_off
        } else {
            s > 0.0
        }
    };
    let mut crossings = Vec::new();
    for l in loops {
        for i in 0..l.len() {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            let sa = (a[0] - base[0]) * normal[0] + (a[1] - base[1]) * normal[1];
            let sb = (b[0] - base[0]) * normal[0] + (b[1] - base[1]) * normal[1];
            if side(sa) != side(sb) {
                let f = if sa.abs() <= near {
                    0.0
                } else if sb.abs() <= near {
                    1.0
                } else {
                    sa / (sa - sb)
                };
                let p = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
                crossings.push((p[0] - base[0]) * direction[0] + (p[1] - base[1]) * direction[1]);
            }
        }
    }
    crossings.sort_by(f64::total_cmp);
    crossings
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[from, to]| (from, to))
        .filter(|(from, to)| to > from)
        .collect()
}

/// The dashes and dots of a pattern line between `from` and `to` along it,
/// the pattern counted from the line's base.
fn dash(
    runs: &mut Vec<Vec<Point2>>,
    dashes: &[f64],
    base: Point2,
    direction: Point2,
    from: f64,
    to: f64,
) {
    let repeat: f64 = dashes.iter().map(|d| d.abs()).sum();
    if repeat.is_nan() || repeat <= 0.0 || (to - from) / repeat > MOST_DASHES as f64 {
        runs.push(vec![at(base, direction, from), at(base, direction, to)]);
        return;
    }
    // Start at the repeat of the pattern that holds `from`.
    let mut position = (from / repeat).floor() * repeat;
    while position < to {
        for &d in dashes {
            let length = d.abs();
            if d > 0.0 {
                let (a, b) = (position.max(from), (position + length).min(to));
                if b > a {
                    runs.push(vec![at(base, direction, a), at(base, direction, b)]);
                } else if b == a && a < to {
                    // A dash that ends just where the line comes inside
                    // leaves a dot, as AutoCAD draws it.
                    runs.push(vec![at(base, direction, a)]);
                }
            } else if d == 0.0 && position >= from && position <= to {
                runs.push(vec![at(base, direction, position)]);
            }
            position += length;
            if position >= to && d != 0.0 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HatchPattern, HatchPatternLine, Point3, LAYER_ZERO};

    fn square(size: f64) -> HatchLoop {
        HatchLoop::Polyline {
            points: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(size, 0.0, 0.0),
                Point3::new(size, size, 0.0),
                Point3::new(0.0, size, 0.0),
            ],
        }
    }

    fn hatch(lines: Vec<HatchPatternLine>, boundary: Vec<HatchLoop>) -> Hatch {
        Hatch {
            pattern: HatchPattern {
                name: "MADE_UP".into(),
                lines,
            },
            scale: 1.0,
            angle: 0.0,
            boundary,
            layer: LAYER_ZERO.into(),
        }
    }

    #[test]
    fn pattern_lines_run_from_edge_to_edge_at_their_spacing() {
        let h = hatch(
            vec![HatchPatternLine {
                angle: 0.0,
                origin: [0.0, 0.5],
                delta: [0.0, 1.0],
                dashes: Vec::new(),
            }],
            vec![square(10.0)],
        );
        let (shapes, _) = draw(&h, 0.25, 0.01);
        let Shape::Stroke { runs, lineweight } = &shapes[0] else {
            panic!("{shapes:?}");
        };
        assert_eq!(*lineweight, 0.25);
        assert_eq!(runs.len(), 10);
        for (i, run) in runs.iter().enumerate() {
            assert_eq!(run, &vec![[0.0, i as f64 + 0.5], [10.0, i as f64 + 0.5]]);
        }
    }

    /// AutoCAD 2026 drew no pattern line along the edges of the example
    /// sheet's coursed panel, only those inside it.
    #[test]
    fn no_pattern_line_is_drawn_along_the_boundary() {
        let h = hatch(
            vec![HatchPatternLine {
                angle: 0.0,
                origin: [0.0, 0.0],
                delta: [0.0, 1.0],
                dashes: Vec::new(),
            }],
            vec![square(10.0)],
        );
        let (shapes, _) = draw(&h, 0.25, 0.01);
        let Shape::Stroke { runs, .. } = &shapes[0] else {
            panic!("{shapes:?}");
        };
        let heights: Vec<f64> = runs.iter().map(|r| r[0][1]).collect();
        assert_eq!(heights, (1..10).map(f64::from).collect::<Vec<_>>());
        // A line through a corner of a diamond still crosses it.
        let diamond = HatchLoop::Polyline {
            points: vec![
                Point3::new(0.0, -1.0, 0.0),
                Point3::new(1.0, 0.0, 0.0),
                Point3::new(0.0, 1.0, 0.0),
                Point3::new(-1.0, 0.0, 0.0),
            ],
        };
        let h = hatch(
            vec![HatchPatternLine {
                angle: 0.0,
                origin: [0.0, 0.0],
                delta: [0.0, 5.0],
                dashes: Vec::new(),
            }],
            vec![diamond],
        );
        let (shapes, _) = draw(&h, 0.25, 0.01);
        let Shape::Stroke { runs, .. } = &shapes[0] else {
            panic!("{shapes:?}");
        };
        assert_eq!(runs, &vec![vec![[-1.0, 0.0], [1.0, 0.0]]]);
    }

    #[test]
    fn a_hole_is_left_empty() {
        let hole = HatchLoop::Polyline {
            points: vec![
                Point3::new(4.0, 4.0, 0.0),
                Point3::new(6.0, 4.0, 0.0),
                Point3::new(6.0, 6.0, 0.0),
                Point3::new(4.0, 6.0, 0.0),
            ],
        };
        let h = hatch(
            vec![HatchPatternLine {
                angle: 0.0,
                origin: [0.0, 0.5],
                delta: [0.0, 1.0],
                dashes: Vec::new(),
            }],
            vec![square(10.0), hole],
        );
        let (shapes, _) = draw(&h, 0.25, 0.01);
        let Shape::Stroke { runs, .. } = &shapes[0] else {
            panic!("{shapes:?}");
        };
        let through: Vec<&Vec<Point2>> = runs.iter().filter(|r| r[0][1] == 5.5).collect();
        assert_eq!(
            through,
            vec![
                &vec![[0.0, 5.5], [4.0, 5.5]],
                &vec![[6.0, 5.5], [10.0, 5.5]]
            ]
        );
    }

    #[test]
    fn dashes_count_from_the_lines_origin() {
        let h = hatch(
            vec![HatchPatternLine {
                angle: 0.0,
                origin: [1.0, 0.5],
                delta: [0.0, 20.0],
                dashes: vec![2.0, -2.0, 0.0, -2.0],
            }],
            vec![square(10.0)],
        );
        let (shapes, _) = draw(&h, 0.25, 0.01);
        let Shape::Stroke { runs, .. } = &shapes[0] else {
            panic!("{shapes:?}");
        };
        let xs: Vec<Vec<f64>> = runs
            .iter()
            .map(|r| r.iter().map(|p| p[0]).collect())
            .collect();
        // From x 1 on: a dash of 2, a gap, a dot, a gap, and again; the dot
        // before x 1 falls at x -1, outside.
        assert_eq!(xs, vec![vec![1.0, 3.0], vec![5.0], vec![7.0, 9.0]]);
    }

    #[test]
    fn a_solid_hatch_is_one_fill_of_all_its_loops() {
        let mut h = hatch(Vec::new(), vec![square(10.0), square(5.0)]);
        h.pattern = HatchPattern::solid();
        let (shapes, _) = draw(&h, 0.25, 0.01);
        assert_eq!(shapes.len(), 1);
        let Shape::Fill { rule, contours } = &shapes[0] else {
            panic!("{shapes:?}");
        };
        assert_eq!(*rule, FillRule::EvenOdd);
        assert_eq!(contours.len(), 2);
    }
}
