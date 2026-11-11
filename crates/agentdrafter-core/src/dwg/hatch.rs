//! Hatches through the seam: AgentDrafter's hatch to the DWG library's and
//! back. This is part of the seam in [`super`], the only code that names
//! acadrust.
//!
//! A DWG keeps a hatch's pattern lines as they are drawn: with the hatch's
//! scale and angle already applied, and each line's step to the next as an
//! offset in the hatch's plane rather than along and across the line, as a
//! `.pat` file states it. The core's model keeps the pattern as defined, so
//! writing applies the scale and angle and reading takes them off again.

use acadrust::entities::{
    BoundaryEdge, BoundaryPath, BoundaryPathFlags, CircularArcEdge, Hatch as DwgHatch,
    HatchPattern as DwgHatchPattern, HatchPatternLine as DwgHatchPatternLine, HatchPatternType,
    HatchStyleType, LineEdge, PolylineEdge,
};
use acadrust::types::{Vector2, Vector3};

use crate::description::normalize_degrees;
use crate::model::hatch::walk;
use crate::model::{Hatch, HatchEdge, HatchLoop, HatchPattern, HatchPatternLine, Point3, SOLID};

/// The DWG hatch for one of ours: not associative, with AutoCAD's normal
/// island detection, and every loop marked external, as for a hatch drawn
/// by selecting objects.
pub(super) fn dwg_hatch(hatch: &Hatch) -> DwgHatch {
    let mut dwg_hatch = if hatch.is_solid() {
        DwgHatch::solid()
    } else {
        let mut pattern = DwgHatchPattern::new(hatch.pattern.name.clone());
        pattern.lines = hatch
            .pattern
            .lines
            .iter()
            .map(|line| dwg_pattern_line(line, hatch.scale, hatch.angle))
            .collect();
        let mut patterned = DwgHatch::with_pattern(pattern);
        // A pattern the sheet defines is a custom pattern to AutoCAD: not
        // one of its own, and carried whole by the hatch.
        patterned.pattern_type = HatchPatternType::Custom;
        patterned.pattern_angle = hatch.angle.to_radians();
        patterned.pattern_scale = hatch.scale;
        patterned
    };
    dwg_hatch.elevation = elevation(hatch);
    dwg_hatch.normal = Vector3::UNIT_Z;
    dwg_hatch.style = HatchStyleType::Normal;
    dwg_hatch.paths = hatch.boundary.iter().map(dwg_loop).collect();
    dwg_hatch
}

/// The `z` the hatch's boundary lies at: its first point's.
fn elevation(hatch: &Hatch) -> f64 {
    match hatch.boundary.first() {
        Some(HatchLoop::Polyline { points }) => points.first().map_or(0.0, |p| p.z),
        Some(HatchLoop::Edges(edges)) => edges.first().map_or(0.0, |e| e.ends().0.z),
        None => 0.0,
    }
}

/// One pattern line as the DWG holds it: turned by the hatch's `angle` and
/// enlarged by its `scale`, with its step to the next line in the plane.
fn dwg_pattern_line(line: &HatchPatternLine, scale: f64, angle: f64) -> DwgHatchPatternLine {
    let direction = normalize_degrees(line.angle + angle);
    let [x, y] = turn(line.origin, angle);
    let [dx, dy] = turn(line.delta, direction);
    DwgHatchPatternLine {
        angle: direction.to_radians(),
        base_point: Vector2::new(x * scale, y * scale),
        offset: Vector2::new(dx * scale, dy * scale),
        dash_lengths: line.dashes.iter().map(|d| d * scale).collect(),
    }
}

fn dwg_loop(hatch_loop: &HatchLoop) -> BoundaryPath {
    let mut path = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
    match hatch_loop {
        HatchLoop::Polyline { points } => {
            // Adding a polyline marks the path as a polyline path.
            path.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(
                points.iter().map(|&p| flat(p)).collect(),
                true,
            )));
        }
        HatchLoop::Edges(edges) => {
            // A sheet description's loops always join up; one that does not
            // is written as it stands, every edge forwards.
            let backwards = walk(edges).unwrap_or_else(|_| vec![false; edges.len()]);
            for (edge, backwards) in edges.iter().zip(backwards) {
                path.add_edge(dwg_edge(edge, backwards));
            }
        }
    }
    path
}

/// One edge of a loop, written in the direction the loop walks it. An arc
/// walked backwards runs clockwise from its end angle to its start angle;
/// the DWG keeps a clockwise arc's angles negated.
fn dwg_edge(edge: &HatchEdge, backwards: bool) -> BoundaryEdge {
    match *edge {
        HatchEdge::Line { from, to } => BoundaryEdge::Line(LineEdge {
            start: flat(from),
            end: flat(to),
        }),
        HatchEdge::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        } => {
            let (start, end) = if backwards {
                (
                    normalize_degrees(-end_angle),
                    normalize_degrees(-start_angle),
                )
            } else {
                (start_angle, end_angle)
            };
            BoundaryEdge::CircularArc(CircularArcEdge {
                center: flat(center),
                radius,
                start_angle: start.to_radians(),
                end_angle: end.to_radians(),
                counter_clockwise: !backwards,
            })
        }
    }
}

/// One of our hatches, if the DWG hatch has a model in the core: flat, not
/// associative, filled solid or with a custom pattern of single lines, with
/// normal island detection and no gradient, bounded by closed polylines
/// without arc segments or by loops of lines and arcs. Seed points, which
/// change nothing that is drawn, are not read.
pub(super) fn read_hatch(dwg_hatch: &DwgHatch, layer: String) -> Option<Hatch> {
    if dwg_hatch.is_mpolygon
        || dwg_hatch.normal != Vector3::UNIT_Z
        || dwg_hatch.gradient_color.enabled
        || dwg_hatch.is_associative
        || dwg_hatch.style != HatchStyleType::Normal
        || dwg_hatch.paths.is_empty()
    {
        return None;
    }
    let (pattern, scale, angle) = if dwg_hatch.is_solid {
        if dwg_hatch.pattern.name != SOLID {
            return None;
        }
        (HatchPattern::solid(), 1.0, 0.0)
    } else {
        let scale = dwg_hatch.pattern_scale;
        if dwg_hatch.pattern_type != HatchPatternType::Custom
            || dwg_hatch.is_double
            || dwg_hatch.pattern.name.eq_ignore_ascii_case(SOLID)
            || dwg_hatch.pattern.lines.is_empty()
            || !(scale.is_finite() && scale > 0.0)
        {
            return None;
        }
        let angle = normalize_degrees(dwg_hatch.pattern_angle.to_degrees());
        let lines = dwg_hatch
            .pattern
            .lines
            .iter()
            .map(|line| read_pattern_line(line, scale, angle))
            .collect();
        let pattern = HatchPattern {
            name: dwg_hatch.pattern.name.clone(),
            lines,
        };
        (pattern, scale, angle)
    };
    let boundary = dwg_hatch
        .paths
        .iter()
        .map(|path| read_loop(path, dwg_hatch.elevation))
        .collect::<Option<Vec<_>>>()?;
    Some(Hatch {
        pattern,
        scale,
        angle,
        boundary,
        layer,
    })
}

/// A pattern line as defined, with the hatch's scale and angle taken off.
fn read_pattern_line(line: &DwgHatchPatternLine, scale: f64, angle: f64) -> HatchPatternLine {
    let direction = normalize_degrees(line.angle.to_degrees());
    let [x, y] = turn([line.base_point.x, line.base_point.y], -angle);
    let [dx, dy] = turn([line.offset.x, line.offset.y], -direction);
    HatchPatternLine {
        angle: normalize_degrees(direction - angle),
        origin: [x / scale + 0.0, y / scale + 0.0],
        delta: [dx / scale + 0.0, dy / scale + 0.0],
        dashes: line.dash_lengths.iter().map(|d| d / scale + 0.0).collect(),
    }
}

fn read_loop(path: &BoundaryPath, elevation: f64) -> Option<HatchLoop> {
    if path.flags.is_polyline() {
        let [BoundaryEdge::Polyline(polyline)] = path.edges.as_slice() else {
            return None;
        };
        // A vertex's z holds its bulge: an arc segment the core has no model for.
        if !polyline.is_closed || polyline.vertices.iter().any(|v| v.z != 0.0) {
            return None;
        }
        let points = polyline
            .vertices
            .iter()
            .map(|v| Point3::new(v.x, v.y, elevation))
            .collect();
        return Some(HatchLoop::Polyline { points });
    }
    path.edges
        .iter()
        .map(|edge| read_edge(edge, elevation))
        .collect::<Option<Vec<_>>>()
        .map(HatchLoop::Edges)
}

fn read_edge(edge: &BoundaryEdge, elevation: f64) -> Option<HatchEdge> {
    let point = |v: Vector2| Point3::new(v.x, v.y, elevation);
    match edge {
        BoundaryEdge::Line(line) => Some(HatchEdge::Line {
            from: point(line.start),
            to: point(line.end),
        }),
        BoundaryEdge::CircularArc(arc) => {
            // A clockwise arc's angles are kept negated: it runs clockwise
            // from -start to -end, so anticlockwise from -end to -start.
            let (start, end) = if arc.counter_clockwise {
                (arc.start_angle, arc.end_angle)
            } else {
                (-arc.end_angle, -arc.start_angle)
            };
            let start_angle = normalize_degrees(start.to_degrees());
            let end_angle = normalize_degrees(end.to_degrees());
            // Equal angles are a whole circle, which the core's arcs cannot say.
            let positive = arc.radius > 0.0 && arc.radius.is_finite();
            if !positive || start_angle == end_angle {
                return None;
            }
            Some(HatchEdge::Arc {
                center: point(arc.center),
                radius: arc.radius,
                start_angle,
                end_angle,
            })
        }
        _ => None,
    }
}

fn flat(p: Point3) -> Vector2 {
    Vector2::new(p.x, p.y)
}

/// `[x, y]` turned anticlockwise by `degrees`. Quarter turns are exact, so
/// a pattern line at 0°, 90°, 180° or 270° keeps its lengths bit for bit.
fn turn([x, y]: [f64; 2], degrees: f64) -> [f64; 2] {
    let quarter_turns = degrees.rem_euclid(360.0) / 90.0;
    let (sin, cos) = if quarter_turns.fract() == 0.0 {
        match quarter_turns as u8 {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        }
    } else {
        degrees.to_radians().sin_cos()
    };
    [x * cos - y * sin, x * sin + y * cos]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(start_angle: f64, end_angle: f64) -> HatchEdge {
        HatchEdge::Arc {
            center: Point3::new(5.0, -2.0, 0.0),
            radius: 3.0,
            start_angle,
            end_angle,
        }
    }

    /// An arc walked backwards is written clockwise with its angles negated,
    /// as the DWG keeps it (the clockwise arc from 90° to 270° is stored as
    /// 270° and 90°), and reads back as the same anticlockwise arc.
    #[test]
    fn an_arc_walked_either_way_reads_back_the_same() {
        let BoundaryEdge::CircularArc(written) = dwg_edge(&arc(270.0, 90.0), true) else {
            panic!("an arc is written as an arc");
        };
        assert!(!written.counter_clockwise);
        assert!((written.start_angle - 270f64.to_radians()).abs() < 1e-12);
        assert!((written.end_angle - 90f64.to_radians()).abs() < 1e-12);
        for (start, end) in [(270.0, 90.0), (30.0, 300.5), (350.0, 10.0)] {
            for backwards in [false, true] {
                let back = read_edge(&dwg_edge(&arc(start, end), backwards), 0.0).unwrap();
                let HatchEdge::Arc {
                    start_angle,
                    end_angle,
                    ..
                } = back
                else {
                    panic!("{back:?}");
                };
                assert!((start_angle - start).abs() < 1e-9, "{start} {backwards}");
                assert!((end_angle - end).abs() < 1e-9, "{end} {backwards}");
            }
        }
    }

    /// The DWG keeps a pattern line turned and enlarged, with its step to
    /// the next line in the hatch's plane; reading takes both off again.
    #[test]
    fn a_pattern_line_is_written_as_drawn_and_read_as_defined() {
        let line = HatchPatternLine {
            angle: 45.0,
            origin: [1.0, 2.0],
            delta: [0.0, 3.175],
            dashes: vec![2.0, -1.0, 0.0, -1.0],
        };
        let at_one = dwg_pattern_line(&line, 1.0, 0.0);
        let half = 3.175 / 2f64.sqrt();
        assert!((at_one.offset.x + half).abs() < 1e-12, "{at_one:?}");
        assert!((at_one.offset.y - half).abs() < 1e-12, "{at_one:?}");
        assert_eq!(at_one.base_point, Vector2::new(1.0, 2.0));

        let drawn = dwg_pattern_line(&line, 2.0, 90.0);
        assert!((drawn.angle - 135f64.to_radians()).abs() < 1e-12);
        assert_eq!(drawn.base_point, Vector2::new(-4.0, 2.0));
        assert_eq!(drawn.dash_lengths, vec![4.0, -2.0, 0.0, -2.0]);
        let back = read_pattern_line(&drawn, 2.0, 90.0);
        assert!((back.angle - 45.0).abs() < 1e-9, "{back:?}");
        assert_eq!(back.origin, [1.0, 2.0]);
        assert!(back.delta[0].abs() < 1e-12 && (back.delta[1] - 3.175).abs() < 1e-12);
        assert_eq!(back.dashes, line.dashes);
    }

    #[test]
    fn quarter_turns_are_exact() {
        assert_eq!(turn([2.0, 3.0], 90.0), [-3.0, 2.0]);
        assert_eq!(turn([2.0, 3.0], -90.0), [3.0, -2.0]);
        assert_eq!(turn([2.0, 3.0], 540.0), [-2.0, -3.0]);
        assert_eq!(turn([2.0, 3.0], 0.0), [2.0, 3.0]);
    }

    /// A hatch the core cannot say is counted, not read wrongly.
    #[test]
    fn a_hatch_the_core_has_no_model_for_is_not_read() {
        let square = || {
            let mut path = BoundaryPath::with_flags(BoundaryPathFlags::EXTERNAL);
            path.add_edge(BoundaryEdge::Polyline(PolylineEdge::new(
                vec![
                    Vector2::new(0.0, 0.0),
                    Vector2::new(1.0, 0.0),
                    Vector2::new(1.0, 1.0),
                ],
                true,
            )));
            path
        };
        let mut solid = DwgHatch::solid();
        solid.add_path(square());
        assert!(read_hatch(&solid, "0".to_owned()).is_some());

        let mut associative = solid.clone();
        associative.is_associative = true;
        let mut gradient = solid.clone();
        gradient.gradient_color.enabled = true;
        let mut outer_only = solid.clone();
        outer_only.style = HatchStyleType::Outer;
        let mut bulged = solid.clone();
        if let BoundaryEdge::Polyline(polyline) = &mut bulged.paths[0].edges[0] {
            polyline.vertices[1].z = 0.5;
        }
        let mut predefined = DwgHatch::with_pattern(DwgHatchPattern::new("ANSI31"));
        predefined.pattern.lines.push(DwgHatchPatternLine {
            angle: 0.0,
            base_point: Vector2::ZERO,
            offset: Vector2::new(0.0, 1.0),
            dash_lengths: Vec::new(),
        });
        predefined.add_path(square());
        let mut custom = predefined.clone();
        custom.pattern_type = HatchPatternType::Custom;
        assert!(read_hatch(&custom, "0".to_owned()).is_some());
        for (what, hatch) in [
            ("associative", associative),
            ("gradient", gradient),
            ("outermost only", outer_only),
            ("arc segments", bulged),
            ("AutoCAD's own pattern", predefined),
        ] {
            assert!(read_hatch(&hatch, "0".to_owned()).is_none(), "{what}");
        }
    }
}
