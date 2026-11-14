//! AutoCAD's object snaps, as points an agent can use: on one object by
//! itself (endpoint, midpoint, centre, quadrant, insertion and node), and
//! between two (intersection, apparent intersection, and perpendicular,
//! tangent and nearest from a point of another object).
//!
//! A [`SnapRef`] names a snapped point so that it can be asked for again:
//! its object by ID or name, the snap's kind, and which of the points of
//! that kind, named by its place in the object. Nothing is selected by
//! coordinates: the point a perpendicular, a tangent or a nearest point is
//! taken from is itself a snap of another object.

use serde::{Deserialize, Serialize};

use super::curve::{crossings, length_of, plan_distance, scale_of, sub, where_on, Curve, P3};
use super::{point3, OwnSnap, Part, Reader};
use crate::model::Point3;

/// One of AutoCAD's object snaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapKind {
    /// The ends of a line, an arc or a spline, and each point of a
    /// polyline, a solid or a leader.
    Endpoint,
    /// The middle of a line, of each segment, and along an arc's, an
    /// elliptical arc's or a spline's length.
    Midpoint,
    /// The centre of a circle, an arc, an ellipse, a donut and each arc
    /// segment of a polyline.
    Center,
    /// The points at 0, 90, 180 and 270 degrees on a circle, an arc or an
    /// arc segment, and the ends of an ellipse's axes, turned with an
    /// insert that turns them.
    Quadrant,
    /// The point text, an insert, an attribute, a table or a multileader's
    /// content is placed at.
    Insertion,
    /// Where justified text's baseline starts, as AutoCAD 2026's node snap
    /// found it.
    Node,
    /// Where two objects cross.
    Intersection,
    /// Where two objects cross seen from above, whatever their heights.
    ApparentIntersection,
    /// The foot of the perpendicular from a point to an object.
    Perpendicular,
    /// Where a line from a point touches a curve.
    Tangent,
    /// The point of an object nearest a point.
    Nearest,
}

impl SnapKind {
    /// The snaps of one object by itself, in the order a reply lists them.
    pub const OWN: [SnapKind; 6] = [
        SnapKind::Endpoint,
        SnapKind::Midpoint,
        SnapKind::Center,
        SnapKind::Quadrant,
        SnapKind::Insertion,
        SnapKind::Node,
    ];

    /// Every snap, in the order error messages list them.
    pub const ALL: [SnapKind; 11] = [
        SnapKind::Endpoint,
        SnapKind::Midpoint,
        SnapKind::Center,
        SnapKind::Quadrant,
        SnapKind::Insertion,
        SnapKind::Node,
        SnapKind::Intersection,
        SnapKind::ApparentIntersection,
        SnapKind::Perpendicular,
        SnapKind::Tangent,
        SnapKind::Nearest,
    ];

    /// The name a request writes, such as `apparent_intersection`.
    pub const fn name(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "endpoint",
            SnapKind::Midpoint => "midpoint",
            SnapKind::Center => "center",
            SnapKind::Quadrant => "quadrant",
            SnapKind::Insertion => "insertion",
            SnapKind::Node => "node",
            SnapKind::Intersection => "intersection",
            SnapKind::ApparentIntersection => "apparent_intersection",
            SnapKind::Perpendicular => "perpendicular",
            SnapKind::Tangent => "tangent",
            SnapKind::Nearest => "nearest",
        }
    }

    /// The snap in words, as `count` of them: "1 endpoint", "2 centers".
    fn counted(self, count: usize) -> String {
        let one = self.word();
        if count == 1 {
            format!("1 {one}")
        } else {
            format!("{count} {one}s")
        }
    }

    /// The snap in words, such as "insertion point".
    pub(crate) const fn word(self) -> &'static str {
        match self {
            SnapKind::Endpoint => "endpoint",
            SnapKind::Midpoint => "midpoint",
            SnapKind::Center => "center",
            SnapKind::Quadrant => "quadrant",
            SnapKind::Insertion => "insertion point",
            SnapKind::Node => "node",
            SnapKind::Intersection => "intersection",
            SnapKind::ApparentIntersection => "apparent intersection",
            SnapKind::Perpendicular => "perpendicular",
            SnapKind::Tangent => "tangent",
            SnapKind::Nearest => "nearest point",
        }
    }

    /// Whether the snap is between the object and another, `with`.
    pub const fn takes_with(self) -> bool {
        matches!(
            self,
            SnapKind::Intersection | SnapKind::ApparentIntersection
        )
    }

    /// Whether the snap is from a point of another object, `from`.
    pub const fn takes_from(self) -> bool {
        matches!(
            self,
            SnapKind::Perpendicular | SnapKind::Tangent | SnapKind::Nearest
        )
    }
}

/// An object, named by its ID or by its name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectRef {
    /// The object's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The object's name, which only one sheet of those looked at may give.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A point one of AutoCAD's object snaps finds, named so that it can be
/// asked for again: the object by its ID or its name, the snap, and which
/// of the points of that kind. In a query, without `snap`, every snap of
/// the object by itself; without `which`, every point of the kind.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapRef {
    /// The object's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The object's name, which only one sheet of those looked at may give.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The snap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<SnapKind>,
    /// Which of the object's points of that kind, as a reply names it, such
    /// as `points[2]`, `segments[1]`, `90` or `objects[3].to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub which: Option<String>,
    /// The other object of an intersection or an apparent intersection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<ObjectRef>,
    /// The point a perpendicular, a tangent or a nearest point is taken
    /// from: a snap of another object, which must name one point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Box<SnapRef>>,
}

impl SnapRef {
    /// The object it names, by ID or name.
    pub fn object(&self) -> ObjectRef {
        ObjectRef {
            id: self.id.clone(),
            name: self.name.clone(),
        }
    }
}

impl ObjectRef {
    /// The object in words, such as "the ID `DEMO-001/border`".
    pub(crate) fn words(&self) -> String {
        match (&self.id, &self.name) {
            (Some(id), _) => format!("the ID `{id}`"),
            (None, Some(name)) => format!("the name `{name}`"),
            (None, None) => "no object".to_owned(),
        }
    }
}

/// One point a snap found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SnapPoint {
    pub snap: SnapKind,
    /// Which of the object's points of this kind it is, by its place, to
    /// ask for it again; none when the object has only one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub which: Option<String>,
    /// For a snap between two objects, the place in the object of the part
    /// it lies on, such as `segments[1]` or `objects[3]`; none for the
    /// object itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// For an intersection, the place of the part of the other object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_part: Option<String>,
    /// When the point is in an insert's block, the blocks its place passes
    /// into, the insert's first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<String>,
    /// Where it is, in the declared unit.
    pub at: Point3,
}

/// Every point of the object's own snaps, `kind` only when given, each
/// with its name, its place in the object, which a point of the object
/// itself with no place of its own, such as a line's midpoint, has not.
pub(crate) fn own_points(
    reader: &Reader,
    object: &crate::model::Object,
    kind: Option<SnapKind>,
) -> Vec<(String, SnapPoint)> {
    let snaps: Vec<OwnSnap> = reader.own_snaps(object);
    let mut points = Vec::new();
    for k in SnapKind::OWN {
        if kind.is_some_and(|wanted| wanted != k) {
            continue;
        }
        let of_kind: Vec<&OwnSnap> = snaps.iter().filter(|s| s.kind == k).collect();
        for snap in of_kind {
            let point = SnapPoint {
                snap: k,
                which: (!snap.which.is_empty()).then(|| snap.which.clone()),
                part: None,
                with_part: None,
                blocks: snap.blocks.clone(),
                at: point3(snap.at),
            };
            points.push((snap.which.clone(), point));
        }
    }
    points
}

/// The tolerance two points within are one, for curves as large as `size`.
fn tolerance(size: f64) -> f64 {
    1e-9 * size
}

/// Where `parts` and `others` cross, seen from above; only where they meet
/// in space too unless `apparent`. Each point lies on the first object, and
/// they are in the order they lie along it: by its parts, then along each.
pub(crate) fn intersections(parts: &[Part], others: &[Part], apparent: bool) -> Vec<SnapPoint> {
    let kind = if apparent {
        SnapKind::ApparentIntersection
    } else {
        SnapKind::Intersection
    };
    // Each crossing, with its part's place among the first object's parts
    // and its parameter along it, to put them in order, and the end of
    // each part it lies at, if it does.
    let mut crossed: Vec<Crossed> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        for (j, other) in others.iter().enumerate() {
            let size = scale_of(&part.curve).max(scale_of(&other.curve));
            for (t, u) in crossings(&part.curve, &other.curve) {
                let (p, q) = (part.curve.point(t), other.curve.point(u));
                if !apparent && (p[2] - q[2]).abs() > tolerance(size) {
                    continue;
                }
                crossed.push(Crossed {
                    part: i,
                    other: j,
                    t,
                    u,
                    t_slack: slack(&part.curve),
                    u_slack: slack(&other.curve),
                    at_end: end_at(&part.curve, t),
                    other_end: end_at(&other.curve, u),
                    size,
                    point: found(kind, part, Some(other), p),
                });
            }
        }
    }
    crossed.sort_by(|a, b| a.part.cmp(&b.part).then(a.t.total_cmp(&b.t)));
    // One point can be found more than once: on two parts of one object
    // that meet there, such as a polyline's corner, at both ends of a
    // closed curve, or where one part crosses two of the other's that
    // cross each other, as a block's lines may. It is one point where both
    // finds lie at an end of their parts and those ends are one place, or
    // where both are at the same parameter of the same part. No two points
    // are one for being close alone.
    let mut kept: Vec<Crossed> = Vec::new();
    for c in crossed {
        let apart = |a: P3, b: P3| {
            if apparent {
                plan_distance(a, b)
            } else {
                length_of(sub(a, b))
            }
        };
        let same_end = |a: Option<P3>, b: Option<P3>, size: f64| match (a, b) {
            (Some(a), Some(b)) => apart(a, b) <= tolerance(size),
            _ => false,
        };
        let seen = kept.iter().any(|k| {
            (k.part == c.part && (k.t - c.t).abs() <= c.t_slack)
                || (k.other == c.other && (k.u - c.u).abs() <= c.u_slack)
                || same_end(k.at_end, c.at_end, c.size)
                || same_end(k.other_end, c.other_end, c.size)
        });
        if !seen {
            kept.push(c);
        }
    }
    numbered(kept.into_iter().map(|c| c.point).collect())
}

/// Where one object's geometry meets another's, and the part of each that
/// meets there.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Touch {
    /// The place in the object of the part that meets the other, such as
    /// `segments[1]` or `objects[3]`; none for the object itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// The place in the other object of the part it meets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_part: Option<String>,
    /// Where they meet, in the declared unit.
    pub at: Point3,
}

/// Where `parts` meet `others` in space: where a part of one crosses or
/// touches a part of the other, as an intersection snap finds it, and where
/// an end of a part of either lies on a part of the other within the
/// tolerance of their size, as a wire drawn to a terminal's end, or along
/// it, meets it. Each place is given once, with the first parts found
/// there, in the order of `parts`.
pub(crate) fn touches(parts: &[Part], others: &[Part]) -> Vec<Touch> {
    let mut found: Vec<(P3, f64, Touch)> = Vec::new();
    let place = |p: &Part| (!p.path.is_empty()).then(|| p.path.clone());
    let mut add = |at: P3, size: f64, part: &Part, other: &Part| {
        if found
            .iter()
            .any(|(seen, s, _)| length_of(sub(*seen, at)) <= tolerance(s.max(size)))
        {
            return;
        }
        found.push((
            at,
            size,
            Touch {
                part: place(part),
                with_part: place(other),
                at: point3(at),
            },
        ));
    };
    for part in parts {
        for other in others {
            let size = scale_of(&part.curve).max(scale_of(&other.curve));
            for (t, u) in crossings(&part.curve, &other.curve) {
                let (p, q) = (part.curve.point(t), other.curve.point(u));
                if length_of(sub(p, q)) <= tolerance(size) {
                    add(p, size, part, other);
                }
            }
            for end in [part.curve.start(), part.curve.end()] {
                if lies_on(end, &other.curve, size) {
                    add(end, size, part, other);
                }
            }
            for end in [other.curve.start(), other.curve.end()] {
                if lies_on(end, &part.curve, size) {
                    add(end, size, part, other);
                }
            }
        }
    }
    found.into_iter().map(|(_, _, t)| t).collect()
}

/// Whether `p` lies on the curve, in space, within the tolerance of `size`.
fn lies_on(p: P3, c: &Curve, size: f64) -> bool {
    nearest_on(c, p).is_some_and(|q| length_of(sub(p, q)) <= tolerance(size))
}

/// A crossing found between a part of the first object and one of the
/// other's.
struct Crossed {
    part: usize,
    other: usize,
    t: f64,
    /// Its parameter along the other object's part.
    u: f64,
    /// How far apart two parameters along either part are one.
    t_slack: f64,
    u_slack: f64,
    /// The end of the first object's part it lies at, if it does.
    at_end: Option<P3>,
    /// The end of the other object's part it lies at, if it does.
    other_end: Option<P3>,
    size: f64,
    point: SnapPoint,
}

/// How far apart two parameters along `c` are one.
fn slack(c: &Curve) -> f64 {
    let (a, b) = c.domain();
    (b - a) * 1e-9
}

/// The end of `c` that the parameter `t` is at, within a little.
fn end_at(c: &Curve, t: f64) -> Option<P3> {
    let (a, b) = c.domain();
    let slack = slack(c);
    if t - a <= slack {
        Some(c.start())
    } else if b - t <= slack {
        Some(c.end())
    } else {
        None
    }
}

fn found(kind: SnapKind, part: &Part, other: Option<&Part>, at: P3) -> SnapPoint {
    let place = |p: &Part| (!p.path.is_empty()).then(|| p.path.clone());
    SnapPoint {
        snap: kind,
        which: None,
        part: place(part),
        with_part: other.and_then(place),
        blocks: part.blocks.clone(),
        at: point3(at),
    }
}

/// The points named by their order, `0`, `1`, ..., when there are several.
fn numbered(mut points: Vec<SnapPoint>) -> Vec<SnapPoint> {
    if points.len() > 1 {
        for (i, p) in points.iter_mut().enumerate() {
            p.which = Some(i.to_string());
        }
    }
    points
}

/// The points of `parts` that `kind`, perpendicular, tangent or nearest,
/// finds from `from`, seen from above, nearest `from` first.
pub(crate) fn from_point(parts: &[Part], kind: SnapKind, from: P3) -> Vec<SnapPoint> {
    let mut points: Vec<(f64, SnapPoint)> = Vec::new();
    for part in parts {
        let at: Vec<P3> = match kind {
            SnapKind::Perpendicular => perpendiculars(&part.curve, from, true),
            SnapKind::Tangent => tangents(&part.curve, from),
            _ => nearest_on(&part.curve, from).into_iter().collect(),
        };
        let size = scale_of(&part.curve);
        for p in at {
            if points.iter().any(|(_, seen)| {
                plan_distance([seen.at.x, seen.at.y, seen.at.z], p) <= tolerance(size)
            }) {
                continue;
            }
            points.push((plan_distance(p, from), found(kind, part, None, p)));
        }
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    if kind == SnapKind::Nearest {
        points.truncate(1);
    }
    numbered(points.into_iter().map(|(_, p)| p).collect())
}

/// The feet of the perpendiculars from `from` to the curve, seen from
/// above. A straight segment's is on the line through it, as AutoCAD's
/// perpendicular snap reaches past a line's or a polyline segment's end,
/// unless `extend` is false; an arc's and a curve's are on it.
fn perpendiculars(c: &Curve, from: P3, extend: bool) -> Vec<P3> {
    match c {
        Curve::Segment { a, b } => {
            let d = sub(*b, *a);
            let l2 = d[0] * d[0] + d[1] * d[1];
            if l2 == 0.0 {
                return Vec::new();
            }
            let t = ((from[0] - a[0]) * d[0] + (from[1] - a[1]) * d[1]) / l2;
            let t = if extend { t } else { t.clamp(0.0, 1.0) };
            vec![c.point(t)]
        }
        Curve::Arc {
            center,
            start,
            sweep,
            ..
        } => {
            if plan_distance(*center, from) == 0.0 {
                return Vec::new();
            }
            let angle = (from[1] - center[1]).atan2(from[0] - center[0]);
            [angle, angle + std::f64::consts::PI]
                .into_iter()
                .filter_map(|a| arc_share(a, *start, *sweep).map(|s| c.point(s)))
                .collect()
        }
        _ => where_on(c, &|p, d| {
            let w = [p[0] - from[0], p[1] - from[1]];
            (
                w[0] * d[0] + w[1] * d[1],
                w[0].hypot(w[1]) * d[0].hypot(d[1]),
            )
        })
        .into_iter()
        .map(|t| c.point(t))
        .collect(),
    }
}

/// Where lines from `from` touch the curve, seen from above: none on a
/// straight segment, or from a point inside a circle.
fn tangents(c: &Curve, from: P3) -> Vec<P3> {
    match c {
        Curve::Segment { .. } => Vec::new(),
        Curve::Arc {
            center,
            radius,
            start,
            sweep,
        } => {
            let d = plan_distance(*center, from);
            if d <= *radius {
                return Vec::new();
            }
            let angle = (from[1] - center[1]).atan2(from[0] - center[0]);
            let off = (radius / d).acos();
            [angle + off, angle - off]
                .into_iter()
                .filter_map(|a| arc_share(a, *start, *sweep).map(|s| c.point(s)))
                .collect()
        }
        _ => where_on(c, &|p, d| {
            let w = [p[0] - from[0], p[1] - from[1]];
            (
                w[0] * d[1] - w[1] * d[0],
                w[0].hypot(w[1]) * d[0].hypot(d[1]),
            )
        })
        .into_iter()
        .map(|t| c.point(t))
        .collect(),
    }
}

/// The point of the curve nearest `from`, seen from above.
fn nearest_on(c: &Curve, from: P3) -> Option<P3> {
    let mut candidates = perpendiculars(c, from, false);
    candidates.push(c.start());
    candidates.push(c.end());
    candidates
        .into_iter()
        .min_by(|a, b| plan_distance(*a, from).total_cmp(&plan_distance(*b, from)))
}

/// The share along the arc from `start` sweeping `sweep` of the point at
/// `angle`; none when it is not on the arc.
fn arc_share(angle: f64, start: f64, sweep: f64) -> Option<f64> {
    use std::f64::consts::TAU;
    let s = if sweep >= 0.0 {
        (angle - start).rem_euclid(TAU) / sweep
    } else {
        (start - angle).rem_euclid(TAU) / -sweep
    };
    let turn = TAU / sweep.abs();
    if s <= 1.0 + 1e-12 {
        Some(s.min(1.0))
    } else if turn - s <= 1e-12 * turn {
        Some(0.0)
    } else {
        None
    }
}

/// The snaps in words, as a reply's message counts them: "2 endpoints, 1
/// midpoint".
pub(crate) fn tally(points: &[SnapPoint]) -> String {
    SnapKind::ALL
        .iter()
        .filter_map(|&k| {
            let n = points.iter().filter(|p| p.snap == k).count();
            (n > 0).then(|| k.counted(n))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The snap in words, `count` of them.
pub(crate) fn counted(kind: SnapKind, count: usize) -> String {
    kind.counted(count)
}
