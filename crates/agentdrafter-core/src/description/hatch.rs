//! Reads the sheet's hatch patterns and its hatches.
//!
//! A hatch pattern is defined under `hatch_patterns`, from its name to its
//! lines, the way a linetype is defined under `linetypes`. A hatch names
//! `SOLID` or one of those patterns, and states its boundary as closed
//! loops of polylines, lines and arcs.

use super::layers::named_entries;
use super::objects::{arc_angles, radius};
use super::strict_yaml::{index_path, key_path, Entry, Kind, Node};
use super::{
    angle, at, bad_value, choice, length, plain_number, point, sequence, unknown_key, wrong_type,
    Fields,
};
use crate::error::{Error, ErrorCode};
use crate::model::hatch::{walk, Break};
use crate::model::{Hatch, HatchEdge, HatchLoop, HatchPattern, HatchPatternLine, Point3, SOLID};
use crate::units::Unit;

/// The keys a hatch pattern takes.
pub(crate) const HATCH_PATTERN_KEYS: [&str; 1] = ["lines"];

/// The keys one line of a hatch pattern takes.
pub(crate) const PATTERN_LINE_KEYS: [&str; 4] = ["angle", "origin", "delta", "dashes"];

/// The keys a hatch takes.
pub(crate) const HATCH_KEYS: [&str; 9] = [
    "pattern",
    "scale",
    "angle",
    "boundary",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The kinds of loop a hatch's boundary holds.
pub(crate) const LOOP_TYPES: [&str; 2] = ["polyline", "edges"];

/// The keys a boundary polyline takes. It is always closed.
pub(crate) const BOUNDARY_POLYLINE_KEYS: [&str; 1] = ["points"];

/// The kinds of edge a loop of edges holds, and the keys each takes.
pub(crate) const EDGE_TYPES: [(&str, &[&str]); 2] = [
    ("line", &["from", "to"]),
    ("arc", &["center", "radius", "start_angle", "end_angle"]),
];

/// The names AutoCAD keeps for its own hatches: its solid hatch, and its
/// user-defined hatch of parallel lines.
pub(crate) const RESERVED_PATTERN_NAMES: [&str; 2] = [SOLID, "_USER"];

/// The longest hatch pattern name AutoCAD allows, in characters.
pub(crate) const LONGEST_PATTERN_NAME: usize = 34;

/// The fewest points a boundary polyline takes, to enclose an area.
pub(crate) const FEWEST_BOUNDARY_POINTS: usize = 3;

/// The fewest edges a loop of edges takes, to close.
pub(crate) const FEWEST_EDGES: usize = 2;

pub(super) fn hatch_patterns(node: &Node, units: Unit) -> Result<Vec<HatchPattern>, Error> {
    let entries = named_entries(node, "hatch_patterns", "hatch pattern")?;
    let mut patterns = Vec::with_capacity(entries.len());
    for entry in entries {
        check_pattern_name(entry)?;
        let path = key_path("hatch_patterns", &entry.key);
        let fields = Fields::of(&entry.value, &path, &HATCH_PATTERN_KEYS, "a hatch pattern")?;
        let lines_path = fields.path_of("lines");
        let lines_node = fields.require("lines")?;
        let items = sequence(lines_node, &lines_path, "a list of pattern lines")?;
        if items.is_empty() {
            return Err(at(
                Error::new(
                    ErrorCode::WrongType,
                    "a hatch pattern has at least one line; for a solid hatch use `SOLID`",
                ),
                &lines_path,
                lines_node.at,
            ));
        }
        let lines = items
            .iter()
            .enumerate()
            .map(|(i, item)| pattern_line(item, &index_path(&lines_path, i), units))
            .collect::<Result<_, _>>()?;
        patterns.push(HatchPattern {
            name: entry.key.clone(),
            lines,
        });
    }
    Ok(patterns)
}

/// Refuses a hatch pattern name AutoCAD keeps for itself or would not
/// accept, beyond what every name in a table must follow.
fn check_pattern_name(entry: &Entry) -> Result<(), Error> {
    let name = &entry.key;
    let problem = if let Some(reserved) = RESERVED_PATTERN_NAMES
        .iter()
        .find(|r| r.eq_ignore_ascii_case(name))
    {
        format!("`{name}` is AutoCAD's own hatch `{reserved}`; give yours another name")
    } else if name.chars().count() > LONGEST_PATTERN_NAME {
        format!("a hatch pattern name has at most {LONGEST_PATTERN_NAME} characters")
    } else if name.chars().any(char::is_whitespace) {
        "a hatch pattern name cannot hold a space".to_owned()
    } else if name.chars().any(char::is_lowercase) {
        format!(
            "a hatch pattern name is written in capitals, as AutoCAD keeps it: `{}`",
            name.to_uppercase()
        )
    } else {
        return Ok(());
    };
    Err(Error::new(ErrorCode::BadValue, problem)
        .at_field(key_path("hatch_patterns", name))
        .at_position(entry.key_at.line, entry.key_at.column))
}

fn pattern_line(node: &Node, path: &str, units: Unit) -> Result<HatchPatternLine, Error> {
    let fields = Fields::of(node, path, &PATTERN_LINE_KEYS, "a hatch pattern line")?;
    let angle = match fields.get("angle") {
        Some(node) => angle(node, &fields.path_of("angle"))?,
        None => 0.0,
    };
    let origin = match fields.get("origin") {
        Some(node) => pair(node, &fields.path_of("origin"), units)?,
        None => [0.0, 0.0],
    };
    let delta_node = fields.require("delta")?;
    let delta_path = fields.path_of("delta");
    let delta = pair(delta_node, &delta_path, units)?;
    if delta[1] == 0.0 {
        return Err(bad_value(
            delta_node,
            &delta_path,
            "the second length of `delta` is the distance between neighbouring lines, so it cannot be 0",
        ));
    }
    let dashes = match fields.get("dashes") {
        Some(node) => dashes(node, &fields.path_of("dashes"), units)?,
        None => Vec::new(),
    };
    Ok(HatchPatternLine {
        angle,
        origin,
        delta,
        dashes,
    })
}

/// Two lengths, `[x, y]`, in the plane of a hatch.
fn pair(node: &Node, path: &str, units: Unit) -> Result<[f64; 2], Error> {
    let items = sequence(node, path, "two lengths [x, y]")?;
    let [x, y] = items else {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                format!(
                    "expected two lengths [x, y] in the hatch's plane; found {}",
                    items.len()
                ),
            ),
            path,
            node.at,
        ));
    };
    Ok([
        length(x, &index_path(path, 0), units)? + 0.0,
        length(y, &index_path(path, 1), units)? + 0.0,
    ])
}

/// The dashes, gaps and dots of a pattern line: at least one dash or dot,
/// and at least one gap, so that the line is neither continuous nor empty.
fn dashes(node: &Node, path: &str, units: Unit) -> Result<Vec<f64>, Error> {
    let items = sequence(node, path, "a list of lengths")?;
    let dashes = items
        .iter()
        .enumerate()
        .map(|(i, item)| length(item, &index_path(path, i), units).map(|l| l + 0.0))
        .collect::<Result<Vec<f64>, _>>()?;
    if dashes.iter().any(|&d| d >= 0.0) && dashes.iter().any(|&d| d < 0.0) {
        return Ok(dashes);
    }
    Err(bad_value(
        node,
        path,
        "`dashes` holds at least one dash or dot (a positive length or 0) and at least one gap (a negative length); for a continuous line leave `dashes` out",
    ))
}

pub(super) fn hatch(
    fields: &Fields,
    units: Unit,
    layer: String,
    patterns: &[HatchPattern],
) -> Result<Hatch, Error> {
    let pattern = pattern(
        fields.require_text("pattern")?,
        &fields.path_of("pattern"),
        patterns,
    )?;
    let (scale, angle) = if pattern.is_solid() {
        for (key, verb) in [("scale", "scale"), ("angle", "turn")] {
            if let Some(node) = fields.get(key) {
                return Err(bad_value(
                    node,
                    &fields.path_of(key),
                    format!("a `{SOLID}` hatch has no pattern to {verb}; leave `{key}` out"),
                ));
            }
        }
        (1.0, 0.0)
    } else {
        let scale = match fields.get("scale") {
            Some(node) => scale(node, &fields.path_of("scale"))?,
            None => 1.0,
        };
        let angle = match fields.get("angle") {
            Some(node) => angle(node, &fields.path_of("angle"))?,
            None => 0.0,
        };
        (scale, angle)
    };
    let boundary = boundary(
        fields.require("boundary")?,
        &fields.path_of("boundary"),
        units,
    )?;
    Ok(Hatch {
        pattern,
        scale,
        angle,
        boundary,
        layer,
        properties: Default::default(),
    })
}

/// The pattern a hatch names: `SOLID` or one of the sheet's hatch patterns,
/// written exactly as it is defined.
fn pattern(node: &Node, path: &str, patterns: &[HatchPattern]) -> Result<HatchPattern, Error> {
    let mut choices = vec![SOLID];
    choices.extend(patterns.iter().map(|p| p.name.as_str()));
    let listed = format!("the sheet's hatch patterns: {}", choices.join(", "));
    let text = choice(node, path, "the name of a hatch pattern", &listed)?;
    if text == SOLID {
        return Ok(HatchPattern::solid());
    }
    if let Some(pattern) = patterns.iter().find(|p| p.name == text) {
        return Ok(pattern.clone());
    }
    Err(at(
        Error::new(
            ErrorCode::UnknownName,
            format!("unknown hatch pattern `{text}`; {listed}"),
        ),
        path,
        node.at,
    ))
}

/// A pattern's scale: a number more than 0, with no unit.
fn scale(node: &Node, path: &str) -> Result<f64, Error> {
    let scale = plain_number(node, path, "a scale, a number more than 0")?;
    if scale > 0.0 {
        Ok(scale)
    } else {
        Err(bad_value(node, path, "a hatch's scale is more than 0"))
    }
}

/// The `z` every point of one hatch's boundary shares: the first point's.
struct Plane {
    z: Option<f64>,
}

impl Plane {
    /// A point of the boundary, checked to lie in the plane of the first.
    fn point(&mut self, node: &Node, path: &str, units: Unit) -> Result<Point3, Error> {
        let p = point(node, path, units)?;
        match self.z {
            None => self.z = Some(p.z),
            Some(z) if z == p.z => {}
            Some(z) => {
                let Kind::Sequence(items) = &node.kind else {
                    unreachable!("the point was read as a list")
                };
                return Err(bad_value(
                    &items[2],
                    &index_path(path, 2),
                    format!(
                        "a hatch lies flat, so every point of its boundary has the z of the first, {z}; this one has {}",
                        p.z
                    ),
                ));
            }
        }
        Ok(p)
    }
}

fn boundary(node: &Node, path: &str, units: Unit) -> Result<Vec<HatchLoop>, Error> {
    let items = sequence(node, path, "a list of loops")?;
    if items.is_empty() {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                "a hatch's boundary has at least one loop",
            ),
            path,
            node.at,
        ));
    }
    let mut plane = Plane { z: None };
    items
        .iter()
        .enumerate()
        .map(|(i, item)| hatch_loop(item, &index_path(path, i), units, &mut plane))
        .collect()
}

/// One entry of a list whose items are mappings with one key, their kind,
/// such as a loop (`polyline` or `edges`) or an edge (`line` or `arc`).
/// `what` names the item and `kinds` lists the kinds it may be.
fn one_of<'a>(node: &'a Node, path: &str, what: &str, kinds: &[&str]) -> Result<&'a Entry, Error> {
    let shape = format!("{what}, a mapping with one key: {}", kinds.join(" or "));
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(node, path, &shape));
    };
    if let Some(entry) = entries.iter().find(|e| !kinds.contains(&e.key.as_str())) {
        return Err(unknown_key(
            entry,
            path,
            &format!("unknown key `{}`", entry.key),
            &format!("{what} is one of {}", kinds.join(", ")),
        ));
    }
    match entries.as_slice() {
        [entry] => Ok(entry),
        _ => Err(wrong_type(node, path, &shape)),
    }
}

fn hatch_loop(node: &Node, path: &str, units: Unit, plane: &mut Plane) -> Result<HatchLoop, Error> {
    let entry = one_of(node, path, "a loop", &LOOP_TYPES)?;
    let path = key_path(path, &entry.key);
    if entry.key == "polyline" {
        let fields = Fields::of(
            &entry.value,
            &path,
            &BOUNDARY_POLYLINE_KEYS,
            "a boundary polyline, which is always closed,",
        )?;
        let points_path = fields.path_of("points");
        let points_node = fields.require("points")?;
        let items = sequence(points_node, &points_path, "a list of points")?;
        if items.len() < FEWEST_BOUNDARY_POINTS {
            return Err(at(
                Error::new(
                    ErrorCode::WrongType,
                    format!(
                        "expected at least {FEWEST_BOUNDARY_POINTS} points, found {}",
                        items.len()
                    ),
                ),
                &points_path,
                points_node.at,
            ));
        }
        let points = items
            .iter()
            .enumerate()
            .map(|(i, item)| plane.point(item, &index_path(&points_path, i), units))
            .collect::<Result<_, _>>()?;
        Ok(HatchLoop::Polyline { points })
    } else {
        edges(&entry.value, &path, units, plane).map(HatchLoop::Edges)
    }
}

fn edges(node: &Node, path: &str, units: Unit, plane: &mut Plane) -> Result<Vec<HatchEdge>, Error> {
    let items = sequence(node, path, "a list of lines and arcs")?;
    if items.len() < FEWEST_EDGES {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                format!(
                    "a loop of edges closes, so it needs at least {FEWEST_EDGES} edges; found {}",
                    items.len()
                ),
            ),
            path,
            node.at,
        ));
    }
    let edges = items
        .iter()
        .enumerate()
        .map(|(i, item)| edge(item, &index_path(path, i), units, plane))
        .collect::<Result<Vec<_>, _>>()?;
    check_joined(&edges, items, path)?;
    Ok(edges)
}

fn edge(node: &Node, path: &str, units: Unit, plane: &mut Plane) -> Result<HatchEdge, Error> {
    let kinds = EDGE_TYPES.map(|(kind, _)| kind);
    let entry = one_of(node, path, "an edge", &kinds)?;
    let path = key_path(path, &entry.key);
    let keys = EDGE_TYPES
        .iter()
        .find(|(kind, _)| *kind == entry.key)
        .map(|(_, keys)| *keys)
        .unwrap_or_default();
    if entry.key == "line" {
        let fields = Fields::of(&entry.value, &path, keys, "a line in a boundary")?;
        let from = plane.point(fields.require("from")?, &fields.path_of("from"), units)?;
        let to_node = fields.require("to")?;
        let to = plane.point(to_node, &fields.path_of("to"), units)?;
        if from == to {
            return Err(bad_value(
                to_node,
                &fields.path_of("to"),
                "a line in a boundary has a length, so `to` differs from `from`",
            ));
        }
        Ok(HatchEdge::Line { from, to })
    } else {
        let fields = Fields::of(&entry.value, &path, keys, "an arc in a boundary")?;
        let center = plane.point(fields.require("center")?, &fields.path_of("center"), units)?;
        let radius = radius(&fields, units)?;
        let (start_angle, end_angle) =
            arc_angles(&fields, "a whole circle in a boundary is two arcs")?;
        Ok(HatchEdge::Arc {
            center,
            radius,
            start_angle,
            end_angle,
        })
    }
}

/// Refuses a loop whose edges do not join end to end and close, naming the
/// edge where the walk breaks and where the walk had reached.
fn check_joined(edges: &[HatchEdge], items: &[Node], path: &str) -> Result<(), Error> {
    let show = |p: &Point3| format!("[{}, {}, {}]", p.x, p.y, p.z);
    let (edge, message) = match walk(edges) {
        Ok(_) => return Ok(()),
        Err(Break::Open { end, start }) => (
            edges.len() - 1,
            format!(
                "a loop closes: its last edge ends at {}, but its first edge starts at {}",
                show(&end),
                show(&start)
            ),
        ),
        Err(Break::Gap { edge, reached }) => {
            let reached = reached.iter().map(show).collect::<Vec<_>>().join(" or ");
            let (a, b) = edges[edge].ends();
            let message = match edges[edge] {
                HatchEdge::Line { .. } => format!(
                    "each line in a loop starts where the edge before it ends, at {reached}; this one starts at {}",
                    show(&a)
                ),
                HatchEdge::Arc { .. } => format!(
                    "each arc in a loop starts or ends where the edge before it ends, at {reached}; this one runs from {} to {}",
                    show(&a),
                    show(&b)
                ),
            };
            (edge, message)
        }
    };
    Err(bad_value(&items[edge], &index_path(path, edge), message))
}
