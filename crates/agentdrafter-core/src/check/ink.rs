//! Where two things a sheet draws touch as they print: a line's ink and a
//! text's letters.
//!
//! Each is the ink one object draws: its strokes, as segments with half
//! their pen's width on each side, and its filled areas, as closed
//! contours. A text's letters are an SHX font's strokes, or a TrueType
//! font's filled outlines; a line is its dashes as its linetype lays them
//! out, or a wide polyline's filled band.

use crate::draw::{Bounds, Point2, Shape};

/// The ink one object draws.
#[derive(Default)]
pub(super) struct Ink {
    /// Each stroke's segment, with half its pen's width in drawing units.
    /// A dot is a segment from a point to itself.
    pub segments: Vec<(Point2, Point2, f64)>,
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
                        [p] => self.segments.push((*p, *p, half)),
                        _ => {
                            for pair in run.windows(2) {
                                self.segments.push((pair[0], pair[1], half));
                            }
                        }
                    }
                    points.extend(run.iter().copied());
                }
            }
            Shape::Fill { contours, .. } => {
                for contour in contours {
                    if contour.len() < 2 {
                        continue;
                    }
                    for i in 0..contour.len() {
                        let next = contour[(i + 1) % contour.len()];
                        self.segments.push((contour[i], next, 0.0));
                    }
                    points.extend(contour.iter().copied());
                    self.fills.push(contour.clone());
                }
            }
        }
        if let Some(b) = Bounds::of(points.into_iter()) {
            let grown = Bounds {
                min: [b.min[0] - reach, b.min[1] - reach],
                max: [b.max[0] + reach, b.max[1] + reach],
            };
            let had = self.bounds.into_iter().flat_map(|b| [b.min, b.max]);
            self.bounds = Bounds::of(had.chain([grown.min, grown.max]));
        }
    }

    /// Where this ink and `other` touch as they print, by more than
    /// `tolerance`: a point where they cross, or where their pens overlap
    /// by more than it, or a point of one inside a filled area of the
    /// other. None when they do not.
    pub fn touches(&self, other: &Ink, tolerance: f64) -> Option<Point2> {
        let (a, b) = (self.bounds?, other.bounds?);
        if a.min[0] > b.max[0] + tolerance
            || b.min[0] > a.max[0] + tolerance
            || a.min[1] > b.max[1] + tolerance
            || b.min[1] > a.max[1] + tolerance
        {
            return None;
        }
        for &(p, q, w) in &self.segments {
            let seg = Bounds::of([p, q].into_iter()).expect("two points");
            for &(r, s, v) in &other.segments {
                let reach = w + v + tolerance;
                if seg.min[0] > r[0].max(s[0]) + reach
                    || r[0].min(s[0]) > seg.max[0] + reach
                    || seg.min[1] > r[1].max(s[1]) + reach
                    || r[1].min(s[1]) > seg.max[1] + reach
                {
                    continue;
                }
                if let Some(at) = cross(p, q, r, s) {
                    return Some(at);
                }
                let (d, at) = distance(p, q, r, s);
                if w + v - d > tolerance {
                    return Some(at);
                }
            }
        }
        // A stroke wholly inside the other's filled area, or the reverse.
        for (inner, outer) in [(self, other), (other, self)] {
            for &(p, _, _) in &inner.segments {
                if !outer.fills.is_empty() && inside(p, &outer.fills) {
                    return Some(p);
                }
            }
        }
        None
    }
}

/// Where segment `p`–`q` crosses segment `r`–`s`, ends included; none when
/// they do not, or lie along one line.
fn cross(p: Point2, q: Point2, r: Point2, s: Point2) -> Option<Point2> {
    let d1 = [q[0] - p[0], q[1] - p[1]];
    let d2 = [s[0] - r[0], s[1] - r[1]];
    let denom = d1[0] * d2[1] - d1[1] * d2[0];
    if denom == 0.0 {
        return None;
    }
    let w = [r[0] - p[0], r[1] - p[1]];
    let t = (w[0] * d2[1] - w[1] * d2[0]) / denom;
    let u = (w[0] * d1[1] - w[1] * d1[0]) / denom;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
        .then(|| [p[0] + t * d1[0], p[1] + t * d1[1]])
}

/// The least distance between segment `p`–`q` and segment `r`–`s`, and
/// the point on the first where it is.
fn distance(p: Point2, q: Point2, r: Point2, s: Point2) -> (f64, Point2) {
    let mut best = (f64::INFINITY, p);
    for (point, a, b, own) in [
        (p, r, s, true),
        (q, r, s, true),
        (r, p, q, false),
        (s, p, q, false),
    ] {
        let near = nearest(point, a, b);
        let d = (point[0] - near[0]).hypot(point[1] - near[1]);
        if d < best.0 {
            best = (d, if own { point } else { near });
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

    fn stroke(runs: Vec<Vec<Point2>>, lineweight: f64) -> Ink {
        let mut ink = Ink::default();
        ink.add(&Shape::Stroke { lineweight, runs }, 1.0);
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
    fn a_stroke_inside_a_filled_letter_touches_it() {
        let mut letter = Ink::default();
        letter.add(
            &Shape::Fill {
                rule: crate::draw::FillRule::NonZero,
                contours: vec![vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]],
            },
            1.0,
        );
        let inner = stroke(vec![vec![[1.0, 2.0], [3.0, 2.0]]], 0.0);
        assert!(inner.touches(&letter, 0.01).is_some());
        let outside = stroke(vec![vec![[5.0, 2.0], [6.0, 2.0]]], 0.0);
        assert!(outside.touches(&letter, 0.01).is_none());
    }
}
