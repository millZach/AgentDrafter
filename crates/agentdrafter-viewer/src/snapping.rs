//! The snap the cursor catches: of the snaps the display list gives, those
//! of the running kinds within reach of the cursor, nearest first, and of
//! two about as near, the one AutoCAD's AutoSnap would take, by the order
//! of their kinds; Tab steps through the rest. The running Nearest is the
//! nearest point of what the page draws, found by the core's `nearest`,
//! taken only when no other snap is in reach, as it lies everywhere on
//! every object. Plain computation, tested natively; `web` draws it.

use agentdrafter_core::{nearest, DisplayList, DisplaySnap, Outline, SnapKind};

/// Which of two snaps about as near the cursor wins: AutoCAD's ends and
/// crossings first, then middles and centres, then the rest.
pub const PRIORITY: [SnapKind; 11] = [
    SnapKind::Endpoint,
    SnapKind::Intersection,
    SnapKind::ApparentIntersection,
    SnapKind::Midpoint,
    SnapKind::Center,
    SnapKind::Quadrant,
    SnapKind::Insertion,
    SnapKind::Node,
    SnapKind::Perpendicular,
    SnapKind::Tangent,
    SnapKind::Nearest,
];

/// The bit of a snap kind in a set of running snaps: its place among
/// [`SnapKind::ALL`].
pub fn bit(kind: SnapKind) -> u32 {
    SnapKind::ALL
        .iter()
        .position(|&k| k == kind)
        .map_or(0, |i| 1 << i)
}

/// A snap in reach of the cursor.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub snap: SnapKind,
    /// The object's place in the sheet's objects.
    pub object: usize,
    /// Which of its points, as `query snap` names it.
    pub which: Option<String>,
    /// For an intersection, the other object's place.
    pub with: Option<usize>,
    /// Where it is, in drawing units.
    pub at: [f64; 3],
    /// How far from the cursor, in drawing units.
    pub distance: f64,
}

fn rank(kind: SnapKind) -> usize {
    PRIORITY
        .iter()
        .position(|&k| k == kind)
        .unwrap_or(PRIORITY.len())
}

/// The snaps of the kinds `running` holds (bits by [`bit`]) within `reach`
/// drawing units of `at`, in the order the cursor takes them: those within
/// `tie` of the nearest first, the higher in [`PRIORITY`] first among them,
/// then the rest, nearest first. One point found twice as one kind, the
/// same in all three of its coordinates, is offered once. With Nearest
/// running and nothing else in reach, the nearest point of what `list`
/// draws, along the `outlines` of its fills.
pub fn candidates(
    snaps: &[DisplaySnap],
    list: &DisplayList,
    outlines: &[Outline],
    running: u32,
    at: [f64; 2],
    reach: f64,
    tie: f64,
) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = Vec::new();
    if running == 0 || reach.is_nan() || reach <= 0.0 {
        return found;
    }
    for s in snaps {
        if running & bit(s.snap) == 0 {
            continue;
        }
        let (dx, dy) = (s.at.x - at[0], s.at.y - at[1]);
        if dx.abs() > reach || dy.abs() > reach {
            continue;
        }
        let distance = dx.hypot(dy);
        if distance > reach {
            continue;
        }
        // Two points one above the other are two: Tab reaches each.
        let same = |c: &Candidate| {
            c.snap == s.snap
                && (c.at[0] - s.at.x)
                    .hypot(c.at[1] - s.at.y)
                    .hypot(c.at[2] - s.at.z)
                    <= reach * 1e-9
        };
        if found.iter().any(same) {
            continue;
        }
        found.push(Candidate {
            snap: s.snap,
            object: s.object,
            which: s.which.clone(),
            with: s.with,
            at: [s.at.x, s.at.y, s.at.z],
            distance,
        });
    }
    if found.is_empty() && running & bit(SnapKind::Nearest) != 0 {
        if let Some((object, p)) = nearest(list, outlines, at, reach) {
            found.push(Candidate {
                snap: SnapKind::Nearest,
                object,
                which: None,
                with: None,
                at: [p[0], p[1], 0.0],
                distance: (p[0] - at[0]).hypot(p[1] - at[1]),
            });
        }
    }
    // Those within `tie` of the nearest are about as near it: among them
    // the kind decides, then the distance; the rest follow by distance.
    let nearest = found
        .iter()
        .map(|c| c.distance)
        .fold(f64::INFINITY, f64::min);
    let close = |c: &Candidate| c.distance <= nearest + tie.max(0.0);
    found.sort_by(|a, b| {
        close(b)
            .cmp(&close(a))
            .then_with(|| {
                if close(a) {
                    rank(a.snap).cmp(&rank(b.snap))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then(a.distance.total_cmp(&b.distance))
            .then(rank(a.snap).cmp(&rank(b.snap)))
    });
    found
}

#[cfg(test)]
mod tests {
    use agentdrafter_core::{Color, Item, Point3, Shape};

    use super::*;

    fn snap(object: usize, snap: SnapKind, x: f64, y: f64) -> DisplaySnap {
        DisplaySnap {
            object,
            snap,
            which: None,
            with: None,
            at: Point3::new(x, y, 0.0),
        }
    }

    const RUNNING: u32 = 0b111_1111;

    #[test]
    fn the_nearest_snap_in_reach_comes_first() {
        let snaps = [
            snap(0, SnapKind::Endpoint, 1.0, 0.0),
            snap(1, SnapKind::Midpoint, 0.3, 0.0),
            snap(2, SnapKind::Center, 5.0, 0.0),
        ];
        let found = candidates(
            &snaps,
            &DisplayList::default(),
            &[],
            RUNNING,
            [0.0, 0.0],
            2.0,
            0.01,
        );
        let kinds: Vec<SnapKind> = found.iter().map(|c| c.snap).collect();
        assert_eq!(kinds, [SnapKind::Midpoint, SnapKind::Endpoint]);
        assert!((found[0].distance - 0.3).abs() < 1e-12);
    }

    #[test]
    fn of_two_about_as_near_autocads_order_picks() {
        let snaps = [
            snap(0, SnapKind::Midpoint, 1.0, 0.0),
            snap(1, SnapKind::Endpoint, 0.0, 1.001),
            snap(2, SnapKind::Intersection, -1.0005, 0.0),
        ];
        let found = candidates(
            &snaps,
            &DisplayList::default(),
            &[],
            RUNNING,
            [0.0, 0.0],
            2.0,
            0.01,
        );
        let kinds: Vec<SnapKind> = found.iter().map(|c| c.snap).collect();
        assert_eq!(
            kinds,
            [
                SnapKind::Endpoint,
                SnapKind::Intersection,
                SnapKind::Midpoint
            ]
        );
    }

    #[test]
    fn the_tie_is_measured_from_the_nearest_snap() {
        // A midpoint 1.9 from the cursor and an endpoint 2.1, 2 apart at
        // most counting as about as near: the endpoint, higher in the
        // order, first, though steps of 2 would part them.
        let snaps = [
            snap(0, SnapKind::Midpoint, 1.9, 0.0),
            snap(1, SnapKind::Endpoint, 0.0, 2.1),
            snap(2, SnapKind::Center, 0.0, -4.5),
            snap(3, SnapKind::Endpoint, -5.0, 0.0),
        ];
        let found = candidates(
            &snaps,
            &DisplayList::default(),
            &[],
            RUNNING,
            [0.0, 0.0],
            10.0,
            2.0,
        );
        let order: Vec<usize> = found.iter().map(|c| c.object).collect();
        // Then the rest by distance, whatever their kinds.
        assert_eq!(order, [1, 0, 2, 3]);
    }

    #[test]
    fn two_points_one_above_the_other_are_two() {
        let mut high = snap(1, SnapKind::Endpoint, 0.5, 0.0);
        high.at.z = 10.0;
        let snaps = [snap(0, SnapKind::Endpoint, 0.5, 0.0), high];
        let found = candidates(
            &snaps,
            &DisplayList::default(),
            &[],
            RUNNING,
            [0.0, 0.0],
            2.0,
            0.01,
        );
        let at: Vec<[f64; 3]> = found.iter().map(|c| c.at).collect();
        assert_eq!(at, [[0.5, 0.0, 0.0], [0.5, 0.0, 10.0]]);
    }

    #[test]
    fn only_the_running_snaps_and_one_point_once() {
        let snaps = [
            snap(0, SnapKind::Endpoint, 0.5, 0.0),
            // The same corner, the end of another line too: offered once.
            snap(1, SnapKind::Endpoint, 0.5, 0.0),
            snap(2, SnapKind::Quadrant, 0.2, 0.0),
        ];
        let only_ends = bit(SnapKind::Endpoint);
        let found = candidates(
            &snaps,
            &DisplayList::default(),
            &[],
            only_ends,
            [0.0, 0.0],
            2.0,
            0.01,
        );
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].snap, found[0].object), (SnapKind::Endpoint, 0));
        // Snapping off: nothing.
        assert!(candidates(
            &snaps,
            &DisplayList::default(),
            &[],
            0,
            [0.0, 0.0],
            2.0,
            0.01
        )
        .is_empty());
    }

    #[test]
    fn nearest_is_taken_only_when_nothing_else_is_in_reach() {
        let list = DisplayList {
            items: vec![Item {
                object: 4,
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
                    lineweight: 0.25,
                    runs: vec![vec![[0.0, 0.0], [10.0, 0.0]]],
                },
            }],
            ..DisplayList::default()
        };
        let running = bit(SnapKind::Endpoint) | bit(SnapKind::Nearest);
        let found = candidates(&[], &list, &[], running, [3.0, 0.4], 1.0, 0.01);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].snap, found[0].object, found[0].at),
            (SnapKind::Nearest, 4, [3.0, 0.0, 0.0])
        );
        let end = [snap(4, SnapKind::Endpoint, 3.5, 0.0)];
        let found = candidates(&end, &list, &[], running, [3.0, 0.4], 1.0, 0.01);
        assert_eq!(
            found.iter().map(|c| c.snap).collect::<Vec<_>>(),
            [SnapKind::Endpoint]
        );
    }
}
