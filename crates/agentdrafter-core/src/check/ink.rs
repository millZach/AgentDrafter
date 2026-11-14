//! Where two things a sheet draws touch as they print: a line's ink and a
//! text's letters.
//!
//! Each is the ink one object draws: its strokes, as segments with half
//! their pen's width on each side, and its filled areas, as closed
//! contours. A text's letters are an SHX font's strokes, or a TrueType
//! font's filled outlines; a line is its dashes as its linetype lays them
//! out, or a wide polyline's filled band.
//!
//! Two inks touch where they overlap by more than the tolerance: two pens
//! by how far their widths overlap, and a pen or an outline and a filled
//! area by how deep it reaches into the area.

use crate::draw::{Bounds, Point2, Shape};

/// The ink one object draws.
#[derive(Default)]
pub(super) struct Ink {
    /// Each stroke's segment, with half its pen's width in drawing units.
    /// A dot is a segment from a point to itself.
    pub strokes: Vec<(Point2, Point2, f64)>,
    /// Each filled area's closed contours, filled by the non-zero rule.
    pub fills: Vec<Vec<Point2>>,
    /// The box round all of it, its pens' widths among it.
    pub bounds: Option<Bounds>,
}

impl Ink {
    /// Adds `shape`, its pen's width turned into drawing units by
    /// `units_per_mm`.
    pub fn add(&mut self, shape: &Shape, units_per_mm: f64) {
        let mut points: Vec<Point2> = Vec::new();
        let mut reach = 0.0_f64;
        match shape {
            Shape::Stroke { lineweight, runs } => {
                let half = lineweight * units_per_mm / 2.0;
                reach = half;
                for run in runs {
                    match run.as_slice() {
                        [] => {}
                        [p] => self.strokes.push((*p, *p, half)),
                        _ => {
                            for pair in run.windows(2) {
                                self.strokes.push((pair[0], pair[1], half));
                            }
                        }
                    }
                    points.extend(run.iter().copied());
                }
            }
            Shape::Fill { contours, .. } => {
                for contour in contours.iter().filter(|c| c.len() >= 3) {
                    points.extend(contour.iter().copied());
                    self.fills.push(contour.clone());
                }
            }
        }
        if let Some(b) = Bounds::of(points.into_iter()) {
            let grown = [
                [b.min[0] - reach, b.min[1] - reach],
                [b.max[0] + reach, b.max[1] + reach],
            ];
            let had = self.bounds.into_iter().flat_map(|b| [b.min, b.max]);
            self.bounds = Bounds::of(had.chain(grown));
        }
    }

    /// The edges of the filled areas, as segments of no width.
    fn edges(&self) -> Vec<(Point2, Point2)> {
        self.fills
            .iter()
            .flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()])))
            .collect()
    }

    /// Where this ink and `other` overlap as they print by more than
    /// `tolerance`: a point inside both inks. None when they do not.
    pub fn touches(&self, other: &Ink, tolerance: f64) -> Option<Point2> {
        let (a, b) = (self.bounds?, other.bounds?);
        if apart(&a, &b, tolerance) {
            return None;
        }
        // Two pens: how far their widths overlap, or a crossing.
        for &(p, q, w) in &self.strokes {
            for &(r, s, v) in &other.strokes {
                if apart_segments(p, q, r, s, w + v + tolerance) {
                    continue;
                }
                if let Some(at) = cross(p, q, r, s) {
                    if w + v > tolerance || (w == 0.0 && v == 0.0) {
                        return Some(at);
                    }
                }
                let (d, on_first, on_second) = distance(p, q, r, s);
                if w + v - d > tolerance {
                    // Between the two centrelines, inside both pens.
                    let t = if w + v > 0.0 { w / (w + v) } else { 0.5 };
                    return Some([
                        on_first[0] + (on_second[0] - on_first[0]) * t,
                        on_first[1] + (on_second[1] - on_first[1]) * t,
                    ]);
                }
            }
        }
        // A pen, or a filled area's outline, reaching into the other's
        // filled area.
        for (inner, outer) in [(self, other), (other, self)] {
            if outer.fills.is_empty() {
                continue;
            }
            let edges = outer.edges();
            let Some(area) = Bounds::of(outer.fills.iter().flatten().copied()) else {
                continue;
            };
            let mut reaching: Vec<(Point2, Point2, f64)> = inner.strokes.clone();
            reaching.extend(inner.edges().into_iter().map(|(p, q)| (p, q, 0.0)));
            for (p, q, w) in reaching {
                if apart_segments(p, q, area.min, area.max, w + tolerance) {
                    continue;
                }
                if let Some(at) = into_fill(p, q, w, &outer.fills, &edges, tolerance) {
                    return Some(at);
                }
            }
        }
        None
    }
}

/// Where a pen `w` wide each side of segment `p`–`q` reaches into the area
/// `fills`, bounded by `edges`, by more than `tolerance`: a point inside
/// both. The segment is cut where it crosses the edges; each piece whose
/// middle is inside reaches as deep as that middle is from the edges, plus
/// the pen. A segment outside the area reaches in by its pen less its
/// distance from the edges.
fn into_fill(
    p: Point2,
    q: Point2,
    w: f64,
    fills: &[Vec<Point2>],
    edges: &[(Point2, Point2)],
    tolerance: f64,
) -> Option<Point2> {
    let mut cuts = vec![0.0, 1.0];
    for &(r, s) in edges {
        if let Some(t) = cross_at(p, q, r, s) {
            cuts.push(t);
        }
    }
    cuts.sort_by(f64::total_cmp);
    let at = |t: f64| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
    let depth = |m: Point2| {
        edges
            .iter()
            .map(|&(r, s)| {
                let n = nearest(m, r, s);
                (m[0] - n[0]).hypot(m[1] - n[1])
            })
            .fold(f64::INFINITY, f64::min)
    };
    for pair in cuts.windows(2) {
        let m = at((pair[0] + pair[1]) / 2.0);
        if inside(m, fills) && depth(m) + w > tolerance {
            return Some(m);
        }
    }
    // Outside the area, the pen may still reach over its edge.
    if w > tolerance {
        for &(r, s) in edges {
            let (d, on_segment, on_edge) = distance(p, q, r, s);
            if w - d > tolerance {
                let t = if w > 0.0 { (d / w).min(1.0) } else { 1.0 };
                let toward = [
                    on_segment[0] + (on_edge[0] - on_segment[0]) * t,
                    on_segment[1] + (on_edge[1] - on_segment[1]) * t,
                ];
                return Some(toward);
            }
        }
    }
    None
}

/// Whether two boxes are apart by more than `margin`.
fn apart(a: &Bounds, b: &Bounds, margin: f64) -> bool {
    a.min[0] > b.max[0] + margin
        || b.min[0] > a.max[0] + margin
        || a.min[1] > b.max[1] + margin
        || b.min[1] > a.max[1] + margin
}

/// Whether segments `p`–`q` and `r`–`s` are apart by more than `margin`,
/// box against box.
fn apart_segments(p: Point2, q: Point2, r: Point2, s: Point2, margin: f64) -> bool {
    p[0].min(q[0]) > r[0].max(s[0]) + margin
        || r[0].min(s[0]) > p[0].max(q[0]) + margin
        || p[1].min(q[1]) > r[1].max(s[1]) + margin
        || r[1].min(s[1]) > p[1].max(q[1]) + margin
}

/// Where along segment `p`–`q`, from 0 to 1, it crosses segment `r`–`s`.
fn cross_at(p: Point2, q: Point2, r: Point2, s: Point2) -> Option<f64> {
    let d1 = [q[0] - p[0], q[1] - p[1]];
    let d2 = [s[0] - r[0], s[1] - r[1]];
    let denom = d1[0] * d2[1] - d1[1] * d2[0];
    if denom == 0.0 {
        return None;
    }
    let w = [r[0] - p[0], r[1] - p[1]];
    let t = (w[0] * d2[1] - w[1] * d2[0]) / denom;
    let u = (w[0] * d1[1] - w[1] * d1[0]) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

/// Where segment `p`–`q` crosses segment `r`–`s`, ends included; none when
/// they do not, or lie along one line.
fn cross(p: Point2, q: Point2, r: Point2, s: Point2) -> Option<Point2> {
    cross_at(p, q, r, s).map(|t| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t])
}

/// The least distance between segment `p`–`q` and segment `r`–`s`, and the
/// nearest points on each.
fn distance(p: Point2, q: Point2, r: Point2, s: Point2) -> (f64, Point2, Point2) {
    let mut best = (f64::INFINITY, p, r);
    for (point, a, b, first) in [
        (p, r, s, true),
        (q, r, s, true),
        (r, p, q, false),
        (s, p, q, false),
    ] {
        let near = nearest(point, a, b);
        let d = (point[0] - near[0]).hypot(point[1] - near[1]);
        if d < best.0 {
            best = if first {
                (d, point, near)
            } else {
                (d, near, point)
            };
        }
    }
    best
}

/// The point of segment `a`–`b` nearest `p`.
fn nearest(p: Point2, a: Point2, b: Point2) -> Point2 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    if length == 0.0 {
        return a;
    }
    let t = (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length).clamp(0.0, 1.0);
    [a[0] + t * ab[0], a[1] + t * ab[1]]
}

/// Whether `p` is inside `contours`, filled by the non-zero rule.
fn inside(p: Point2, contours: &[Vec<Point2>]) -> bool {
    let mut winding = 0;
    for contour in contours {
        for i in 0..contour.len() {
            let (a, b) = (contour[i], contour[(i + 1) % contour.len()]);
            let side = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
            if a[1] <= p[1] {
                if b[1] > p[1] && side > 0.0 {
                    winding += 1;
                }
            } else if b[1] <= p[1] && side < 0.0 {
                winding -= 1;
            }
        }
    }
    winding != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::FillRule;

    fn stroke(runs: Vec<Vec<Point2>>, lineweight: f64) -> Ink {
        let mut ink = Ink::default();
        ink.add(&Shape::Stroke { lineweight, runs }, 1.0);
        ink
    }

    fn filled(contour: Vec<Point2>) -> Ink {
        let mut ink = Ink::default();
        ink.add(
            &Shape::Fill {
                rule: FillRule::NonZero,
                contours: vec![contour],
            },
            1.0,
        );
        ink
    }

    #[test]
    fn crossing_strokes_touch_and_apart_ones_do_not() {
        let line = stroke(vec![vec![[0.0, 0.0], [10.0, 0.0]]], 0.0);
        let letter = stroke(vec![vec![[5.0, -1.0], [5.0, 1.0]]], 0.0);
        assert_eq!(line.touches(&letter, 0.01), Some([5.0, 0.0]));
        let above = stroke(vec![vec![[5.0, 0.5], [5.0, 1.0]]], 0.0);
        assert_eq!(line.touches(&above, 0.01), None);
        // Pens 0.6 wide each, 0.5 apart: their ink overlaps by 0.1.
        let wide_line = stroke(vec![vec![[0.0, 0.0], [10.0, 0.0]]], 0.6);
        let wide_above = stroke(vec![vec![[5.0, 0.5], [5.0, 1.0]]], 0.6);
        assert!(wide_line.touches(&wide_above, 0.01).is_some());
        assert!(wide_line.touches(&wide_above, 0.2).is_none());
    }

    #[test]
    fn where_two_pens_touch_is_inside_both() {
        let line = stroke(vec![vec![[0.0, 0.0], [10.0, 0.0]]], 0.25);
        let letter = stroke(vec![vec![[2.0, 0.2], [8.0, 0.2]]], 0.25);
        let at = line.touches(&letter, 0.01).unwrap();
        assert!(
            at[1] <= 0.125 + 1e-12 && at[1] >= 0.2 - 0.125 - 1e-12,
            "{at:?}"
        );
    }

    #[test]
    fn a_stroke_inside_a_filled_letter_touches_it_and_a_brush_does_not() {
        let stem = filled(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 10.0], [0.0, 10.0]]);
        let inner = stroke(vec![vec![[0.2, 2.0], [0.8, 2.0]]], 0.0);
        assert!(inner.touches(&stem, 0.01).is_some());
        let outside = stroke(vec![vec![[5.0, 2.0], [6.0, 2.0]]], 0.0);
        assert!(outside.touches(&stem, 0.01).is_none());
        // A 1 wide pen whose edge reaches 0.005 into the stem: within the
        // tolerance, so it does not touch; one reaching 0.1 in does.
        let brush = stroke(vec![vec![[1.495, 4.5], [2.0, 4.5]]], 1.0);
        assert!(brush.touches(&stem, 0.01).is_none());
        let into = stroke(vec![vec![[1.4, 4.5], [2.0, 4.5]]], 1.0);
        assert!(into.touches(&stem, 0.01).is_some());
        // A hairline running along the stem's edge does not reach into it.
        let along = stroke(vec![vec![[1.0, 2.0], [1.0, 8.0]]], 0.0);
        assert!(along.touches(&stem, 0.01).is_none());
        // A hairline across the stem reaches half its width in.
        let across = stroke(vec![vec![[-1.0, 5.0], [2.0, 5.0]]], 0.0);
        let at = across.touches(&stem, 0.01).unwrap();
        assert!((0.0..=1.0).contains(&at[0]), "{at:?}");
    }
}
