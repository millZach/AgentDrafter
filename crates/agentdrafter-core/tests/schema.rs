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

const SCALE: Place = Place {
    name: "a hatch's scale",
    yaml: "units: mm\nhatch_patterns: {P: {lines: [{delta: [0, 1]}]}}\nobjects:\n  - hatch: {pattern: P, scale: V, boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]}\n",
    json: r#"{"units": "mm", "hatch_patterns": {"P": {"lines": [{"delta": [0, 1]}]}}, "objects": [{"hatch": {"pattern": "P", "scale": V, "boundary": [{"polyline": {"points": [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]}}]}"#,
};

const SOLID_SCALE: Place = Place {
    name: "a solid hatch's scale",
    yaml: "units: mm\nobjects:\n  - hatch: {pattern: SOLID, scale: V, boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]}\n",
    json: r#"{"units": "mm", "objects": [{"hatch": {"pattern": "SOLID", "scale": V, "boundary": [{"polyline": {"points": [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]}}]}"#,
};

const PATTERN_NAME: Place = Place {
    name: "a hatch pattern's name",
    yaml: "units: mm\nhatch_patterns:\n  V: {lines: [{delta: [0, 1]}]}\nobjects: []\n",
    json: r#"{"units": "mm", "hatch_patterns": {V: {"lines": [{"delta": [0, 1]}]}}, "objects": []}"#,
};

const DELTA: Place = Place {
    name: "a pattern line's delta",
    yaml: "units: mm\nhatch_patterns:\n  P: {lines: [{delta: V}]}\nobjects: []\n",
    json: r#"{"units": "mm", "hatch_patterns": {"P": {"lines": [{"delta": V}]}}, "objects": []}"#,
};

const DASHES: Place = Place {
    name: "a pattern line's dashes",
    yaml: "units: mm\nhatch_patterns:\n  P: {lines: [{delta: [0, 1], dashes: V}]}\nobjects: []\n",
    json: r#"{"units": "mm", "hatch_patterns": {"P": {"lines": [{"delta": [0, 1], "dashes": V}]}}, "objects": []}"#,
};

const BOUNDARY: Place = Place {
    name: "a hatch's boundary",
    yaml: "units: mm\nobjects:\n  - hatch: {pattern: SOLID, boundary: V}\n",
    json: r#"{"units": "mm", "objects": [{"hatch": {"pattern": "SOLID", "boundary": V}}]}"#,
};

const FONT: Place = Place {
    name: "a text style's font",
    yaml: "units: mm\ntext_styles:\n  a: {font: V}\nobjects: []\n",
    json: r#"{"units": "mm", "text_styles": {"a": {"font": V}}, "objects": []}"#,
};

const STYLE_WIDTH_FACTOR: Place = Place {
    name: "a text style's width factor",
    yaml: "units: mm\ntext_styles:\n  a: {font: a.shx, width_factor: V}\nobjects: []\n",
    json: r#"{"units": "mm", "text_styles": {"a": {"font": "a.shx", "width_factor": V}}, "objects": []}"#,
};

const CONTENTS: Place = Place {
    name: "a text's contents",
    yaml: "units: mm\nobjects:\n  - text: {contents: V, position: [0, 0, 0], height: 1}\n",
    json: r#"{"units": "mm", "objects": [{"text": {"contents": V, "position": [0, 0, 0], "height": 1}}]}"#,
};

const JUSTIFY: Place = Place {
    name: "a text's justify",
    yaml:
        "units: mm\nobjects:\n  - text: {contents: A, position: [0, 0, 0], height: 1, justify: V}\n",
    json: r#"{"units": "mm", "objects": [{"text": {"contents": "A", "position": [0, 0, 0], "height": 1, "justify": V}}]}"#,
};

const ATTACH: Place = Place {
    name: "an mtext's justify",
    yaml: "units: mm\nobjects:\n  - mtext: {contents: A, position: [0, 0, 0], height: 1, justify: V}\n",
    json: r#"{"units": "mm", "objects": [{"mtext": {"contents": "A", "position": [0, 0, 0], "height": 1, "justify": V}}]}"#,
};

const MTEXT_WIDTH: Place = Place {
    name: "an mtext's width",
    yaml:
        "units: mm\nobjects:\n  - mtext: {contents: A, position: [0, 0, 0], height: 1, width: V}\n",
    json: r#"{"units": "mm", "objects": [{"mtext": {"contents": "A", "position": [0, 0, 0], "height": 1, "width": V}}]}"#,
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
        // A hatch's scale: a number more than 0, with no unit.
        (&SCALE, "2", json!(2), true),
        (&SCALE, "'0.5'", json!("0.5"), true),
        (&SCALE, "1e-3", json!(0.001), true),
        (&SCALE, "0", json!(0), false),
        (&SCALE, "-1.5", json!(-1.5), false),
        (&SCALE, "'2 mm'", json!("2 mm"), false),
        (&SCALE, "big", json!("big"), false),
        // A solid hatch has no scale.
        (&SOLID_SCALE, "1", json!(1), false),
        (&SOLID_SCALE, "'1'", json!("1"), false),
        // A hatch pattern's name: capitals, no spaces, at most 34 characters.
        (&PATTERN_NAME, "FINE", json!("FINE"), true),
        (&PATTERN_NAME, "'DASH-DOT_2'", json!("DASH-DOT_2"), true),
        (&PATTERN_NAME, "ÅÉØ_1", json!("ÅÉØ_1"), true),
        (
            &PATTERN_NAME,
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ01234567",
            json!("ABCDEFGHIJKLMNOPQRSTUVWXYZ01234567"),
            true,
        ),
        (
            &PATTERN_NAME,
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ012345678",
            json!("ABCDEFGHIJKLMNOPQRSTUVWXYZ012345678"),
            false,
        ),
        (&PATTERN_NAME, "fine", json!("fine"), false),
        (&PATTERN_NAME, "Fine", json!("Fine"), false),
        (&PATTERN_NAME, "éCLAT", json!("éCLAT"), false),
        (&PATTERN_NAME, "SOLID", json!("SOLID"), false),
        (&PATTERN_NAME, "_USER", json!("_USER"), false),
        (&PATTERN_NAME, "'TWO WORDS'", json!("TWO WORDS"), false),
        (&PATTERN_NAME, "'A|B'", json!("A|B"), false),
        // A pattern line's delta: two lengths.
        (&DELTA, "[0, 1]", json!([0, 1]), true),
        (&DELTA, "['1 in', '2.5mm']", json!(["1 in", "2.5mm"]), true),
        (&DELTA, "[1]", json!([1]), false),
        (&DELTA, "[0, 1, 0]", json!([0, 1, 0]), false),
        (&DELTA, "[0, 1 furlong]", json!([0, "1 furlong"]), false),
        // A pattern line's dashes: at least two lengths.
        (&DASHES, "[1, -1]", json!([1, -1]), true),
        (&DASHES, "['0.5 mm', 0, -1]", json!(["0.5 mm", 0, -1]), true),
        (&DASHES, "[-1]", json!([-1]), false),
        (&DASHES, "[]", json!([]), false),
        // A boundary: loops of the right shape.
        (
            &BOUNDARY,
            "[{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]",
            json!([{"polyline": {"points": [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]),
            true,
        ),
        (
            &BOUNDARY,
            "[{edges: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}, {arc: {center: [0.5, 0, 0], radius: 0.5, start_angle: 0, end_angle: 180}}]}]",
            json!([{"edges": [{"line": {"from": [0, 0, 0], "to": [1, 0, 0]}}, {"arc": {"center": [0.5, 0, 0], "radius": 0.5, "start_angle": 0, "end_angle": 180}}]}]),
            true,
        ),
        (&BOUNDARY, "[]", json!([]), false),
        (
            &BOUNDARY,
            "[{polyline: {points: [[0, 0, 0], [1, 0, 0]]}}]",
            json!([{"polyline": {"points": [[0, 0, 0], [1, 0, 0]]}}]),
            false,
        ),
        (
            &BOUNDARY,
            "[{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]], closed: true}}]",
            json!([{"polyline": {"points": [[0, 0, 0], [1, 0, 0], [1, 1, 0]], "closed": true}}]),
            false,
        ),
        (
            &BOUNDARY,
            "[{edges: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}]}]",
            json!([{"edges": [{"line": {"from": [0, 0, 0], "to": [1, 0, 0]}}]}]),
            false,
        ),
        (
            &BOUNDARY,
            "[{circle: {center: [0, 0, 0], radius: 1}}]",
            json!([{"circle": {"center": [0, 0, 0], "radius": 1}}]),
            false,
        ),
        // A font: an SHX or TrueType file name, the extension in any case.
        (&FONT, "romans.shx", json!("romans.shx"), true),
        (&FONT, "'Arial.TTF'", json!("Arial.TTF"), true),
        (&FONT, "a.b.shx", json!("a.b.shx"), true),
        (&FONT, "romans", json!("romans"), false),
        (&FONT, "arial.otf", json!("arial.otf"), false),
        (&FONT, "'.shx'", json!(".shx"), false),
        (
            &FONT,
            "'fonts/romans.shx'",
            json!("fonts/romans.shx"),
            false,
        ),
        (&FONT, "'C:romans.shx'", json!("C:romans.shx"), false),
        // A width factor: a number from 0.01 to 100, quoted or not.
        (&STYLE_WIDTH_FACTOR, "0.8", json!(0.8), true),
        (&STYLE_WIDTH_FACTOR, "'0.8'", json!("0.8"), true),
        (&STYLE_WIDTH_FACTOR, "100", json!(100), true),
        (&STYLE_WIDTH_FACTOR, "0.01", json!(0.01), true),
        (&STYLE_WIDTH_FACTOR, "0", json!(0), false),
        (&STYLE_WIDTH_FACTOR, "101", json!(101), false),
        (&STYLE_WIDTH_FACTOR, "'0.8 mm'", json!("0.8 mm"), false),
        (&STYLE_WIDTH_FACTOR, "narrow", json!("narrow"), false),
        // Contents: text, whatever a type-guessing reader makes of it.
        (&CONTENTS, "PUMP", json!("PUMP"), true),
        (&CONTENTS, "100", json!(100), true),
        (&CONTENTS, "'yes'", json!("yes"), true),
        (&CONTENTS, "[A]", json!(["A"]), false),
        // A single line's justification.
        (&JUSTIFY, "middle_center", json!("middle_center"), true),
        (&JUSTIFY, "'left'", json!("left"), true),
        (&JUSTIFY, "middle", json!("middle"), true),
        (&JUSTIFY, "centre", json!("centre"), false),
        (&JUSTIFY, "Left", json!("Left"), false),
        (&JUSTIFY, "aligned", json!("aligned"), false),
        // Multiline text's justification: no baseline ones.
        (&ATTACH, "bottom_right", json!("bottom_right"), true),
        (&ATTACH, "left", json!("left"), false),
        (&ATTACH, "middle", json!("middle"), false),
        // An mtext width: a length, 0 or more (the sign JSON Schema cannot see).
        (&MTEXT_WIDTH, "80", json!(80), true),
        (&MTEXT_WIDTH, "'3 in'", json!("3 in"), true),
        (&MTEXT_WIDTH, "wide", json!("wide"), false),
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
