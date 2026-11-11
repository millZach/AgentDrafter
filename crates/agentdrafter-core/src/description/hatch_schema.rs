//! The hatch parts of the sheet description's schema: hatch patterns and
//! hatches, built from the same tables `hatch.rs` checks keys against.
//!
//! What JSON Schema cannot see is left to the reader, as everywhere in the
//! schema: a scale above 0 when it is written as a string, a `delta` whose
//! distance across is 0, dashes that hold a dash and a gap, a boundary that
//! lies flat and joins up, and whether the pattern a hatch names is defined.

use serde_json::{json, Map, Value};

use super::hatch::{
    BOUNDARY_POLYLINE_KEYS, EDGE_TYPES, FEWEST_BOUNDARY_POINTS, FEWEST_EDGES, HATCH_PATTERN_KEYS,
    LONGEST_PATTERN_NAME, LOOP_TYPES, PATTERN_LINE_KEYS, RESERVED_PATTERN_NAMES,
};
use super::layers::FORBIDDEN_IN_NAMES;
use super::schema::{object_def, NUMBER};
use crate::model::SOLID;

/// The schema of the top-level `hatch_patterns` key of a sheet or a block
/// library: `whose` they are.
pub(super) fn hatch_patterns_property(whose: &str) -> Value {
    json!({
        "description": format!(
            "The {whose}'s hatch patterns, from name to pattern. `{SOLID}`, AutoCAD's solid hatch, \
             needs no definition, and neither it nor `_USER` can be defined."
        ),
        "type": "object",
        "propertyNames": { "$ref": "#/$defs/hatch_pattern_name" },
        "additionalProperties": { "$ref": "#/$defs/hatch_pattern" },
    })
}

/// Adds the definitions hatches and hatch patterns use to `defs`.
pub(super) fn add_defs(defs: &mut Map<String, Value>) {
    let forbidden = FORBIDDEN_IN_NAMES.replace('\\', "\\\\");
    let reserved = RESERVED_PATTERN_NAMES
        .map(|name| {
            name.chars()
                .map(|c| {
                    if c.is_ascii_alphabetic() {
                        format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
                    } else {
                        c.to_string()
                    }
                })
                .collect::<String>()
        })
        .join("|");
    defs.insert(
        "hatch_pattern_name".into(),
        json!({
            "description": format!(
                "A hatch pattern name: 1 to {LONGEST_PATTERN_NAME} characters in capitals, as \
                 AutoCAD keeps it, with no spaces, holding none of {FORBIDDEN_IN_NAMES} and no \
                 control characters, and not {}.",
                RESERVED_PATTERN_NAMES.join(" or ")
            ),
            "type": "string",
            "minLength": 1,
            "maxLength": LONGEST_PATTERN_NAME,
            "pattern": format!("^[^{forbidden}\\x00-\\x1f\\s\\p{{Lowercase}}]+$"),
            "not": { "pattern": format!("^({reserved})$") },
        }),
    );
    defs.insert(
        "vector2".into(),
        json!({
            "description": "Two lengths [x, y] in the plane of a hatch.",
            "type": "array",
            "items": { "$ref": "#/$defs/length" },
            "minItems": 2,
            "maxItems": 2,
        }),
    );
    let [lines_key] = HATCH_PATTERN_KEYS;
    defs.insert(
        "hatch_pattern".into(),
        json!({
            "description": "A hatch pattern: families of parallel lines, as in an AutoCAD .pat \
                            file. Its lengths are in the declared unit and are enlarged by each \
                            hatch's scale.",
            "type": "object",
            "required": [lines_key],
            "additionalProperties": false,
            "properties": {
                lines_key: {
                    "description": "The pattern's lines, at least one.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/hatch_pattern_line" },
                    "minItems": 1,
                },
            },
        }),
    );
    let [angle_key, origin_key, delta_key, dashes_key] = PATTERN_LINE_KEYS;
    defs.insert(
        "hatch_pattern_line".into(),
        json!({
            "description": "One family of parallel lines in a hatch pattern, as one line of an \
                            AutoCAD .pat file states it.",
            "type": "object",
            "required": [delta_key],
            "additionalProperties": false,
            "properties": {
                angle_key: {
                    "$ref": "#/$defs/angle",
                    "description": "The direction of the lines: degrees anticlockwise from the X \
                                    axis. Without it, 0.",
                },
                origin_key: {
                    "$ref": "#/$defs/vector2",
                    "description": "A point one of the lines passes through. Without it, [0, 0].",
                },
                delta_key: {
                    "$ref": "#/$defs/vector2",
                    "description": "From one line to the next, as [along, across]: along runs in \
                                    the lines' direction and shifts where the dashes start; \
                                    across is the distance between neighbouring lines and is \
                                    not 0.",
                },
                dashes_key: {
                    "description": "The dashes, gaps and dots each line repeats: a positive \
                                    length is a dash, a negative one a gap, 0 a dot. At least \
                                    one dash or dot and at least one gap, in any order. Without \
                                    it, the lines are continuous.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/length" },
                    "minItems": 2,
                },
            },
        }),
    );
    let mut hatch = object_def(
        "An area filled solid or with a pattern, inside a boundary of closed loops, in the plane \
         parallel to XY at the z its boundary's points share. A point inside an odd number of \
         loops is filled, as AutoCAD's normal island detection reads them. It draws no outline.",
        &["pattern", "boundary"],
        json!({
            "pattern": {
                "description": format!(
                    "`{SOLID}` for a solid hatch, or the name of a pattern under `hatch_patterns`, \
                     written exactly as it is there."
                ),
                "type": ["string", "number", "boolean", "null"],
            },
            "scale": {
                "description": format!(
                    "How much the pattern is enlarged, a number more than 0 with no unit. \
                     Without it, 1. Not given with `{SOLID}`."
                ),
                "anyOf": [
                    { "type": "number", "exclusiveMinimum": 0 },
                    { "type": "string", "pattern": format!("^{NUMBER}$") },
                ],
            },
            "angle": {
                "$ref": "#/$defs/angle",
                "description": format!(
                    "How far the pattern is turned: degrees anticlockwise. Without it, 0. Not \
                     given with `{SOLID}`."
                ),
            },
            "boundary": {
                "description": "The closed loops that bound the area, at least one. Every point \
                                of every loop has the same z.",
                "type": "array",
                "items": { "$ref": "#/$defs/hatch_loop" },
                "minItems": 1,
            },
        }),
    );
    hatch["if"] =
        json!({ "required": ["pattern"], "properties": { "pattern": { "const": SOLID } } });
    hatch["then"] =
        json!({ "not": { "anyOf": [{ "required": ["scale"] }, { "required": ["angle"] }] } });
    defs.insert("hatch".into(), hatch);

    let [points_key] = BOUNDARY_POLYLINE_KEYS;
    let [polyline_key, edges_key] = LOOP_TYPES;
    defs.insert(
        "hatch_loop".into(),
        json!({
            "description": "One closed loop of a hatch's boundary: a mapping with one key, \
                            polyline or edges.",
            "oneOf": [
                {
                    "type": "object",
                    "required": [polyline_key],
                    "additionalProperties": false,
                    "properties": {
                        polyline_key: {
                            "description": "Straight segments through the points in order, \
                                            always closed back to the first.",
                            "type": "object",
                            "required": [points_key],
                            "additionalProperties": false,
                            "properties": {
                                points_key: {
                                    "type": "array",
                                    "items": { "$ref": "#/$defs/point" },
                                    "minItems": FEWEST_BOUNDARY_POINTS,
                                },
                            },
                        },
                    },
                },
                {
                    "type": "object",
                    "required": [edges_key],
                    "additionalProperties": false,
                    "properties": {
                        edges_key: {
                            "description": "Lines and arcs joined end to end, the last ending \
                                            where the first starts. Each line starts where the \
                                            edge before it ends; an arc may be walked either way.",
                            "type": "array",
                            "items": { "$ref": "#/$defs/hatch_edge" },
                            "minItems": FEWEST_EDGES,
                        },
                    },
                },
            ],
        }),
    );
    let edges: Vec<Value> = EDGE_TYPES
        .iter()
        .map(|(kind, keys)| {
            let properties: Map<String, Value> = keys
                .iter()
                .map(|key| {
                    let value = match *key {
                        "from" | "to" | "center" => json!({ "$ref": "#/$defs/point" }),
                        "radius" => json!({ "$ref": "#/$defs/length", "description": "A length more than 0." }),
                        _ => json!({ "$ref": "#/$defs/angle" }),
                    };
                    ((*key).to_owned(), value)
                })
                .collect();
            json!({
                "type": "object",
                "required": [kind],
                "additionalProperties": false,
                "properties": {
                    *kind: {
                        "type": "object",
                        "required": keys,
                        "additionalProperties": false,
                        "properties": properties,
                    },
                },
            })
        })
        .collect();
    defs.insert(
        "hatch_edge".into(),
        json!({
            "description": "A line or an arc in a loop of a hatch's boundary. An arc runs \
                            anticlockwise from its start angle to its end angle.",
            "oneOf": edges,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::description::schema::sheet_description_schema;

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    fn sorted<'a>(keys: &[&'a str]) -> Vec<&'a str> {
        let mut keys = keys.to_vec();
        keys.sort_unstable();
        keys
    }

    /// The tables `hatch.rs` checks keys against are the ones the schema states.
    #[test]
    fn the_schema_and_the_reader_know_the_same_hatch_keys() {
        let schema = sheet_description_schema();
        let defs = &schema["$defs"];
        assert_eq!(
            keys(&defs["hatch_pattern"]["properties"]),
            sorted(&HATCH_PATTERN_KEYS)
        );
        assert_eq!(
            keys(&defs["hatch_pattern_line"]["properties"]),
            sorted(&PATTERN_LINE_KEYS)
        );
        let loops = defs["hatch_loop"]["oneOf"].as_array().unwrap();
        let loop_types: Vec<&str> = loops.iter().map(|l| keys(&l["properties"])[0]).collect();
        assert_eq!(loop_types, LOOP_TYPES);
        assert_eq!(
            keys(&loops[0]["properties"][LOOP_TYPES[0]]["properties"]),
            sorted(&BOUNDARY_POLYLINE_KEYS)
        );
        let edges = defs["hatch_edge"]["oneOf"].as_array().unwrap();
        assert_eq!(edges.len(), EDGE_TYPES.len());
        for (edge, (kind, edge_keys)) in edges.iter().zip(EDGE_TYPES) {
            assert_eq!(keys(&edge["properties"]), [kind]);
            assert_eq!(
                keys(&edge["properties"][kind]["properties"]),
                sorted(edge_keys)
            );
        }
    }
}
