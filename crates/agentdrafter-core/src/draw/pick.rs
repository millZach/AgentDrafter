//! Picking: which of a sheet's objects the reviewer means by a point, found
//! in the display list the viewer draws, so what is picked is what is
//! drawn there, and the page holds no second hit-test of its own.
//!
//! Each item of the display list names the object of the sheet that draws
//! it, an insert for everything its block draws. A stroke is reached within
//! `reach` of its pen's edge, and a fill anywhere inside it, by its fill
//! rule, or within `reach` of its edge. The object drawn nearest the point
//! is picked; of two as near, the one drawn later, which lies on top.
//!
//! What model space draws through a viewport is cut at the viewport's
//! window, or at the boundary it is clipped to, as every output cuts its
//! ink there: a point outside it is no nearer to the item than to the
//! window or the boundary, so a fill cut away there is not picked.

use super::clip::{distance_to_outline, inside_outline};
use super::{Bounds, DisplayList, FillRule, Item, Point2, Shape};

/// The place in the sheet's objects of the object drawn nearest `at`,
/// within `reach` drawing units, or none when nothing is drawn that near.
/// `millimetres_per_unit` is how many millimetres of paper one drawing
/// unit spans, which sets how wide each pen is in drawing units.
pub fn pick(
    list: &DisplayList,
    at: Point2,
    reach: f64,
    millimetres_per_unit: f64,
) -> Option<usize> {
    let mut best: Option<(f64, usize)> = None;
    for item in &list.items {
        let distance = match &item.shape {
            Shape::Stroke { lineweight, runs } => {
                let half = lineweight / millimetres_per_unit.max(f64::MIN_POSITIVE) / 2.0;
                let nearest = runs
                    .iter()
                    .map(|run| to_run(at, run))
                    .fold(f64::INFINITY, f64::min);
                (nearest - half).max(0.0)
            }
            Shape::Fill { rule, contours } => match item.clip_boundary {
                // A fill seen through a clipped viewport is cut at its box
                // alone; what shows of it is what lies inside the boundary.
                Some(b) if b < list.clip_boundaries.len() => to_visible_fill(
                    at,
                    *rule,
                    contours,
                    &list.clip_boundaries[b],
                    millimetres_per_unit,
                ),
                _ => to_fill(at, *rule, contours),
            },
            Shape::Mask { contours } => match item.clip_boundary {
                Some(b) if b < list.clip_boundaries.len() => to_visible_fill(
                    at,
                    FillRule::EvenOdd,
                    contours,
                    &list.clip_boundaries[b],
                    millimetres_per_unit,
                ),
                _ => to_fill(at, FillRule::EvenOdd, contours),
            },
            Shape::Image {
                origin,
                u_axis,
                v_axis,
                clip,
                clip_inverted,
                ..
            } => {
                let contours = vec![Shape::image_outline(
                    *origin,
                    *u_axis,
                    *v_axis,
                    clip.as_deref(),
                    *clip_inverted,
                )];
                match item.clip_boundary {
                    Some(b) if b < list.clip_boundaries.len() => to_visible_fill(
                        at,
                        FillRule::EvenOdd,
                        &contours,
                        &list.clip_boundaries[b],
                        millimetres_per_unit,
                    ),
                    _ => to_fill(at, FillRule::EvenOdd, &contours),
                }
            }
        };
        let distance = distance.max(outside_cut(list, item, at));
        // Later items lie on top, so they win a tie.
        if distance <= reach && best.is_none_or(|(d, _)| distance <= d) {
            best = Some((distance, item.object));
        }
    }
    best.map(|(_, object)| object)
}

/// How far `at` lies from the fill of `contours` by `rule`: 0 inside it.
fn to_fill(at: Point2, rule: FillRule, contours: &[Vec<Point2>]) -> f64 {
    if inside(at, rule, contours) {
        0.0
    } else {
        contours
            .iter()
            .map(|c| to_ring(at, c))
            .fold(f64::INFINITY, f64::min)
    }
}

/// How far off a piece of edge its sides are sampled, in millimetres of
/// paper: a tenth of a micrometre.
const SIDE_MM: f64 = 1e-4;

/// How far `at` lies from what shows of the fill of `contours` by `rule`
/// through the closed outline `outline`, a clipped viewport's boundary, in
/// a drawing whose unit spans `millimetres_per_unit` millimetres of paper.
///
/// One rule: a point is visible-filled when it lies inside the outline, or
/// on it, and the rule fills it. A point visible-filled is on the fill.
/// Otherwise the distance is to the nearest pickable piece of edge: a
/// piece of a contour or of the outline, each cut where it crosses any of
/// them, whose two sides, sampled just off its middle, differ in being
/// visible-filled. A seam, or contours that cancel by the rule, have both
/// sides alike and are not pickable; a fill's edge on the outline is, once
/// filled inside and not shown outside. When no piece is pickable, nothing
/// of the fill shows and it is out of reach.
fn to_visible_fill(
    at: Point2,
    rule: FillRule,
    contours: &[Vec<Point2>],
    outline: &[Point2],
    millimetres_per_unit: f64,
) -> f64 {
    let side = SIDE_MM / millimetres_per_unit.max(f64::MIN_POSITIVE);
    // On the outline is inside it, within a hundredth of the side's step.
    let visible_filled = |p: Point2| {
        (inside_outline(p, outline) || distance_to_outline(p, outline) <= side / 100.0)
            && inside(p, rule, contours)
    };
    if visible_filled(at) {
        return 0.0;
    }
    let mut rings: Vec<&[Point2]> = contours.iter().map(Vec::as_slice).collect();
    rings.push(outline);
    let mut nearest = f64::INFINITY;
    for ring in &rings {
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            for (p, q) in pieces(a, b, &rings) {
                let d = [q[0] - p[0], q[1] - p[1]];
                let length = d[0].hypot(d[1]);
                if length == 0.0 {
                    continue;
                }
                let middle = [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0];
                let off = [-d[1] / length * side, d[0] / length * side];
                let left = visible_filled([middle[0] + off[0], middle[1] + off[1]]);
                let right = visible_filled([middle[0] - off[0], middle[1] - off[1]]);
                if left != right {
                    nearest = nearest.min(to_segment(at, p, q));
                }
            }
        }
    }
    nearest
}

/// The segment from `a` to `b` cut where it meets an edge of any of the
/// closed `rings`, in order along it.
fn pieces(a: Point2, b: Point2, rings: &[&[Point2]]) -> Vec<(Point2, Point2)> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let mut cuts = vec![0.0, 1.0];
    for ring in rings {
        for j in 0..ring.len() {
            let (c, d) = (ring[j], ring[(j + 1) % ring.len()]);
            let s = [d[0] - c[0], d[1] - c[1]];
            let denominator = r[0] * s[1] - r[1] * s[0];
            if denominator == 0.0 {
                continue;
            }
            let q = [c[0] - a[0], c[1] - a[1]];
            let t = (q[0] * s[1] - q[1] * s[0]) / denominator;
            let u = (q[0] * r[1] - q[1] * r[0]) / denominator;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                cuts.push(t);
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let at = |t: f64| [a[0] + t * r[0], a[1] + t * r[1]];
    cuts.windows(2).map(|w| (at(w[0]), at(w[1]))).collect()
}

/// How far `at` lies outside what `item` is cut at, the window of the
/// viewport it shows through or the boundary that viewport is clipped to:
/// 0 inside it, or for an item drawn whole.
fn outside_cut(list: &DisplayList, item: &Item, at: Point2) -> f64 {
    if let Some(outline) = item.clip_boundary.and_then(|b| list.clip_boundaries.get(b)) {
        return if inside_outline(at, outline) {
            0.0
        } else {
            distance_to_outline(at, outline)
        };
    }
    item.clip.map_or(0.0, |b| {
        let dx = (b.min[0] - at[0]).max(at[0] - b.max[0]).max(0.0);
        let dy = (b.min[1] - at[1]).max(at[1] - b.max[1]).max(0.0);
        dx.hypot(dy)
    })
}

/// The box round everything the sheet's object at `object` draws, or none
/// when it draws nothing.
pub fn object_bounds(list: &DisplayList, object: usize) -> Option<Bounds> {
    Bounds::of(
        list.items
            .iter()
            .filter(|item| item.object == object)
            .flat_map(|item| match &item.shape {
                Shape::Stroke { runs, .. } => runs.iter().flatten().copied().collect::<Vec<_>>(),
                Shape::Fill { contours, .. } | Shape::Mask { contours } => {
                    contours.iter().flatten().copied().collect::<Vec<_>>()
                }
                Shape::Image {
                    origin,
                    u_axis,
                    v_axis,
                    clip,
                    clip_inverted,
                    ..
                } => {
                    Shape::image_outline(*origin, *u_axis, *v_axis, clip.as_deref(), *clip_inverted)
                }
            }),
    )
}

/// How far `p` is from the polyline `run`; a run of one point is a dot.
fn to_run(p: Point2, run: &[Point2]) -> f64 {
    match run {
        [] => f64::INFINITY,
        [dot] => distance(p, *dot),
        points => points
            .windows(2)
            .map(|pair| to_segment(p, pair[0], pair[1]))
            .fold(f64::INFINITY, f64::min),
    }
}

/// How far `p` is from the closed contour `ring`.
fn to_ring(p: Point2, ring: &[Point2]) -> f64 {
    let Some(&last) = ring.last() else {
        return f64::INFINITY;
    };
    let mut previous = last;
    let mut nearest = f64::INFINITY;
    for &point in ring {
        nearest = nearest.min(to_segment(p, previous, point));
        previous = point;
    }
    nearest
}

/// The outline of a fill whose edges the nearest snap finds a point on,
/// and the place among the display list's items of the fill it is of.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub item: usize,
    /// The pieces of its contours' edges with the fill on one side only.
    pub pieces: Vec<(Point2, Point2)>,
}

/// The outlines of the fills `edges` names by their places among `list`'s
/// items, in order, as the display list's `edges` gives them: a polyline's
/// with width, a solid's and a donut's. Worked out once a sheet is drawn,
/// for [`nearest`] to look along.
pub fn outlines(list: &DisplayList, edges: &[usize]) -> Vec<Outline> {
    edges
        .iter()
        .filter_map(|&item| match &list.items.get(item)?.shape {
            Shape::Fill { rule, contours } => Some(Outline {
                item,
                pieces: fill_outline(*rule, contours),
            }),
            Shape::Mask { contours } => Some(Outline {
                item,
                pieces: fill_outline(FillRule::EvenOdd, contours),
            }),
            Shape::Stroke { .. } | Shape::Image { .. } => None,
        })
        .collect()
}

/// The outer outline of a fill: the pieces of its contours' edges that
/// have the fill on one side and not the other. A polyline with width is
/// drawn as a contour for each segment, and where two meet, their shared
/// end, a seam, has the fill on both sides: it is no edge of what is drawn,
/// and neither is an edge inside another contour.
pub(crate) fn fill_outline(rule: FillRule, contours: &[Vec<Point2>]) -> Vec<(Point2, Point2)> {
    let edges: Vec<(Point2, Point2)> = contours
        .iter()
        .filter(|ring| ring.len() >= 2)
        .flat_map(|ring| (0..ring.len()).map(move |i| (ring[i], ring[(i + 1) % ring.len()])))
        .filter(|(a, b)| a != b)
        .collect();
    let Some(bounds) = Bounds::of(edges.iter().flat_map(|(a, b)| [*a, *b])) else {
        return Vec::new();
    };
    let size = (bounds.max[0] - bounds.min[0])
        .max(bounds.max[1] - bounds.min[1])
        .max(f64::MIN_POSITIVE);
    // How near a point is to lie on an edge, and how far either side of an
    // edge the fill is looked for.
    let on = size * 1e-9;
    let side = size * 1e-6;
    let mut out = Vec::new();
    for (n, &(a, b)) in edges.iter().enumerate() {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = dx.hypot(dy);
        // Where other edges cross or touch it cut it into pieces, each
        // wholly an edge of the fill or not.
        let mut cuts = vec![0.0, 1.0];
        for (m, &(c, d)) in edges.iter().enumerate() {
            if m == n
                || c[0].max(d[0]) < a[0].min(b[0]) - on
                || c[0].min(d[0]) > a[0].max(b[0]) + on
                || c[1].max(d[1]) < a[1].min(b[1]) - on
                || c[1].min(d[1]) > a[1].max(b[1]) + on
            {
                continue;
            }
            for p in [c, d] {
                if to_segment(p, a, b) <= on {
                    cuts.push(((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (length * length));
                }
            }
            let (ex, ey) = (d[0] - c[0], d[1] - c[1]);
            let across = dx * ey - dy * ex;
            if across.abs() > f64::EPSILON * length * ex.hypot(ey) {
                let t = ((c[0] - a[0]) * ey - (c[1] - a[1]) * ex) / across;
                let u = ((c[0] - a[0]) * dy - (c[1] - a[1]) * dx) / across;
                if (0.0..=1.0).contains(&u) {
                    cuts.push(t);
                }
            }
        }
        cuts.retain(|t| (0.0..=1.0).contains(t));
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|x, y| (*x - *y).abs() <= 1e-12);
        let normal = [-dy / length * side, dx / length * side];
        let at = |t: f64| [a[0] + t * dx, a[1] + t * dy];
        for pair in cuts.windows(2) {
            let mid = at((pair[0] + pair[1]) / 2.0);
            let left = inside([mid[0] + normal[0], mid[1] + normal[1]], rule, contours);
            let right = inside([mid[0] - normal[0], mid[1] - normal[1]], rule, contours);
            if left != right {
                out.push((at(pair[0]), at(pair[1])));
            }
        }
    }
    out
}

/// The point of a stroke drawn on the page itself nearest `at`, within
/// `reach` drawing units, and the place in the sheet's objects of the
/// object that draws it: AutoCAD's nearest snap as the cursor finds it,
/// on what the display list draws, its curves within its tolerance. Of the
/// fills, only those with one of `outlines`, in the order of their items,
/// are looked at, along their outlines, not the seams inside them. Model
/// space seen through a viewport is not. Of two as near, the one drawn
/// later, which lies on top.
pub fn nearest(
    list: &DisplayList,
    outlines: &[Outline],
    at: Point2,
    reach: f64,
) -> Option<(usize, Point2)> {
    let mut best: Option<(f64, usize, Point2)> = None;
    for (k, item) in list.items.iter().enumerate() {
        if item.viewport.is_some() {
            continue;
        }
        let pieces: Vec<(Point2, Point2)> = match &item.shape {
            Shape::Stroke { runs, .. } => runs
                .iter()
                .flat_map(|run| match run.as_slice() {
                    [] => Vec::new(),
                    [dot] => vec![(*dot, *dot)],
                    points => points.windows(2).map(|w| (w[0], w[1])).collect(),
                })
                .collect(),
            Shape::Fill { .. } | Shape::Mask { .. } | Shape::Image { .. } => {
                match outlines.binary_search_by_key(&k, |o| o.item) {
                    Ok(o) => outlines[o].pieces.clone(),
                    Err(_) => Vec::new(),
                }
            }
        };
        for (a, b) in pieces {
            let p = foot(at, a, b);
            let d = distance(at, p);
            if d <= reach && best.is_none_or(|(e, _, _)| d <= e) {
                best = Some((d, item.object, p));
            }
        }
    }
    best.map(|(_, object, p)| (object, p))
}

/// The point of the segment from `a` to `b` nearest `p`.
fn foot(p: Point2, a: Point2, b: Point2) -> Point2 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    if length2 <= 0.0 {
        return a;
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0.0, 1.0);
    [a[0] + t * dx, a[1] + t * dy]
}

fn to_segment(p: Point2, a: Point2, b: Point2) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    if length2 <= 0.0 {
        return distance(p, a);
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0.0, 1.0);
    distance(p, [a[0] + t * dx, a[1] + t * dy])
}

fn distance(a: Point2, b: Point2) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Whether `p` is inside the contours by `rule`: inside an odd number of
/// them, or where they wind round it other than zero times.
pub(crate) fn inside(p: Point2, rule: FillRule, contours: &[Vec<Point2>]) -> bool {
    let mut winding = 0i32;
    let mut crossings = 0u32;
    for ring in contours.iter().filter(|r| r.len() >= 3) {
        let mut previous = ring[ring.len() - 1];
        for &point in ring {
            let (a, b) = (previous, point);
            if (a[1] <= p[1]) != (b[1] <= p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    crossings += 1;
                    winding += if b[1] > a[1] { 1 } else { -1 };
                }
            }
            previous = point;
        }
    }
    match rule {
        FillRule::EvenOdd => crossings % 2 == 1,
        FillRule::NonZero => winding != 0,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_nearest_point_is_on_the_stroke_drawn_nearest() {
        let list = DisplayList {
            items: vec![
                stroke(0, &[[0.0, 0.0], [10.0, 0.0]], 0.25),
                stroke(1, &[[0.0, 2.0], [10.0, 2.0]], 0.25),
            ],
            ..DisplayList::default()
        };
        assert_eq!(nearest(&list, &[], [4.0, 0.5], 1.0), Some((0, [4.0, 0.0])));
        assert_eq!(nearest(&list, &[], [4.0, 1.6], 1.0), Some((1, [4.0, 2.0])));
        // Past the end, the end; out of reach, none.
        assert_eq!(
            nearest(&list, &[], [11.0, 0.0], 2.0),
            Some((0, [10.0, 0.0]))
        );
        assert_eq!(nearest(&list, &[], [4.0, 5.0], 1.0), None);
    }

    #[test]
    fn the_nearest_point_is_on_the_edge_of_a_fill_a_line_draws() {
        // A polyline with width draws only a fill: its edge is found where
        // the display list's `edges` names it, and a fill it does not name,
        // text's or a hatch's, is passed over.
        let list = DisplayList {
            items: vec![square(0, [0.0, 0.0], 2.0), square(1, [10.0, 0.0], 2.0)],
            ..DisplayList::default()
        };
        let named = outlines(&list, &[0]);
        assert_eq!(
            nearest(&list, &named, [1.0, 2.5], 1.0),
            Some((0, [1.0, 2.0]))
        );
        // The closing edge, from the last point back to the first.
        assert_eq!(
            nearest(&list, &named, [-0.5, 1.0], 1.0),
            Some((0, [0.0, 1.0]))
        );
        assert_eq!(nearest(&list, &named, [11.0, 2.5], 1.0), None);
        assert_eq!(nearest(&list, &[], [1.0, 2.5], 1.0), None);
    }

    #[test]
    fn the_nearest_point_is_on_a_fills_outline_not_the_seams_inside_it() {
        // Two segments of a polyline 4 wide, (0, 0) to (10, 0) to (20, 0),
        // a contour each, meeting at x = 10: their shared end is inside
        // what is drawn.
        let quad = |x0: f64, x1: f64| vec![[x0, -2.0], [x1, -2.0], [x1, 2.0], [x0, 2.0]];
        let list = DisplayList {
            items: vec![Item {
                shape: Shape::Fill {
                    rule: FillRule::NonZero,
                    contours: vec![quad(0.0, 10.0), quad(10.0, 20.0)],
                },
                ..square(0, [0.0, 0.0], 1.0)
            }],
            ..DisplayList::default()
        };
        let named = outlines(&list, &[0]);
        // Inside, by the seam: the outline is 1.5 away, out of reach.
        assert_eq!(nearest(&list, &named, [10.0, 0.5], 1.0), None);
        // By the outline, the nearest point on it.
        assert_eq!(
            nearest(&list, &named, [10.0, 1.5], 1.0),
            Some((0, [10.0, 2.0]))
        );
        assert_eq!(
            nearest(&list, &named, [20.5, 0.0], 1.0),
            Some((0, [20.0, 0.0]))
        );
        // The seam is no piece of the outline; its ends' sides are.
        assert!(named[0]
            .pieces
            .iter()
            .all(|(a, b)| !(a[0] == 10.0 && b[0] == 10.0)));
    }

    use super::super::{Color, Item};
    use super::*;

    fn stroke(object: usize, points: &[Point2], lineweight: f64) -> Item {
        Item {
            object,
            part: None,
            nested: Vec::new(),
            cell: None,
            viewport: None,
            clip: None,
            clip_boundary: None,
            color: Color(7),
            true_color: None,
            plot_style: None,
            transparency: 0,
            shape: Shape::Stroke {
                lineweight,
                runs: vec![points.to_vec()],
            },
        }
    }

    fn square(object: usize, at: Point2, side: f64) -> Item {
        let [x, y] = at;
        Item {
            object,
            part: None,
            nested: Vec::new(),
            cell: None,
            viewport: None,
            clip: None,
            clip_boundary: None,
            color: Color(1),
            true_color: None,
            plot_style: None,
            transparency: 0,
            shape: Shape::Fill {
                rule: FillRule::EvenOdd,
                contours: vec![vec![
                    [x, y],
                    [x + side, y],
                    [x + side, y + side],
                    [x, y + side],
                ]],
            },
        }
    }

    fn list(items: Vec<Item>) -> DisplayList {
        DisplayList {
            items,
            ..DisplayList::default()
        }
    }

    #[test]
    fn a_fill_wholly_cut_away_is_not_picked_beside_the_boundary() {
        // A small solid just outside a triangular boundary, within reach of
        // both the solid and the boundary: nothing of it shows.
        let mut solid = square(0, [70.0, 70.2], 0.4);
        solid.viewport = Some(1);
        solid.clip = Some(Bounds {
            min: [20.0, 20.0],
            max: [120.0, 120.0],
        });
        solid.clip_boundary = Some(0);
        let mut sheet = list(vec![solid]);
        sheet.clip_boundaries = vec![vec![[20.0, 20.0], [120.0, 20.0], [20.0, 120.0]]];
        assert_eq!(pick(&sheet, [70.0, 69.8], 0.5, 1.0), None);
        // Inside a notched boundary, the part that shows is picked; in the
        // notch, beside it, what is cut away is not.
        let mut wide = square(0, [0.0, 40.0], 100.0);
        wide.viewport = Some(1);
        wide.clip = Some(Bounds {
            min: [0.0, 0.0],
            max: [100.0, 100.0],
        });
        wide.clip_boundary = Some(0);
        let mut sheet = list(vec![wide]);
        sheet.clip_boundaries = vec![vec![
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 100.0],
            [60.0, 100.0],
            [60.0, 20.0],
            [40.0, 20.0],
            [40.0, 100.0],
            [0.0, 100.0],
        ]];
        assert_eq!(pick(&sheet, [20.0, 60.0], 0.1, 1.0), Some(0));
        assert_eq!(pick(&sheet, [50.0, 60.0], 0.1, 1.0), None);
    }

    #[test]
    fn a_fill_whose_hole_covers_the_boundary_is_not_picked_on_a_seam() {
        // A hatch whose hole covers the whole of a concave boundary, cut at
        // the boundary's box as the pipeline cuts it: the outer contour and
        // the hole meet there and cancel by the even-odd rule, so nothing of
        // it shows, on a seam of the boundary's triangles or anywhere.
        let mut hatch = Item {
            shape: Shape::Fill {
                rule: FillRule::EvenOdd,
                contours: vec![
                    vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
                    vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
                ],
            },
            ..square(0, [0.0, 0.0], 1.0)
        };
        hatch.viewport = Some(1);
        hatch.clip = Some(Bounds {
            min: [0.0, 0.0],
            max: [100.0, 100.0],
        });
        hatch.clip_boundary = Some(0);
        let mut sheet = list(vec![hatch]);
        sheet.clip_boundaries = vec![vec![[0.0, 0.0], [100.0, 0.0], [20.0, 20.0], [0.0, 100.0]]];
        assert_eq!(pick(&sheet, [10.0, 10.0], 0.1, 1.0), None);
        assert_eq!(pick(&sheet, [5.0, 50.0], 0.1, 1.0), None);
    }

    #[test]
    fn what_a_viewport_cuts_away_is_not_picked() {
        // A solid seen through a viewport clipped to a triangle: its fill
        // is cut at the triangle's box, and its ink at the triangle.
        let mut solid = square(0, [-100.0, -100.0], 300.0);
        solid.viewport = Some(1);
        solid.clip = Some(Bounds {
            min: [20.0, 20.0],
            max: [120.0, 120.0],
        });
        solid.clip_boundary = Some(0);
        let mut sheet = list(vec![solid.clone()]);
        sheet.clip_boundaries = vec![vec![[20.0, 20.0], [120.0, 20.0], [20.0, 120.0]]];
        assert_eq!(pick(&sheet, [110.0, 110.0], 0.1, 1.0), None);
        assert_eq!(pick(&sheet, [40.0, 40.0], 0.1, 1.0), Some(0));
        // Through a rectangular viewport, outside its window.
        solid.clip_boundary = None;
        let sheet = list(vec![solid]);
        assert_eq!(pick(&sheet, [130.0, 50.0], 0.1, 1.0), None);
        assert_eq!(pick(&sheet, [110.0, 110.0], 0.1, 1.0), Some(0));
    }

    #[test]
    fn the_nearest_object_within_reach_is_picked() {
        let sheet = list(vec![
            stroke(0, &[[0.0, 0.0], [10.0, 0.0]], 0.25),
            stroke(1, &[[0.0, 2.0], [10.0, 2.0]], 0.25),
        ]);
        assert_eq!(pick(&sheet, [5.0, 0.4], 0.5, 1.0), Some(0));
        assert_eq!(pick(&sheet, [5.0, 1.4], 0.5, 1.0), Some(1));
        assert_eq!(
            pick(&sheet, [5.0, 1.0], 0.5, 1.0),
            None,
            "nothing within reach"
        );
        assert_eq!(
            pick(&sheet, [12.0, 0.0], 0.5, 1.0),
            None,
            "past the line's end"
        );
    }

    #[test]
    fn a_pen_is_reached_at_its_edge_not_its_middle() {
        // A 2 mm pen on a sheet drawn in millimetres reaches 1 mm each side.
        let sheet = list(vec![stroke(0, &[[0.0, 0.0], [10.0, 0.0]], 2.0)]);
        assert_eq!(pick(&sheet, [5.0, 1.4], 0.5, 1.0), Some(0));
        assert_eq!(pick(&sheet, [5.0, 1.6], 0.5, 1.0), None);
        // In inches, the same pen is 2 / 25.4 in wide.
        assert_eq!(pick(&sheet, [5.0, 0.1], 0.07, 25.4), Some(0));
    }

    #[test]
    fn a_fill_is_picked_inside_and_the_later_object_wins_a_tie() {
        let sheet = list(vec![
            square(0, [0.0, 0.0], 4.0),
            square(1, [2.0, 2.0], 4.0),
            stroke(2, &[[20.0, 0.0], [20.0, 1.0]], 0.0),
        ]);
        assert_eq!(pick(&sheet, [1.0, 1.0], 0.1, 1.0), Some(0));
        assert_eq!(pick(&sheet, [3.0, 3.0], 0.1, 1.0), Some(1), "on top");
        assert_eq!(pick(&sheet, [5.0, 5.0], 0.1, 1.0), Some(1));
        assert_eq!(
            pick(&sheet, [4.05, 1.0], 0.1, 1.0),
            Some(0),
            "near its edge"
        );
        assert_eq!(pick(&sheet, [20.0, 2.0], 1.5, 1.0), Some(2), "a dot's end");
    }

    #[test]
    fn an_objects_box_holds_everything_it_draws() {
        let sheet = list(vec![
            stroke(3, &[[1.0, 1.0], [2.0, 5.0]], 0.25),
            square(4, [0.0, 0.0], 1.0),
            square(3, [6.0, -1.0], 1.0),
        ]);
        assert_eq!(
            object_bounds(&sheet, 3),
            Some(Bounds {
                min: [1.0, -1.0],
                max: [7.0, 5.0]
            })
        );
        assert_eq!(object_bounds(&sheet, 9), None);
    }
}
