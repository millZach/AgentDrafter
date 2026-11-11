//! The schema `agentdrafter schema` prints and the reader agree on what a
//! sheet description may hold.
//!
//! Each case is one value, written as YAML for the reader and as the JSON a
//! YAML reader that guesses types makes of it for the schema: `'42'` is the
//! string "42" and `42` the number 42. Both must accept it or both refuse
//! it. The cases leave out what JSON Schema cannot see: such a reader turns
//! `+5` and `5` into the same number, and JSON Schema cannot compare a
//! value against another (a spline's degree against its number of points).

use agentdrafter_core::{parse_sheet_description, sheet_description_schema};
use serde_json::{json, Value};

/// Where a value goes: a sheet description in YAML and the same sheet in
/// JSON, each with `V` where the value belongs.
struct Place {
    name: &'static str,
    yaml: &'static str,
    json: &'static str,
}

const COLOR: Place = Place {
    name: "a layer's color",
    yaml: "units: mm\nlayers:\n  a: {color: V}\nobjects: []\n",
    json: r#"{"units": "mm", "layers": {"a": {"color": V}}, "objects": []}"#,
};

const LINEWEIGHT: Place = Place {
    name: "a layer's lineweight",
    yaml: "units: mm\nlayers:\n  a: {lineweight: V}\nobjects: []\n",
    json: r#"{"units": "mm", "layers": {"a": {"lineweight": V}}, "objects": []}"#,
};

const CLOSED: Place = Place {
    name: "a polyline's closed",
    yaml: "units: mm\nobjects:\n  - polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]], closed: V}\n",
    json: r#"{"units": "mm", "objects": [{"polyline": {"points": [[0, 0, 0], [1, 0, 0], [1, 1, 0]], "closed": V}}]}"#,
};

/// Eleven control points, so every degree from 1 to 10 has enough.
const DEGREE: Place = Place {
    name: "a spline's degree",
    yaml: "units: mm\nobjects:\n  - spline: {control_points: [[0, 0, 0], [1, 1, 0], [2, 0, 0], [3, 1, 0], [4, 0, 0], [5, 1, 0], [6, 0, 0], [7, 1, 0], [8, 0, 0], [9, 1, 0], [10, 0, 0]], degree: V}\n",
    json: r#"{"units": "mm", "objects": [{"spline": {"control_points": [[0, 0, 0], [1, 1, 0], [2, 0, 0], [3, 1, 0], [4, 0, 0], [5, 1, 0], [6, 0, 0], [7, 1, 0], [8, 0, 0], [9, 1, 0], [10, 0, 0]], "degree": V}}]}"#,
};

const RADIUS: Place = Place {
    name: "a circle's radius",
    yaml: "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: V}\n",
    json: r#"{"units": "mm", "objects": [{"circle": {"center": [0, 0, 0], "radius": V}}]}"#,
};

const ANGLE: Place = Place {
    name: "an arc's start angle",
    yaml: "units: mm\nobjects:\n  - arc: {center: [0, 0, 0], radius: 1, start_angle: V, end_angle: 1}\n",
    json: r#"{"units": "mm", "objects": [{"arc": {"center": [0, 0, 0], "radius": 1, "start_angle": V, "end_angle": 1}}]}"#,
};

const LAYER: Place = Place {
    name: "an object's layer",
    yaml: "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1, layer: V}\n",
    json: r#"{"units": "mm", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1, "layer": V}}]}"#,
};

/// `(place, value as YAML, the same value as JSON, accepted)`.
fn cases() -> Vec<(&'static Place, &'static str, Value, bool)> {
    vec![
        // A color: an index from 1 to 255, quoted or not, or a name.
        (&COLOR, "42", json!(42), true),
        (&COLOR, "'42'", json!("42"), true),
        (&COLOR, "\"255\"", json!("255"), true),
        (&COLOR, "'007'", json!("007"), true),
        (&COLOR, "red", json!("red"), true),
        (&COLOR, "'white'", json!("white"), true),
        (&COLOR, "0", json!(0), false),
        (&COLOR, "256", json!(256), false),
        (&COLOR, "'0'", json!("0"), false),
        (&COLOR, "'256'", json!("256"), false),
        (&COLOR, "'1.5'", json!("1.5"), false),
        (&COLOR, "'+5'", json!("+5"), false),
        (&COLOR, "Red", json!("Red"), false),
        (&COLOR, "purple", json!("purple"), false),
        // A lineweight: a standard width in millimetres, with or without mm.
        (&LINEWEIGHT, "0.5", json!(0.5), true),
        (&LINEWEIGHT, "'0.50 mm'", json!("0.50 mm"), true),
        (&LINEWEIGHT, "0.35 mm", json!("0.35 mm"), true),
        (&LINEWEIGHT, "'0.35mm'", json!("0.35mm"), true),
        (&LINEWEIGHT, "'0.35'", json!("0.35"), true),
        (&LINEWEIGHT, "'1.0'", json!("1.0"), true),
        (&LINEWEIGHT, "'0'", json!("0"), true),
        (&LINEWEIGHT, "2.11", json!(2.11), true),
        (&LINEWEIGHT, "0.33", json!(0.33), false),
        (&LINEWEIGHT, "'0.33 mm'", json!("0.33 mm"), false),
        (&LINEWEIGHT, "'0.35 in'", json!("0.35 in"), false),
        (&LINEWEIGHT, "'10'", json!("10"), false),
        (&LINEWEIGHT, "'-0.35'", json!("-0.35"), false),
        (&LINEWEIGHT, "'3.5e-1 mm'", json!("3.5e-1 mm"), false),
        (&LINEWEIGHT, "thick", json!("thick"), false),
        // closed: true or false, quoted or not.
        (&CLOSED, "true", json!(true), true),
        (&CLOSED, "'true'", json!("true"), true),
        (&CLOSED, "\"false\"", json!("false"), true),
        (&CLOSED, "'yes'", json!("yes"), false),
        (&CLOSED, "'True'", json!("True"), false),
        (&CLOSED, "'1'", json!("1"), false),
        // A degree: a whole number from 1 to 10, quoted or not.
        (&DEGREE, "2", json!(2), true),
        (&DEGREE, "'2'", json!("2"), true),
        (&DEGREE, "'02'", json!("02"), true),
        (&DEGREE, "'10'", json!("10"), true),
        (&DEGREE, "0", json!(0), false),
        (&DEGREE, "11", json!(11), false),
        (&DEGREE, "2.5", json!(2.5), false),
        (&DEGREE, "'0'", json!("0"), false),
        (&DEGREE, "'11'", json!("11"), false),
        (&DEGREE, "'2.5'", json!("2.5"), false),
        (&DEGREE, "'two'", json!("two"), false),
        (&DEGREE, "'+2'", json!("+2"), false),
        // A length, with or without its own unit.
        (&RADIUS, "4", json!(4), true),
        (&RADIUS, "'4'", json!("4"), true),
        (&RADIUS, "4 in", json!("4 in"), true),
        (&RADIUS, "'4in'", json!("4in"), true),
        (&RADIUS, "'4 furlong'", json!("4 furlong"), false),
        (&RADIUS, "'4  in'", json!("4  in"), false),
        (&RADIUS, "'.5'", json!(".5"), false),
        // An angle: degrees with no unit.
        (&ANGLE, "90", json!(90), true),
        (&ANGLE, "'-45.5'", json!("-45.5"), true),
        (&ANGLE, "'90 deg'", json!("90 deg"), false),
        // A layer: 0 and '0' are one layer.
        (&LAYER, "0", json!(0), true),
        (&LAYER, "'0'", json!("0"), true),
    ]
}

#[test]
fn the_schema_accepts_exactly_the_values_the_reader_accepts() {
    let schema = sheet_description_schema();
    let validator = jsonschema::validator_for(&schema).expect("the schema is a valid JSON Schema");
    let mut disagreements = Vec::new();
    for (place, yaml_value, json_value, accepted) in cases() {
        let yaml = place.yaml.replace('V', yaml_value);
        let json: Value = serde_json::from_str(&place.json.replace('V', &json_value.to_string()))
            .expect("the JSON case is JSON");
        let reader = parse_sheet_description(&yaml);
        let by_schema = validator.is_valid(&json);
        if reader.is_ok() != accepted || by_schema != accepted {
            disagreements.push(format!(
                "{} {yaml_value}: expected {}, the reader {} ({}), the schema {}",
                place.name,
                if accepted { "accepted" } else { "refused" },
                if reader.is_ok() { "accepts" } else { "refuses" },
                reader.err().map(|e| e.message).unwrap_or_default(),
                if by_schema { "accepts" } else { "refuses" },
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "{} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}
