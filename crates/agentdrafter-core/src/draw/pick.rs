//! Picking: which of a sheet's objects the reviewer means by a point, found
//! in the display list the viewer draws, so what is picked is what is
//! drawn there, and the page holds no second hit-test of its own.
//!
//! Each item of the display list names the object of the sheet that draws
//! it, an insert for everything its block draws. A stroke is reached within
//! `reach` of its pen's edge, and a fill anywhere inside it, by its fill
//! rule, or within `reach` of its edge. The object drawn nearest the point
//! is picked; of two as near, the one drawn later, which lies on top.

use super::{Bounds, DisplayList, FillRule, Point2, Shape};

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
            Shape::Fill { rule, contours } => {
                if inside(at, *rule, contours) {
                    0.0
                } else {
                    contours
                        .iter()
                        .map(|c| to_ring(at, c))
                        .fold(f64::INFINITY, f64::min)
                }
            }
        };
        // Later items lie on top, so they win a tie.
        if distance <= reach && best.is_none_or(|(d, _)| distance <= d) {
            best = Some((distance, item.object));
        }
    }
    best.map(|(_, object)| object)
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
                Shape::Fill { contours, .. } => {
                    contours.iter().flatten().copied().collect::<Vec<_>>()
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
fn inside(p: Point2, rule: FillRule, contours: &[Vec<Point2>]) -> bool {
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
    use super::super::{Color, Item};
    use super::*;

    fn stroke(object: usize, points: &[Point2], lineweight: f64) -> Item {
        Item {
            object,
            part: None,
            nested: Vec::new(),
            color: Color(7),
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
            color: Color(1),
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
