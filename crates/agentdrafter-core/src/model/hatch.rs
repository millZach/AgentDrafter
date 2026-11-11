//! The core's model of a hatch: an area filled solid or with a pattern,
//! inside a boundary of one or more closed loops.
//!
//! A DWG keeps no table of hatch patterns: each hatch carries its pattern's
//! lines. The model does the same. A sheet description defines a pattern
//! once, by name, and every hatch that names it carries a copy.

use serde::{Deserialize, Serialize};

use super::{layer_zero, Point3};

/// The pattern name of AutoCAD's solid hatch. A hatch with this pattern
/// fills its area solid and has no pattern lines, scale or angle.
pub const SOLID: &str = "SOLID";

/// An area filled solid or with a pattern, inside a boundary. It lies in
/// the plane parallel to XY at the `z` its boundary's points share, and
/// draws no outline: an outline is a line, polyline or arc of its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hatch {
    /// What fills the area: `SOLID`, or a pattern of lines.
    pub pattern: HatchPattern,
    /// How much the pattern is enlarged: 1 draws it as defined. 1 for `SOLID`.
    pub scale: f64,
    /// How far the pattern is turned: degrees anticlockwise, in `[0, 360)`.
    /// 0 for `SOLID`.
    pub angle: f64,
    /// The closed loops that bound the area, as AutoCAD's normal island
    /// detection reads them: a point inside an odd number of loops is
    /// filled, so a loop inside another is a hole, and a loop inside that
    /// is filled again.
    pub boundary: Vec<HatchLoop>,
    #[serde(default = "layer_zero")]
    pub layer: String,
}

impl Hatch {
    /// Whether the hatch fills its area solid rather than with a pattern.
    pub fn is_solid(&self) -> bool {
        self.pattern.is_solid()
    }
}

/// A hatch pattern: a name and the families of parallel lines it draws, as
/// in an AutoCAD `.pat` file. `SOLID` has no lines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HatchPattern {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<HatchPatternLine>,
}

impl HatchPattern {
    /// The pattern of AutoCAD's solid hatch.
    pub fn solid() -> Self {
        Self {
            name: SOLID.to_owned(),
            lines: Vec::new(),
        }
    }

    /// Whether this is the pattern of AutoCAD's solid hatch.
    pub fn is_solid(&self) -> bool {
        self.name == SOLID
    }
}

/// One family of parallel lines in a hatch pattern, as one line of an
/// AutoCAD `.pat` file states it, before the hatch's scale and angle.
/// Lengths are in the declared unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HatchPatternLine {
    /// The direction of the lines: degrees anticlockwise from the X axis,
    /// in `[0, 360)`.
    pub angle: f64,
    /// A point one of the lines passes through, as `[x, y]` in the hatch's plane.
    pub origin: [f64; 2],
    /// From one line to the next, as `[along, across]`: `along` runs in the
    /// lines' direction and shifts where the dashes start, and `across`,
    /// never 0, is the distance between neighbouring lines.
    pub delta: [f64; 2],
    /// The dashes, gaps and dots each line repeats: a positive length is a
    /// dash, a negative one a gap, and 0 a dot. Empty for a continuous line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashes: Vec<f64>,
}

/// One closed loop of a hatch's boundary.
///
/// It is written as `{"polyline": {"points": [...]}}` or
/// `{"edges": [...]}` in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HatchLoop {
    /// Straight segments through three or more points, in order, closed
    /// back to the first.
    Polyline { points: Vec<Point3> },
    /// Lines and arcs joined end to end, the last ending where the first
    /// starts. Each line starts where the edge before it ends; an arc,
    /// whose angles always run anticlockwise, may be walked either way.
    Edges(Vec<HatchEdge>),
}

/// A line or an arc in a loop of a hatch's boundary.
///
/// It is written as `{"line": {...}}` or `{"arc": {...}}` in JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HatchEdge {
    Line {
        from: Point3,
        to: Point3,
    },
    /// Part of a circle from `start_angle` anticlockwise to `end_angle`, in
    /// degrees in `[0, 360)`.
    Arc {
        center: Point3,
        radius: f64,
        start_angle: f64,
        end_angle: f64,
    },
}

impl HatchEdge {
    /// The edge's two ends as it is written: a line's `from` and `to`, an
    /// arc's points at its start angle and at its end angle.
    pub fn ends(&self) -> (Point3, Point3) {
        match *self {
            HatchEdge::Line { from, to } => (from, to),
            HatchEdge::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => {
                let at = |degrees: f64| {
                    let (sin, cos) = degrees.to_radians().sin_cos();
                    Point3::new(center.x + radius * cos, center.y + radius * sin, center.z)
                };
                (at(start_angle), at(end_angle))
            }
        }
    }

    /// Every coordinate and radius that states the edge: a line's ends, or
    /// an arc's centre, radius and ends. A loop's size is the largest of them.
    fn sizes(&self) -> Vec<f64> {
        let (a, b) = self.ends();
        let mut sizes = vec![a.x, a.y, b.x, b.y];
        if let HatchEdge::Arc { center, radius, .. } = *self {
            sizes.extend([center.x, center.y, radius]);
        }
        sizes
    }
}

/// How close two ends of edges must be to join, as a share of the largest
/// coordinate or radius in their loop, or of one unit if that is more.
pub(crate) const JOIN_TOLERANCE: f64 = 1e-9;

/// Where a loop of edges stops joining up, as [`walk`] finds it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Break {
    /// The edge at this index does not start, or for an arc start or end,
    /// at any of the points the walk had `reached`.
    Gap { edge: usize, reached: Vec<Point3> },
    /// Every edge joins the one before it, but the last ends at `end`,
    /// not at `start`, where the first starts.
    Open { end: Point3, start: Point3 },
}

/// Walks a loop of edges end to end and says which way each is walked:
/// `true` for an edge walked backwards, from its second end to its first.
/// Each line starts where the edge before it ends. An arc, whose angles
/// always run anticlockwise, is walked whichever way joins; a first arc
/// runs whichever way lets the second edge join it, forwards if both do.
pub(crate) fn walk(edges: &[HatchEdge]) -> Result<Vec<bool>, Break> {
    let Some(first) = edges.first() else {
        return Ok(Vec::new());
    };
    let ends: Vec<(Point3, Point3)> = edges.iter().map(HatchEdge::ends).collect();
    let size = edges
        .iter()
        .flat_map(HatchEdge::sizes)
        .fold(1.0, |size: f64, value| size.max(value.abs()));
    let tolerance = size * JOIN_TOLERANCE;
    let meets =
        |p: Point3, q: Point3| (p.x - q.x).abs() <= tolerance && (p.y - q.y).abs() <= tolerance;

    let (a0, b0) = ends[0];
    let first_is_arc = matches!(first, HatchEdge::Arc { .. });
    let mut backwards = vec![false; edges.len()];
    let mut reached = if first_is_arc { vec![b0, a0] } else { vec![b0] };
    for (i, (edge, &(a, b))) in edges.iter().zip(&ends).enumerate().skip(1) {
        let ways: &[(Point3, Point3, bool)] = match edge {
            HatchEdge::Line { .. } => &[(a, b, false)],
            HatchEdge::Arc { .. } => &[(a, b, false), (b, a, true)],
        };
        let joined = reached.iter().find_map(|&here| {
            ways.iter()
                .find(|(start, _, _)| meets(*start, here))
                .map(|&(_, far, back)| (here, far, back))
        });
        let Some((here, far, back)) = joined else {
            return Err(Break::Gap { edge: i, reached });
        };
        if i == 1 && first_is_arc {
            backwards[0] = here == a0;
        }
        backwards[i] = back;
        reached = vec![far];
    }
    let start = if backwards[0] { b0 } else { a0 };
    let end = reached[0];
    if edges.len() > 1 && meets(end, start) {
        Ok(backwards)
    } else {
        Err(Break::Open { end, start })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(from: [f64; 2], to: [f64; 2]) -> HatchEdge {
        HatchEdge::Line {
            from: Point3::new(from[0], from[1], 0.0),
            to: Point3::new(to[0], to[1], 0.0),
        }
    }

    fn arc(center: [f64; 2], radius: f64, start_angle: f64, end_angle: f64) -> HatchEdge {
        HatchEdge::Arc {
            center: Point3::new(center[0], center[1], 0.0),
            radius,
            start_angle,
            end_angle,
        }
    }

    #[test]
    fn a_loop_walks_its_arcs_whichever_way_joins() {
        // A slot: both arcs forwards.
        let slot = [
            line([0.0, 0.0], [30.0, 0.0]),
            arc([30.0, 5.0], 5.0, 270.0, 90.0),
            line([30.0, 10.0], [0.0, 10.0]),
            arc([0.0, 5.0], 5.0, 90.0, 270.0),
        ];
        assert_eq!(walk(&slot), Ok(vec![false; 4]));
        // A notch: the arc ends where the line before it ends.
        let notch = [
            line([0.0, 0.0], [4.0, 0.0]),
            line([4.0, 0.0], [4.0, 4.0]),
            line([4.0, 4.0], [3.0, 4.0]),
            arc([2.0, 4.0], 1.0, 180.0, 0.0),
            line([1.0, 4.0], [0.0, 4.0]),
            line([0.0, 4.0], [0.0, 0.0]),
        ];
        assert_eq!(
            walk(&notch),
            Ok(vec![false, false, false, true, false, false])
        );
        // A first arc runs whichever way lets the second edge join it.
        let tombstone = [
            arc([0.0, 0.0], 1.0, 0.0, 180.0),
            line([1.0, 0.0], [1.0, -1.0]),
            line([1.0, -1.0], [-1.0, -1.0]),
            line([-1.0, -1.0], [-1.0, 0.0]),
        ];
        assert_eq!(walk(&tombstone), Ok(vec![true, false, false, false]));
        let disc = [
            arc([0.0, 0.0], 1.0, 0.0, 180.0),
            arc([0.0, 0.0], 1.0, 180.0, 0.0),
        ];
        assert_eq!(walk(&disc), Ok(vec![false, false]));
    }

    #[test]
    fn a_loop_that_does_not_join_up_says_where() {
        let gap = [line([0.0, 0.0], [4.0, 0.0]), line([4.0, 1.0], [0.0, 0.0])];
        assert_eq!(
            walk(&gap),
            Err(Break::Gap {
                edge: 1,
                reached: vec![Point3::new(4.0, 0.0, 0.0)],
            })
        );
        let open = [line([0.0, 0.0], [4.0, 0.0]), line([4.0, 0.0], [4.0, 4.0])];
        assert_eq!(
            walk(&open),
            Err(Break::Open {
                end: Point3::new(4.0, 4.0, 0.0),
                start: Point3::new(0.0, 0.0, 0.0),
            })
        );
        // Ends a little more than the tolerance apart do not join.
        let near = [
            line([0.0, 0.0], [1000.0, 0.0]),
            line([1000.0, 2e-6], [0.0, 0.0]),
        ];
        assert!(matches!(walk(&near), Err(Break::Gap { edge: 1, .. })));
        let within = [
            line([0.0, 0.0], [1000.0, 0.0]),
            line([1000.0, 5e-7], [0.0, 0.0]),
        ];
        assert!(walk(&within).is_ok());
    }

    /// The tolerance scales with the largest coordinate or radius in the
    /// loop, an arc's centre among them: here the centre, at x 2000, is
    /// larger than any end or radius, so a gap of 1.5e-6 still joins.
    #[test]
    fn an_arcs_centre_counts_in_the_size_of_its_loop() {
        let sliver = arc([2000.0, 0.0], 1000.0, 170.0, 190.0);
        let (start, end) = sliver.ends();
        let closing = HatchEdge::Line {
            from: end,
            to: Point3::new(start.x, start.y + 1.5e-6, 0.0),
        };
        assert_eq!(walk(&[sliver.clone(), closing]), Ok(vec![false, false]));
        let too_far = HatchEdge::Line {
            from: end,
            to: Point3::new(start.x, start.y + 2.5e-6, 0.0),
        };
        assert!(matches!(walk(&[sliver, too_far]), Err(Break::Open { .. })));
    }
}
