//! The sheet description's schema, as a JSON Schema (draft 2020-12) an
//! agent can read before it writes a sheet description.
//!
//! The schema is built from the same tables the reader checks keys against:
//! the object types and their keys, the units, the colour names and the
//! lineweights. A test holds the two together, so a key cannot be added to
//! one and not the other.
//!
//! JSON Schema describes JSON values, and a YAML reader that guesses types
//! turns `100` into a number and `4 in` into a string, so a length here is a
//! number or a string that reads as one. The rules of the strict YAML subset
//! (no anchors, tags or directives, every key once) are not JSON Schema; the
//! top-level description states them.

use serde_json::{json, Map, Value};

use super::layers::{
    colour_names, COLOUR_NAMES, FORBIDDEN_IN_NAMES, LONGEST_DESCRIPTION, LONGEST_NAME,
    LONGEST_PATTERN,
};
use super::objects::{FEWEST_FIT_POINTS, FIT_DEGREE, HIGHEST_DEGREE, OBJECT_TYPES};
use super::SHEET_KEYS;
use crate::model::{Lineweight, CONTINUOUS, LAYER_ZERO, WHITE};
use crate::units::Unit;

/// A plain decimal number, as `docs/sheet-description.md` defines it.
const NUMBER: &str = "-?[0-9]+(\\.[0-9]+)?([eE][-+]?[0-9]+)?";

/// The sheet description's schema.
pub fn sheet_description_schema() -> Value {
    let units = Unit::ALL.map(Unit::symbol);
    let unit_pattern = units.join("|");
    let lineweights: Vec<f64> = Lineweight::ALL.iter().map(|w| w.millimetres()).collect();
    let lineweight_pattern = Lineweight::ALL
        .map(|w| w.to_string().replace('.', "\\."))
        .join("|");
    let colours = COLOUR_NAMES.map(|(name, _)| name);
    let objects: Vec<Value> = OBJECT_TYPES
        .iter()
        .map(|(name, _)| {
            json!({
                "type": "object",
                "required": [name],
                "additionalProperties": false,
                "properties": { *name: { "$ref": format!("#/$defs/{name}") } },
            })
        })
        .collect();

    let mut defs = Map::new();
    defs.insert(
        "length".into(),
        json!({
            "description": format!(
                "A length: a number in the declared unit, such as 100, or a number with its own \
                 unit after it and at most one space, such as \"4 in\" or \"4in\", which is \
                 converted to the declared unit. Units: {}. A number is an optional -, digits, \
                 an optional . and digits, and an optional exponent such as e3.",
                units.join(", ")
            ),
            "anyOf": [
                { "type": "number" },
                { "type": "string", "pattern": format!("^{NUMBER}( ?({unit_pattern}))?$") },
            ],
        }),
    );
    defs.insert(
        "point".into(),
        json!({
            "description": "A point [x, y, z]: three lengths. Every coordinate is 3D; on a flat \
                            sheet z is 0.",
            "type": "array",
            "items": { "$ref": "#/$defs/length" },
            "minItems": 3,
            "maxItems": 3,
        }),
    );
    defs.insert(
        "angle".into(),
        json!({
            "description": "An angle in degrees, counted anticlockwise, with no unit. Whole turns \
                            are taken off: 360 is 0 and -90 is 270.",
            "anyOf": [
                { "type": "number" },
                { "type": "string", "pattern": format!("^{NUMBER}$") },
            ],
        }),
    );
    defs.insert(
        "name".into(),
        json!({
            "description": format!(
                "A layer or linetype name: 1 to {LONGEST_NAME} characters, not starting or \
                 ending with a space, holding none of {FORBIDDEN_IN_NAMES} and no control \
                 characters. AutoCAD ignores case in names, so two names that differ only in \
                 case are the same name."
            ),
            "type": "string",
            "minLength": 1,
            "maxLength": LONGEST_NAME,
            "pattern": name_pattern(),
        }),
    );
    defs.insert(
        "layer_name".into(),
        json!({
            "description": format!(
                "The layer the object sits on: `{LAYER_ZERO}`, which every drawing has, or a \
                 name under `layers`, written exactly as it is there. Without it the object \
                 sits on layer `{LAYER_ZERO}`."
            ),
            "type": ["string", "number"],
        }),
    );
    defs.insert(
        "linetype".into(),
        json!({
            "description": "A pattern of dashes, gaps and dots that layers draw their objects with.",
            "type": "object",
            "required": ["pattern"],
            "additionalProperties": false,
            "properties": {
                "description": {
                    "description": format!(
                        "What AutoCAD's linetype manager shows beside the name, at most \
                         {LONGEST_DESCRIPTION} characters."
                    ),
                    "type": ["string", "number", "boolean", "null"],
                },
                "pattern": {
                    "description": format!(
                        "2 to {LONGEST_PATTERN} lengths, repeated along each object: a positive \
                         length is a dash, a negative one a gap, 0 a dot. It starts with a dash \
                         or a dot, then a gap, as in an AutoCAD .lin file."
                    ),
                    "type": "array",
                    "items": { "$ref": "#/$defs/length" },
                    "minItems": 2,
                    "maxItems": LONGEST_PATTERN,
                },
            },
        }),
    );
    defs.insert(
        "layer".into(),
        json!({
            "description": "A layer: its objects draw with its colour, linetype and lineweight.",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "colour": {
                    "description": format!(
                        "An AutoCAD colour index from 1 to 255, or one of {} (1 to 7). \
                         Without it, {WHITE} (white).",
                        colour_names()
                    ),
                    "anyOf": [
                        { "type": "integer", "minimum": 1, "maximum": 255 },
                        { "enum": colours },
                    ],
                },
                "linetype": {
                    "description": format!(
                        "`{CONTINUOUS}`, the solid line every drawing has, or a name under \
                         `linetypes`, written exactly as it is there. Without it, `{CONTINUOUS}`."
                    ),
                    "type": ["string", "number"],
                },
                "lineweight": {
                    "description": format!(
                        "The pen width the layer plots with, in millimetres whatever the \
                         declared unit, with or without `mm`: one of {}. Without it, AutoCAD's \
                         default lineweight.",
                        Lineweight::choices()
                    ),
                    "anyOf": [
                        { "enum": lineweights },
                        { "type": "string", "pattern": format!("^({lineweight_pattern})( ?mm)?$") },
                    ],
                },
            },
        }),
    );
    defs.insert(
        "line".into(),
        object_def(
            "A straight line between two points.",
            &["from", "to"],
            json!({
                "from": { "$ref": "#/$defs/point", "description": "Where the line starts." },
                "to": { "$ref": "#/$defs/point", "description": "Where the line ends." },
            }),
        ),
    );
    defs.insert(
        "polyline".into(),
        object_def(
            "Straight segments joined end to end through two or more points. When every point \
             has the same z it is AutoCAD's lightweight polyline at that elevation; otherwise a \
             3D polyline.",
            &["points"],
            json!({
                "points": {
                    "description": "The points in order: at least 2, or 3 when closed.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/point" },
                    "minItems": 2,
                },
                "closed": {
                    "description": "Whether a last segment joins the last point back to the \
                                    first. Without it, false.",
                    "type": "boolean",
                },
            }),
        ),
    );
    defs.insert(
        "arc".into(),
        object_def(
            "Part of a circle, drawn anticlockwise from its start angle to its end angle, in the \
             plane parallel to XY at its centre's z.",
            &["centre", "radius", "start_angle", "end_angle"],
            json!({
                "centre": { "$ref": "#/$defs/point" },
                "radius": { "$ref": "#/$defs/length", "description": "A length more than 0." },
                "start_angle": {
                    "$ref": "#/$defs/angle",
                    "description": "Where the arc starts: degrees anticlockwise from the X axis.",
                },
                "end_angle": {
                    "$ref": "#/$defs/angle",
                    "description": "Where the arc ends: degrees anticlockwise from the X axis. \
                                    It differs from the start angle once whole turns are taken off.",
                },
            }),
        ),
    );
    defs.insert(
        "circle".into(),
        object_def(
            "A circle in the plane parallel to XY at its centre's z.",
            &["centre", "radius"],
            json!({
                "centre": { "$ref": "#/$defs/point" },
                "radius": { "$ref": "#/$defs/length", "description": "A length more than 0." },
            }),
        ),
    );
    defs.insert(
        "ellipse".into(),
        object_def(
            "An ellipse in the plane parallel to XY at its centre's z. With start_angle and \
             end_angle, only the part from one to the other, anticlockwise.",
            &["centre", "major_axis", "ratio"],
            json!({
                "centre": { "$ref": "#/$defs/point" },
                "major_axis": {
                    "$ref": "#/$defs/point",
                    "description": "From the centre to one end of the major axis, as [dx, dy, dz]; \
                                    dz is 0, and dx and dy are not both 0.",
                },
                "ratio": {
                    "description": "The minor axis over the major axis: more than 0 and at most 1.",
                    "anyOf": [
                        { "type": "number", "exclusiveMinimum": 0, "maximum": 1 },
                        { "type": "string", "pattern": format!("^{NUMBER}$") },
                    ],
                },
                "start_angle": {
                    "$ref": "#/$defs/angle",
                    "description": "Where an elliptical arc starts: degrees anticlockwise from the \
                                    major axis. Given with end_angle or not at all.",
                },
                "end_angle": {
                    "$ref": "#/$defs/angle",
                    "description": "Where an elliptical arc ends: degrees anticlockwise from the \
                                    major axis. Given with start_angle or not at all.",
                },
            }),
        ),
    );
    defs.insert(
        "spline".into(),
        {
            let mut spline = object_def(
                "A smooth curve, given either by points it passes through (fit_points) or by the \
                 control points of a B-spline (control_points), not both.",
                &[],
                json!({
                    "fit_points": {
                        "description": format!(
                            "At least {FEWEST_FIT_POINTS} points the curve passes through, in \
                             order, no two in a row the same. AutoCAD fits a cubic through them."
                        ),
                        "type": "array",
                        "items": { "$ref": "#/$defs/point" },
                        "minItems": FEWEST_FIT_POINTS,
                    },
                    "control_points": {
                        "description": "The control points of a clamped B-spline with uniform \
                                        knots: the curve starts at the first and ends at the \
                                        last. There are more control points than the degree.",
                        "type": "array",
                        "items": { "$ref": "#/$defs/point" },
                        "minItems": 2,
                    },
                    "degree": {
                        "description": format!(
                            "The degree of a curve by control points, 1 to {HIGHEST_DEGREE}. \
                             Without it, {FIT_DEGREE}. Not given with fit_points."
                        ),
                        "type": "integer",
                        "minimum": 1,
                        "maximum": HIGHEST_DEGREE,
                    },
                }),
            );
            spline["oneOf"] = json!([
                { "required": ["fit_points"], "not": { "anyOf": [{ "required": ["control_points"] }, { "required": ["degree"] }] } },
                { "required": ["control_points"], "not": { "required": ["fit_points"] } },
            ]);
            spline
        },
    );

    let [units_key, linetypes_key, layers_key, objects_key] = SHEET_KEYS;
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AgentDrafter sheet description",
        "description": "The text file that states what is on a new sheet; a build turns it into \
                        a DWG. It is written in a strict subset of YAML 1.2: one document; no \
                        anchors, aliases, tags or directives; every key is text and appears once \
                        in its mapping; a key not in this schema is an error that lists the keys \
                        allowed there. Every scalar is read as text and given its meaning here, so \
                        `yes`, `null` and `0x10` are never booleans, nulls or numbers. Comments, \
                        block style and flow style are all allowed. Lengths are in the declared \
                        unit; angles are in degrees, anticlockwise; a lineweight is in millimetres.",
        "type": "object",
        "required": [units_key, objects_key],
        "additionalProperties": false,
        "properties": {
            units_key: {
                "description": "The declared unit: every length on the sheet is in it unless it \
                                carries its own. It becomes the DWG's insertion unit. There is no \
                                default.",
                "enum": units,
            },
            linetypes_key: {
                "description": format!(
                    "The sheet's linetypes, from name to linetype. `{CONTINUOUS}`, `ByLayer` and \
                     `ByBlock` are in every drawing and cannot be defined again."
                ),
                "type": "object",
                "propertyNames": { "$ref": "#/$defs/name" },
                "additionalProperties": { "$ref": "#/$defs/linetype" },
            },
            layers_key: {
                "description": format!(
                    "The sheet's layers, from name to layer. Layer `{LAYER_ZERO}` is in every \
                     drawing; listing it sets its colour, linetype and lineweight."
                ),
                "type": "object",
                "propertyNames": { "$ref": "#/$defs/name" },
                "additionalProperties": { "$ref": "#/$defs/layer" },
            },
            objects_key: {
                "description": "The objects on the sheet, in drawing order. Each is a mapping \
                                with one key, its type.",
                "type": "array",
                "items": { "oneOf": objects },
            },
        },
        "$defs": defs,
    })
}

/// The definition of one object type: its own `properties` and `layer`.
fn object_def(description: &str, required: &[&str], properties: Value) -> Value {
    let mut properties = properties;
    properties["layer"] = json!({ "$ref": "#/$defs/layer_name" });
    json!({
        "description": description,
        "type": "object",
        "required": required,
        "additionalProperties": false,
        "properties": properties,
    })
}

/// A regular expression for a layer or linetype name: no forbidden or
/// control characters, and no space at either end.
fn name_pattern() -> String {
    let forbidden = FORBIDDEN_IN_NAMES.replace('\\', "\\\\");
    let inner = format!("[^{forbidden}\\x00-\\x1f]");
    let end = format!("[^{forbidden}\\x00-\\x1f\\s]");
    format!("^{end}({inner}*{end})?$")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::description::layers::{LAYER_KEYS, LINETYPE_KEYS};

    fn keys(value: &Value) -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    fn sorted(keys: &[&str]) -> Vec<String> {
        let mut keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
        keys.sort();
        keys
    }

    #[test]
    fn the_schema_and_the_reader_know_the_same_keys() {
        let schema = sheet_description_schema();
        assert_eq!(keys(&schema["properties"]), sorted(&SHEET_KEYS));
        let defs = &schema["$defs"];
        assert_eq!(
            keys(&defs["linetype"]["properties"]),
            sorted(&LINETYPE_KEYS)
        );
        assert_eq!(keys(&defs["layer"]["properties"]), sorted(&LAYER_KEYS));
        let listed: Vec<String> = schema["properties"]["objects"]["items"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| keys(&o["properties"])[0].clone())
            .collect();
        let types: Vec<String> = OBJECT_TYPES.iter().map(|(t, _)| (*t).to_owned()).collect();
        assert_eq!(listed, types);
        for (name, object_keys) in OBJECT_TYPES {
            assert_eq!(
                keys(&defs[name]["properties"]),
                sorted(object_keys),
                "the keys of {name}"
            );
        }
    }

    #[test]
    fn every_reference_in_the_schema_resolves() {
        fn walk(value: &Value, defs: &Value) {
            match value {
                Value::Object(map) => {
                    if let Some(Value::String(target)) = map.get("$ref") {
                        let name = target.strip_prefix("#/$defs/").expect("a local reference");
                        assert!(defs.get(name).is_some(), "{target} resolves");
                    }
                    map.values().for_each(|v| walk(v, defs));
                }
                Value::Array(items) => items.iter().for_each(|v| walk(v, defs)),
                _ => {}
            }
        }
        let schema = sheet_description_schema();
        walk(&schema, &schema["$defs"]);
    }
}
