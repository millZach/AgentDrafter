//! Geometric stretch (`stretch` / `STRETCH` / `S`): moves endpoints and
//! vertices inside a selection box by a displacement while leaving vertices
//! outside the box fixed, so connected geometry follows.

use crate::draw::polyline::arc_of;
use crate::error::{Error, ErrorCode};
use crate::model::{
    Arc, HatchEdge, HatchLoop, Line, Measured, Object, Point3, Polyline, RevisionCloud,
    RevisionCloudBoundary, Solid, Spline, Wipeout,
};

const EPS: f64 = 1e-6;

/// Axis-aligned 2D selection box in XY (`min_x <= max_x`, `min_y <= max_y`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StretchBox {
    pub(crate) min_x: f64,
    pub(crate) min_y: f64,
    pub(crate) max_x: f64,
    pub(crate) max_y: f64,
}

impl StretchBox {
    pub(crate) fn from_corners(a: [f64; 3], b: [f64; 3]) -> Result<Self, Error> {
        let min_x = a[0].min(b[0]);
        let max_x = a[0].max(b[0]);
        let min_y = a[1].min(b[1]);
        let max_y = a[1].max(b[1]);
        if (max_x - min_x) <= EPS || (max_y - min_y) <= EPS {
            return Err(Error::new(
                ErrorCode::BadValue,
                "stretch `box` must have positive width and height".to_owned(),
            ));
        }
        Ok(Self {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    }

    pub(crate) fn contains(&self, p: Point3) -> bool {
        p.x >= self.min_x - EPS
            && p.x <= self.max_x + EPS
            && p.y >= self.min_y - EPS
            && p.y <= self.max_y + EPS
    }
}

/// How `stretch` affects a single object.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub(crate) enum StretchAction {
    /// No vertex or anchor lay inside the box.
    Untouched,
    /// The entire object moves rigidly by `delta` (preserving template row
    /// bindings, attributes, and handles via `move_by` / `move_object`).
    MoveWhole,
    /// A subset of vertices moved; replace the object with this stretched geometry.
    Replace(Object),
}

fn shift(p: Point3, delta: [f64; 3]) -> Point3 {
    Point3::new(p.x + delta[0], p.y + delta[1], p.z + delta[2])
}

fn norm_deg(a: f64) -> f64 {
    let d = a.rem_euclid(360.0);
    if (d - 360.0).abs() <= 1e-12 || d.abs() <= 1e-12 {
        0.0
    } else {
        d
    }
}

fn arc_endpoint(arc: &Arc, deg: f64) -> Point3 {
    let (s, c) = deg.to_radians().sin_cos();
    Point3::new(
        arc.center.x + arc.radius * c,
        arc.center.y + arc.radius * s,
        arc.center.z,
    )
}

/// Determines how `object` behaves under AutoCAD-style `STRETCH` with `box_`
/// and displacement `delta`.
pub(crate) fn compute_stretch(
    object: &Object,
    box_: StretchBox,
    delta: [f64; 3],
) -> Result<StretchAction, Error> {
    if delta[0].abs() <= 1e-12 && delta[1].abs() <= 1e-12 && delta[2].abs() <= 1e-12 {
        return Ok(StretchAction::Untouched);
    }

    match object {
        Object::Line(line) => {
            let a_in = box_.contains(line.from);
            let b_in = box_.contains(line.to);
            match (a_in, b_in) {
                (false, false) => Ok(StretchAction::Untouched),
                (true, true) => Ok(StretchAction::MoveWhole),
                (true, false) => {
                    let from = shift(line.from, delta);
                    if (from.x - line.to.x).hypot(from.y - line.to.y) <= EPS
                        && (from.z - line.to.z).abs() <= EPS
                    {
                        return Err(Error::new(
                            ErrorCode::BadValue,
                            "stretching this line would collapse it to zero length".to_owned(),
                        ));
                    }
                    Ok(StretchAction::Replace(Object::Line(Line {
                        from,
                        to: line.to,
                        layer: line.layer.clone(),
                        properties: line.properties.clone(),
                    })))
                }
                (false, true) => {
                    let to = shift(line.to, delta);
                    if (line.from.x - to.x).hypot(line.from.y - to.y) <= EPS
                        && (line.from.z - to.z).abs() <= EPS
                    {
                        return Err(Error::new(
                            ErrorCode::BadValue,
                            "stretching this line would collapse it to zero length".to_owned(),
                        ));
                    }
                    Ok(StretchAction::Replace(Object::Line(Line {
                        from: line.from,
                        to,
                        layer: line.layer.clone(),
                        properties: line.properties.clone(),
                    })))
                }
            }
        }
        Object::Polyline(poly) => {
            let hits: Vec<bool> = poly.points.iter().map(|&p| box_.contains(p)).collect();
            let count = hits.iter().filter(|&&h| h).count();
            if count == 0 {
                Ok(StretchAction::Untouched)
            } else if count == poly.points.len() {
                Ok(StretchAction::MoveWhole)
            } else {
                let points: Vec<Point3> = poly
                    .points
                    .iter()
                    .zip(&hits)
                    .map(|(&p, &h)| if h { shift(p, delta) } else { p })
                    .collect();
                Ok(StretchAction::Replace(Object::Polyline(Polyline {
                    points,
                    bulges: poly.bulges.clone(),
                    widths: poly.widths.clone(),
                    width: poly.width,
                    closed: poly.closed,
                    layer: poly.layer.clone(),
                    properties: poly.properties.clone(),
                })))
            }
        }
        Object::Arc(arc) => {
            let p0 = arc_endpoint(arc, arc.start_angle);
            let p1 = arc_endpoint(arc, arc.end_angle);
            let c_in = box_.contains(arc.center);
            let s_in = box_.contains(p0);
            let e_in = box_.contains(p1);
            if c_in || (s_in && e_in) {
                return Ok(StretchAction::MoveWhole);
            }
            if !s_in && !e_in {
                return Ok(StretchAction::Untouched);
            }
            let new_p0 = if s_in { shift(p0, delta) } else { p0 };
            let new_p1 = if e_in { shift(p1, delta) } else { p1 };
            let chord = (new_p1.x - new_p0.x).hypot(new_p1.y - new_p0.y);
            if chord <= EPS {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "stretching this arc would collapse its endpoints together".to_owned(),
                ));
            }
            let mut sweep = (arc.end_angle - arc.start_angle).rem_euclid(360.0);
            if sweep <= 1e-9 {
                sweep = 360.0;
            }
            let bulge = (sweep.to_radians() * 0.25).tan();
            let Some(sa) = arc_of([new_p0.x, new_p0.y], [new_p1.x, new_p1.y], bulge) else {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "could not stretch arc geometry".to_owned(),
                ));
            };
            let start_angle = norm_deg(sa.start.to_degrees());
            let end_angle = norm_deg((sa.start + sa.sweep).to_degrees());
            Ok(StretchAction::Replace(Object::Arc(Arc {
                center: Point3::new(sa.center[0], sa.center[1], arc.center.z),
                radius: sa.radius,
                start_angle,
                end_angle,
                layer: arc.layer.clone(),
                properties: arc.properties.clone(),
            })))
        }
        Object::Circle(c) => {
            if box_.contains(c.center) {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Ellipse(e) => {
            if box_.contains(e.center) {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Donut(d) => {
            if box_.contains(d.center) {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Spline(sp) => {
            let pts = if !sp.fit_points.is_empty() {
                &sp.fit_points
            } else {
                &sp.control_points
            };
            let hits: Vec<bool> = pts.iter().map(|&p| box_.contains(p)).collect();
            let count = hits.iter().filter(|&&h| h).count();
            if count == 0 {
                Ok(StretchAction::Untouched)
            } else if count == pts.len() {
                Ok(StretchAction::MoveWhole)
            } else if !sp.fit_points.is_empty() {
                let fit_points = sp
                    .fit_points
                    .iter()
                    .zip(&hits)
                    .map(|(&p, &h)| if h { shift(p, delta) } else { p })
                    .collect();
                Ok(StretchAction::Replace(Object::Spline(Spline {
                    fit_points,
                    control_points: Vec::new(),
                    degree: sp.degree,
                    layer: sp.layer.clone(),
                    properties: sp.properties.clone(),
                })))
            } else {
                let control_points = sp
                    .control_points
                    .iter()
                    .zip(&hits)
                    .map(|(&p, &h)| if h { shift(p, delta) } else { p })
                    .collect();
                Ok(StretchAction::Replace(Object::Spline(Spline {
                    fit_points: Vec::new(),
                    control_points,
                    degree: sp.degree,
                    layer: sp.layer.clone(),
                    properties: sp.properties.clone(),
                })))
            }
        }
        Object::Solid(s) => {
            let hits: Vec<bool> = s.corners.iter().map(|&p| box_.contains(p)).collect();
            let count = hits.iter().filter(|&&h| h).count();
            if count == 0 {
                Ok(StretchAction::Untouched)
            } else if count == s.corners.len() {
                Ok(StretchAction::MoveWhole)
            } else {
                let corners = s
                    .corners
                    .iter()
                    .zip(&hits)
                    .map(|(&p, &h)| if h { shift(p, delta) } else { p })
                    .collect();
                Ok(StretchAction::Replace(Object::Solid(Solid {
                    corners,
                    layer: s.layer.clone(),
                    properties: s.properties.clone(),
                })))
            }
        }
        Object::Wipeout(w) => {
            let hits: Vec<bool> = w.points.iter().map(|&p| box_.contains(p)).collect();
            let count = hits.iter().filter(|&&h| h).count();
            if count == 0 {
                Ok(StretchAction::Untouched)
            } else if count == w.points.len() {
                Ok(StretchAction::MoveWhole)
            } else {
                let points = w
                    .points
                    .iter()
                    .zip(&hits)
                    .map(|(&p, &h)| if h { shift(p, delta) } else { p })
                    .collect();
                Ok(StretchAction::Replace(Object::Wipeout(Wipeout {
                    points,
                    layer: w.layer.clone(),
                    properties: w.properties.clone(),
                })))
            }
        }
        Object::RevisionCloud(rc) => match &rc.boundary {
            RevisionCloudBoundary::Rectangle { from, to } => {
                let corners = [
                    *from,
                    Point3::new(to.x, from.y, from.z),
                    *to,
                    Point3::new(from.x, to.y, from.z),
                ];
                let hits: Vec<bool> = corners.iter().map(|&p| box_.contains(p)).collect();
                let count = hits.iter().filter(|&&h| h).count();
                if count == 0 {
                    Ok(StretchAction::Untouched)
                } else if count == 4 {
                    Ok(StretchAction::MoveWhole)
                } else {
                    let points: Vec<Point3> = corners
                        .iter()
                        .zip(&hits)
                        .map(|(&p, &h)| if h { shift(p, delta) } else { p })
                        .collect();
                    Ok(StretchAction::Replace(Object::RevisionCloud(
                        RevisionCloud {
                            boundary: RevisionCloudBoundary::Polygon(points),
                            arc_length: rc.arc_length,
                            style: rc.style,
                            reverse: rc.reverse,
                            layer: rc.layer.clone(),
                            properties: rc.properties.clone(),
                        },
                    )))
                }
            }
            RevisionCloudBoundary::Polygon(points) => {
                let hits: Vec<bool> = points.iter().map(|&p| box_.contains(p)).collect();
                let count = hits.iter().filter(|&&h| h).count();
                if count == 0 {
                    Ok(StretchAction::Untouched)
                } else if count == points.len() {
                    Ok(StretchAction::MoveWhole)
                } else {
                    let new_pts = points
                        .iter()
                        .zip(&hits)
                        .map(|(&p, &h)| if h { shift(p, delta) } else { p })
                        .collect();
                    Ok(StretchAction::Replace(Object::RevisionCloud(
                        RevisionCloud {
                            boundary: RevisionCloudBoundary::Polygon(new_pts),
                            arc_length: rc.arc_length,
                            style: rc.style,
                            reverse: rc.reverse,
                            layer: rc.layer.clone(),
                            properties: rc.properties.clone(),
                        },
                    )))
                }
            }
        },
        // Rigid / annotation objects move whole when their insertion/anchor point
        // lies inside the stretch box (matching AutoCAD STRETCH).
        Object::Insert(i) => {
            if box_.contains(i.position) {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Text(t) => {
            if box_.contains(t.position) {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::MText(t) => {
            if box_.contains(t.position) {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Table(t) => {
            if box_.contains(t.position) {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Leader(l) => {
            if l.points.iter().all(|&p| box_.contains(p)) {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::MLeader(m) => {
            let all_leaders = m
                .leaders
                .iter()
                .flat_map(|pts| pts.iter())
                .all(|&p| box_.contains(p));
            let has_points = m.leaders.iter().any(|pts| !pts.is_empty());
            if has_points && all_leaders {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Dimension(d) => {
            let inside = match &d.measured {
                Measured::Linear { points, .. } | Measured::Aligned { points } => {
                    box_.contains(points[0]) && box_.contains(points[1])
                }
                Measured::Radius { center, point } | Measured::Diameter { center, point } => {
                    box_.contains(*center) && box_.contains(*point)
                }
                Measured::Angular { lines } => lines.iter().flatten().all(|&p| box_.contains(p)),
                Measured::ThreePointAngular { center, points } => {
                    box_.contains(*center) && box_.contains(points[0]) && box_.contains(points[1])
                }
                Measured::Ordinate { feature, .. } => box_.contains(*feature),
            };
            if inside {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Hatch(h) => {
            let mut any = false;
            let mut all = true;
            for lp in &h.boundary {
                match lp {
                    HatchLoop::Polyline { points } => {
                        for &p in points {
                            any = true;
                            if !box_.contains(p) {
                                all = false;
                            }
                        }
                    }
                    HatchLoop::Edges(edges) => {
                        for edge in edges {
                            match *edge {
                                HatchEdge::Line { from, to } => {
                                    any = true;
                                    if !box_.contains(from) || !box_.contains(to) {
                                        all = false;
                                    }
                                }
                                HatchEdge::Arc { center, .. } => {
                                    any = true;
                                    if !box_.contains(center) {
                                        all = false;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if any && all {
                Ok(StretchAction::MoveWhole)
            } else {
                Ok(StretchAction::Untouched)
            }
        }
        Object::Viewport(_) | Object::AttributeDefinition(_) => Ok(StretchAction::Untouched),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Insert, Properties};

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
    }

    #[test]
    fn stretch_line_polyline_arc_and_rigid_insert() {
        let b = StretchBox::from_corners([40.0, 0.0, 0.0], [100.0, 100.0, 0.0]).unwrap();
        let d = [25.0, 0.0, 0.0];

        // Wire crossing into the stretch box: left end stays at 10, right end moves 60 -> 85!
        let wire = Object::Line(Line {
            from: Point3::new(10.0, 50.0, 0.0),
            to: Point3::new(60.0, 50.0, 0.0),
            layer: "wires".to_owned(),
            properties: Properties::default(),
        });
        let StretchAction::Replace(Object::Line(stretched)) = compute_stretch(&wire, b, d).unwrap()
        else {
            panic!()
        };
        close(stretched.from.x, 10.0);
        close(stretched.to.x, 85.0);

        // Enclosure box (closed polyline 0..60 x 10..90): right edge moves 60 -> 85, left stays at 0!
        let rect = Object::Polyline(Polyline::through(
            vec![
                Point3::new(0.0, 10.0, 0.0),
                Point3::new(60.0, 10.0, 0.0),
                Point3::new(60.0, 90.0, 0.0),
                Point3::new(0.0, 90.0, 0.0),
            ],
            true,
            "enclosure",
        ));
        let StretchAction::Replace(Object::Polyline(p)) = compute_stretch(&rect, b, d).unwrap()
        else {
            panic!()
        };
        close(p.points[0].x, 0.0);
        close(p.points[1].x, 85.0);
        close(p.points[2].x, 85.0);
        close(p.points[3].x, 0.0);

        // Device insert inside box moves whole!
        let dev = Object::Insert(Insert {
            block: "DEVICE".to_owned(),
            position: Point3::new(60.0, 50.0, 0.0),
            scale: 1.0,
            rotation: 0.0,
            mirrored: false,
            readable_attributes: false,
            attributes: Vec::new(),
            state: std::collections::BTreeMap::new(),
            evaluated_block: None,
            name: None,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        assert!(matches!(
            compute_stretch(&dev, b, d).unwrap(),
            StretchAction::MoveWhole
        ));

        // Arc with only end_angle endpoint inside box stretches smoothly keeping sweep.
        let arc = Object::Arc(Arc {
            center: Point3::new(30.0, 50.0, 0.0),
            radius: 20.0,
            start_angle: 180.0, // (10, 50) outside
            end_angle: 0.0,     // (50, 50) inside
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let StretchAction::Replace(Object::Arc(sa)) = compute_stretch(&arc, b, d).unwrap() else {
            panic!()
        };
        // Endpoints are now (10, 50) and (75, 50) with 180-deg semicircle -> radius = 32.5, center x = 42.5.
        close(sa.center.x, 42.5);
        close(sa.radius, 32.5);
    }
}
