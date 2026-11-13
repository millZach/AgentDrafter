//! The schema `agentdrafter schema` prints and the reader agree on what a
//! sheet description may hold.
//!
//! Each case is one value, written as YAML for the reader and as the JSON a
//! YAML reader that guesses types makes of it for the schema: `'42'` is the
//! string "42" and `42` the number 42. Both must accept it or both refuse
//! it. The cases leave out what JSON Schema cannot see: such a reader turns
//! `+5` and `5` into the same number, and JSON Schema cannot compare a
//! value against another (a spline's degree against its number of points).

use std::fs;
use std::path::Path;

use agentdrafter_core::{
    block_library_schema, parse_block_library, parse_sheet_description, read_sheet_description,
    sheet_description_schema, Error,
};
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

const OBJECT_COLOR: Place = Place {
    name: "an object's color",
    yaml: "units: mm
objects:
  - circle: {center: [0, 0, 0], radius: 1, color: V}
",
    json: r#"{"units": "mm", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1, "color": V}}]}"#,
};

const OBJECT_LINETYPE: Place = Place {
    name: "an object's linetype",
    yaml: "units: mm
linetypes:
  DASHED: {pattern: [1, -1]}
objects:
  - circle: {center: [0, 0, 0], radius: 1, linetype: V}
",
    json: r#"{"units": "mm", "linetypes": {"DASHED": {"pattern": [1, -1]}}, "objects": [{"circle": {"center": [0, 0, 0], "radius": 1, "linetype": V}}]}"#,
};

const OBJECT_LINEWEIGHT: Place = Place {
    name: "an object's lineweight",
    yaml: "units: mm
objects:
  - circle: {center: [0, 0, 0], radius: 1, lineweight: V}
",
    json: r#"{"units": "mm", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1, "lineweight": V}}]}"#,
};

const LINETYPE_SCALE: Place = Place {
    name: "an object's linetype scale",
    yaml: "units: mm
objects:
  - circle: {center: [0, 0, 0], radius: 1, linetype_scale: V}
",
    json: r#"{"units": "mm", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1, "linetype_scale": V}}]}"#,
};

const LTSCALE: Place = Place {
    name: "a sheet's ltscale",
    yaml: "units: mm
ltscale: V
objects: []
",
    json: r#"{"units": "mm", "ltscale": V, "objects": []}"#,
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

const OBJECT_ID: Place = Place {
    name: "an object's id",
    yaml: "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1, id: V}\n",
    json: r#"{"units": "mm", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1, "id": V}}]}"#,
};

const OBJECT_NAME: Place = Place {
    name: "an object's name",
    yaml: "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1, name: V}\n",
    json: r#"{"units": "mm", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1, "name": V}}]}"#,
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

/// The block library the places on a sheet that names one draw on: a block
/// `B` with the attribute `TAG`.
const LIBRARY: &str =
    "units: mm\nblocks:\n  B: {objects: [{attdef: {tag: TAG, position: [0, 0, 0], height: 1}}]}\n";

const INSERT_SCALE: Place = Place {
    name: "an insert's scale",
    yaml: "units: mm\nblock_library: lib.yaml\nobjects:\n  - insert: {block: B, position: [0, 0, 0], scale: V}\n",
    json: r#"{"units": "mm", "block_library": "lib.yaml", "objects": [{"insert": {"block": "B", "position": [0, 0, 0], "scale": V}}]}"#,
};

const ATTRIBUTE_VALUE: Place = Place {
    name: "an insert's attribute value",
    yaml: "units: mm\nblock_library: lib.yaml\nobjects:\n  - insert: {block: B, position: [0, 0, 0], attributes: {TAG: V}}\n",
    json: r#"{"units": "mm", "block_library": "lib.yaml", "objects": [{"insert": {"block": "B", "position": [0, 0, 0], "attributes": {"TAG": V}}}]}"#,
};

const ATTRIBUTE_KEY: Place = Place {
    name: "an insert's attribute tag",
    yaml: "units: mm\nblock_library: lib.yaml\nobjects:\n  - insert: {block: B, position: [0, 0, 0], attributes: {V: A-1}}\n",
    json: r#"{"units": "mm", "block_library": "lib.yaml", "objects": [{"insert": {"block": "B", "position": [0, 0, 0], "attributes": {V: "A-1"}}}]}"#,
};

const INSERT_NAME: Place = Place {
    name: "an insert's name",
    yaml: "units: mm\nblock_library: lib.yaml\nobjects:\n  - insert: {block: B, position: [0, 0, 0], name: V}\n",
    json: r#"{"units": "mm", "block_library": "lib.yaml", "objects": [{"insert": {"block": "B", "position": [0, 0, 0], "name": V}}]}"#,
};

const INSERT_BLOCK: Place = Place {
    name: "an insert's block",
    yaml: "units: mm\nblock_library: lib.yaml\nobjects:\n  - insert: {block: V, position: [0, 0, 0]}\n",
    json: r#"{"units": "mm", "block_library": "lib.yaml", "objects": [{"insert": {"block": V, "position": [0, 0, 0]}}]}"#,
};

const LIBRARY_PATH: Place = Place {
    name: "a sheet's block library",
    yaml: "units: mm\nblock_library: V\nobjects: []\n",
    json: r#"{"units": "mm", "block_library": V, "objects": []}"#,
};

const PAPER: Place = Place {
    name: "a sheet's paper",
    yaml: "units: mm\npaper: V\nobjects: []\n",
    json: r#"{"units": "mm", "paper": V, "objects": []}"#,
};

const MTEXT_CONTENTS: Place = Place {
    name: "an mtext's contents",
    yaml: "units: mm\nobjects:\n  - mtext: {contents: V, position: [0, 0, 0], height: 1}\n",
    json: r#"{"units": "mm", "objects": [{"mtext": {"contents": V, "position": [0, 0, 0], "height": 1}}]}"#,
};

const TEXT_STYLE: Place = Place {
    name: "a text's style",
    yaml:
        "units: mm\nobjects:\n  - text: {contents: A, position: [0, 0, 0], height: 1, style: V}\n",
    json: r#"{"units": "mm", "objects": [{"text": {"contents": "A", "position": [0, 0, 0], "height": 1, "style": V}}]}"#,
};

const LINETYPE_DESCRIPTION: Place = Place {
    name: "a linetype's description",
    yaml: "units: mm\nlinetypes:\n  L: {pattern: [1, -1], description: V}\nobjects: []\n",
    json: r#"{"units": "mm", "linetypes": {"L": {"pattern": [1, -1], "description": V}}, "objects": []}"#,
};

const LAYER_LINETYPE: Place = Place {
    name: "a layer's linetype",
    yaml: "units: mm\nlayers:\n  a: {linetype: V}\nobjects: []\n",
    json: r#"{"units": "mm", "layers": {"a": {"linetype": V}}, "objects": []}"#,
};

const HATCH_PATTERN: Place = Place {
    name: "a hatch's pattern",
    yaml: "units: mm\nobjects:\n  - hatch: {pattern: V, boundary: [{polyline: {points: [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]}\n",
    json: r#"{"units": "mm", "objects": [{"hatch": {"pattern": V, "boundary": [{"polyline": {"points": [[0, 0, 0], [1, 0, 0], [1, 1, 0]]}}]}}]}"#,
};

const SHEET_TEXT_CELL: Place = Place {
    name: "a sheet's text's cell",
    yaml: "units: mm\nobjects:\n  - text: {contents: A, position: [0, 0, 0], height: 1, cell: V}\n",
    json: r#"{"units": "mm", "objects": [{"text": {"contents": "A", "position": [0, 0, 0], "height": 1, "cell": V}}]}"#,
};

/// `(place, value as YAML, the same value as JSON, accepted)`.
type Case = (&'static Place, &'static str, Value, bool);

/// The cases of a bare null on the text or path field at `place`: `null`,
/// `Null`, `NULL`, `~` and a value left empty, which a type-guessing YAML
/// reader makes null. Zach decided on 2026-11-11 that a bare null leaves
/// the field out, as if it were not written, so an `optional` field takes
/// it and a required one refuses it. Then, unless the field names
/// something JSON Schema cannot look up, such as a layer, the `quoted`
/// word, which is text, accepted where that text is.
fn bare_null(
    place: &'static Place,
    optional: bool,
    quoted: Option<(&'static str, bool)>,
) -> Vec<Case> {
    let mut cases: Vec<Case> = ["null", "Null", "NULL", "~", ""]
        .into_iter()
        .map(|written| (place, written, json!(null), optional))
        .collect();
    if let Some((written, accepted)) = quoted {
        let text = written.trim_matches(['\'', '"']);
        cases.push((place, written, json!(text), accepted));
    }
    cases
}

/// A bare null on every text and path field of a sheet description.
fn bare_null_cases() -> Vec<Case> {
    [
        bare_null(&OBJECT_ID, true, Some(("'null'", true))),
        bare_null(&OBJECT_NAME, true, Some(("'null'", true))),
        bare_null(&INSERT_NAME, true, Some(("\"~\"", true))),
        bare_null(&LAYER, true, None),
        bare_null(&TEXT_STYLE, true, None),
        bare_null(&LAYER_LINETYPE, true, None),
        bare_null(&LINETYPE_DESCRIPTION, true, Some(("'null'", true))),
        bare_null(&LIBRARY_PATH, true, None),
        bare_null(&ATTRIBUTE_VALUE, true, Some(("'NULL'", true))),
        bare_null(&CONTENTS, false, Some(("'null'", true))),
        bare_null(&MTEXT_CONTENTS, false, Some(("'~'", true))),
        bare_null(&FONT, false, Some(("'null'", false))),
        bare_null(&INSERT_BLOCK, false, None),
        bare_null(&HATCH_PATTERN, false, None),
    ]
    .concat()
}

fn cases() -> Vec<Case> {
    let mut cases = bare_null_cases();
    cases.extend(vec![
        // A paper: two lengths more than 0, or a standard size and the way
        // it lies, written exactly.
        (&PAPER, "[297, 210]", json!([297, 210]), true),
        (&PAPER, "['17 in', '11 in']", json!(["17 in", "11 in"]), true),
        (&PAPER, "['17', 11.5]", json!(["17", 11.5]), true),
        (&PAPER, "ANSI B landscape", json!("ANSI B landscape"), true),
        (&PAPER, "ISO A4 portrait", json!("ISO A4 portrait"), true),
        (&PAPER, "ANSI B", json!("ANSI B"), false),
        (&PAPER, "'ansi b landscape'", json!("ansi b landscape"), false),
        (&PAPER, "ANSI F landscape", json!("ANSI F landscape"), false),
        (&PAPER, "[17]", json!([17]), false),
        (&PAPER, "[17, 11, 0]", json!([17, 11, 0]), false),
        (&PAPER, "[0, 11]", json!([0, 11]), false),
        (&PAPER, "[-17, 11]", json!([-17, 11]), false),
        (&PAPER, "[17, wide]", json!([17, "wide"]), false),
        (&PAPER, "17", json!(17), false),
        (&PAPER, "{width: 17}", json!({"width": 17}), false),
        // A cell is a block's: a sheet's text takes none.
        (
            &SHEET_TEXT_CELL,
            "[[0, 0, 0], [1, 1, 0]]",
            json!([[0, 0, 0], [1, 1, 0]]),
            false,
        ),
        // An insert's scale: a number more than 0, with no unit.
        (&INSERT_SCALE, "2", json!(2), true),
        (&INSERT_SCALE, "'0.5'", json!("0.5"), true),
        (&INSERT_SCALE, "0", json!(0), false),
        (&INSERT_SCALE, "-1", json!(-1), false),
        (&INSERT_SCALE, "'2 mm'", json!("2 mm"), false),
        (&INSERT_SCALE, "big", json!("big"), false),
        // An attribute's value: one line of text, which may be empty.
        (&ATTRIBUTE_VALUE, "A-1", json!("A-1"), true),
        (&ATTRIBUTE_VALUE, "100", json!(100), true),
        (&ATTRIBUTE_VALUE, "'yes'", json!("yes"), true),
        (&ATTRIBUTE_VALUE, "''", json!(""), true),
        (&ATTRIBUTE_VALUE, "'50%'", json!("50%"), true),
        (&ATTRIBUTE_VALUE, "\"A\\nB\"", json!("A\nB"), false),
        (&ATTRIBUTE_VALUE, "'\\U+00B0'", json!("\\U+00B0"), false),
        (&ATTRIBUTE_VALUE, "[A]", json!(["A"]), false),
        // An attribute's tag, as a key: in capitals.
        (&ATTRIBUTE_KEY, "TAG", json!("TAG"), true),
        (&ATTRIBUTE_KEY, "tag", json!("tag"), false),
        (&ATTRIBUTE_KEY, "'T G'", json!("T G"), false),
        // A name: text with no space at either end, given once.
        (&INSERT_NAME, "jb", json!("jb"), true),
        (&INSERT_NAME, "'junction box 2'", json!("junction box 2"), true),
        (&INSERT_NAME, "42", json!(42), true),
        (&INSERT_NAME, "' jb'", json!(" jb"), false),
        (&INSERT_NAME, "''", json!(""), false),
        (&INSERT_NAME, "[jb]", json!(["jb"]), false),
        // A block library's path: relative, with / between folders.
        (&LIBRARY_PATH, "lib.yaml", json!("lib.yaml"), true),
        (&LIBRARY_PATH, "'./lib.yaml'", json!("./lib.yaml"), true),
        (&LIBRARY_PATH, "sub/../lib.yaml", json!("sub/../lib.yaml"), true),
        (&LIBRARY_PATH, "/lib.yaml", json!("/lib.yaml"), false),
        (&LIBRARY_PATH, "'sub\\lib.yaml'", json!("sub\\lib.yaml"), false),
        (&LIBRARY_PATH, "sub//lib.yaml", json!("sub//lib.yaml"), false),
        (&LIBRARY_PATH, "'C:lib.yaml'", json!("C:lib.yaml"), false),
        (&LIBRARY_PATH, "sub/", json!("sub/"), false),
        (&LIBRARY_PATH, "''", json!(""), false),
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
        // An object's own color: ByLayer, ByBlock, or a layer's color.
        (&OBJECT_COLOR, "ByLayer", json!("ByLayer"), true),
        (&OBJECT_COLOR, "'ByBlock'", json!("ByBlock"), true),
        (&OBJECT_COLOR, "42", json!(42), true),
        (&OBJECT_COLOR, "'42'", json!("42"), true),
        (&OBJECT_COLOR, "red", json!("red"), true),
        (&OBJECT_COLOR, "bylayer", json!("bylayer"), false),
        (&OBJECT_COLOR, "0", json!(0), false),
        (&OBJECT_COLOR, "256", json!(256), false),
        (&OBJECT_COLOR, "purple", json!("purple"), false),
        // An object's own linetype: ByLayer, ByBlock, Continuous or the sheet's.
        (&OBJECT_LINETYPE, "ByLayer", json!("ByLayer"), true),
        (&OBJECT_LINETYPE, "ByBlock", json!("ByBlock"), true),
        (&OBJECT_LINETYPE, "Continuous", json!("Continuous"), true),
        (&OBJECT_LINETYPE, "DASHED", json!("DASHED"), true),
        (&OBJECT_LINETYPE, "null", json!(null), true),
        (&OBJECT_LINETYPE, "[DASHED]", json!(["DASHED"]), false),
        // An object's own lineweight: ByLayer, ByBlock, Default or a layer's.
        (&OBJECT_LINEWEIGHT, "ByLayer", json!("ByLayer"), true),
        (&OBJECT_LINEWEIGHT, "ByBlock", json!("ByBlock"), true),
        (&OBJECT_LINEWEIGHT, "Default", json!("Default"), true),
        (&OBJECT_LINEWEIGHT, "0.35", json!(0.35), true),
        (&OBJECT_LINEWEIGHT, "'0.50 mm'", json!("0.50 mm"), true),
        (&OBJECT_LINEWEIGHT, "default", json!("default"), false),
        (&OBJECT_LINEWEIGHT, "0.33", json!(0.33), false),
        // A linetype scale: a number more than 0, with no unit.
        (&LINETYPE_SCALE, "0.5", json!(0.5), true),
        (&LINETYPE_SCALE, "'2'", json!("2"), true),
        (&LINETYPE_SCALE, "0", json!(0), false),
        (&LINETYPE_SCALE, "-1", json!(-1), false),
        (&LINETYPE_SCALE, "'2 mm'", json!("2 mm"), false),
        (&LTSCALE, "2.5", json!(2.5), true),
        (&LTSCALE, "'0.25'", json!("0.25"), true),
        (&LTSCALE, "0", json!(0), false),
        (&LTSCALE, "big", json!("big"), false),
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
        // An object's ID: ASCII letters, digits, _ - . and :, as text.
        (&OBJECT_ID, "pump-1", json!("pump-1"), true),
        (&OBJECT_ID, "'loop:FT_101.3'", json!("loop:FT_101.3"), true),
        (&OBJECT_ID, "101", json!(101), true),
        (&OBJECT_ID, "'101'", json!("101"), true),
        (&OBJECT_ID, "1.5", json!(1.5), true),
        (&OBJECT_ID, "true", json!(true), true),
        (&OBJECT_ID, "''", json!(""), false),
        (&OBJECT_ID, "'a b'", json!("a b"), false),
        (&OBJECT_ID, "'DEMO-001/a'", json!("DEMO-001/a"), false),
        // A place written whole as an `id`, as an operation writes it when
        // it deletes an object before it, keeps the object's ID.
        (&OBJECT_ID, "'objects[3]'", json!("objects[3]"), true),
        (&OBJECT_ID, "'rows[1].objects[0]'", json!("rows[1].objects[0]"), true),
        (&OBJECT_ID, "'objects[03]'", json!("objects[03]"), false),
        (&OBJECT_ID, "'objects[3'", json!("objects[3"), false),
        (&OBJECT_ID, "'Objects[3]'", json!("Objects[3]"), false),
        (&OBJECT_ID, "'rows[1].objects'", json!("rows[1].objects"), false),
        (&OBJECT_ID, "'état'", json!("état"), false),
        (&OBJECT_ID, "[a]", json!(["a"]), false),
        (&OBJECT_ID, "{a: b}", json!({"a": "b"}), false),
        // An object's name, which every object on a sheet may carry.
        (&OBJECT_NAME, "frame", json!("frame"), true),
        (&OBJECT_NAME, "'Pump P-101 frame'", json!("Pump P-101 frame"), true),
        (&OBJECT_NAME, "7", json!(7), true),
        (&OBJECT_NAME, "''", json!(""), false),
        (&OBJECT_NAME, "' frame'", json!(" frame"), false),
        (&OBJECT_NAME, "[a]", json!(["a"]), false),
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
    ]);
    // Contents are required, so contents that do not show are an error:
    // empty, quoted, or only white space, Unicode's included. A character
    // that is not white space, even one that draws nothing, shows.
    for place in [&CONTENTS, &MTEXT_CONTENTS] {
        cases.extend([
            (place, "''", json!(""), false),
            (place, "' '", json!(" "), false),
            (
                place,
                "\"\\t \\u3000\\u2029\"",
                json!("\t \u{3000}\u{2029}"),
                false,
            ),
            (place, "' A '", json!(" A "), true),
            (place, "\"\\u00a0A\"", json!("\u{a0}A"), true),
            (place, "\"\\ufeff\"", json!("\u{feff}"), true),
        ]);
    }
    cases.extend(vec![
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
    ]);
    cases
}

/// Reads a sheet description as `agentdrafter build` does. One that names a
/// block library is written to a folder beside [`LIBRARY`], as `lib.yaml`.
fn read_sheet(yaml: &str) -> Result<(), Error> {
    if !yaml.contains("block_library") {
        return parse_sheet_description(yaml).map(|_| ());
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("core-schema");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("lib.yaml"), LIBRARY).unwrap();
    let description = dir.join("sheet.yaml");
    fs::write(&description, yaml).unwrap();
    read_sheet_description(&description).map(|_| ())
}

#[test]
fn the_schema_accepts_exactly_the_values_the_reader_accepts() {
    assert_reader_and_schema_agree(&sheet_description_schema(), cases(), read_sheet);
}

const TAG: Place = Place {
    name: "an attribute definition's tag",
    yaml:
        "units: mm\nblocks:\n  B: {objects: [{attdef: {tag: V, position: [0, 0, 0], height: 1}}]}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"objects": [{"attdef": {"tag": V, "position": [0, 0, 0], "height": 1}}]}}}"#,
};

const INVISIBLE: Place = Place {
    name: "an attribute definition's invisible",
    yaml: "units: mm\nblocks:\n  B: {objects: [{attdef: {tag: T, position: [0, 0, 0], height: 1, invisible: V}}]}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"objects": [{"attdef": {"tag": "T", "position": [0, 0, 0], "height": 1, "invisible": V}}]}}}"#,
};

const DEFAULT: Place = Place {
    name: "an attribute definition's default",
    yaml: "units: mm\nblocks:\n  B: {objects: [{attdef: {tag: T, position: [0, 0, 0], height: 1, default: V}}]}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"objects": [{"attdef": {"tag": "T", "position": [0, 0, 0], "height": 1, "default": V}}]}}}"#,
};

const PROMPT: Place = Place {
    name: "an attribute definition's prompt",
    yaml: "units: mm\nblocks:\n  B: {objects: [{attdef: {tag: T, position: [0, 0, 0], height: 1, prompt: V}}]}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"objects": [{"attdef": {"tag": "T", "position": [0, 0, 0], "height": 1, "prompt": V}}]}}}"#,
};

const BLOCK_NAME: Place = Place {
    name: "a block's name",
    yaml: "units: mm\nblocks:\n  V: {objects: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}]}\n",
    json: r#"{"units": "mm", "blocks": {V: {"objects": [{"line": {"from": [0, 0, 0], "to": [1, 0, 0]}}]}}}"#,
};

const BASE_POINT: Place = Place {
    name: "a block's base point",
    yaml: "units: mm\nblocks:\n  B: {base_point: V, objects: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}]}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"base_point": V, "objects": [{"line": {"from": [0, 0, 0], "to": [1, 0, 0]}}]}}}"#,
};

const BLOCK_OBJECTS: Place = Place {
    name: "a block's objects",
    yaml: "units: mm\nblocks:\n  B: {objects: V}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"objects": V}}}"#,
};

const ATTDEF_CELL: Place = Place {
    name: "an attribute definition's cell",
    yaml: "units: mm\nblocks:\n  B: {objects: [{attdef: {tag: T, position: [0, 0, 0], height: 1, cell: V}}]}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"objects": [{"attdef": {"tag": "T", "position": [0, 0, 0], "height": 1, "cell": V}}]}}}"#,
};

const TEXT_CELL: Place = Place {
    name: "a block's text's cell",
    yaml: "units: mm\nblocks:\n  B: {objects: [{text: {contents: A, position: [0, 0, 0], height: 1, cell: V}}]}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"objects": [{"text": {"contents": "A", "position": [0, 0, 0], "height": 1, "cell": V}}]}}}"#,
};

const MTEXT_CELL: Place = Place {
    name: "a block's mtext's cell",
    yaml: "units: mm\nblocks:\n  B: {objects: [{mtext: {contents: A, position: [0, 0, 0], height: 1, cell: V}}]}\n",
    json: r#"{"units": "mm", "blocks": {"B": {"objects": [{"mtext": {"contents": "A", "position": [0, 0, 0], "height": 1, "cell": V}}]}}}"#,
};

/// `(place in a block library, value as YAML, the same value as JSON, accepted)`.
fn library_cases() -> Vec<Case> {
    let mut cases = [
        bare_null(&TAG, false, Some(("'NULL'", true))),
        bare_null(&DEFAULT, true, Some(("'null'", true))),
        bare_null(&PROMPT, true, Some(("'null'", true))),
    ]
    .concat();
    cases.extend(vec![
        // A cell: two opposite corners, each a point.
        (
            &ATTDEF_CELL,
            "[[0, 0, 0], [5, 2, 0]]",
            json!([[0, 0, 0], [5, 2, 0]]),
            true,
        ),
        (
            &ATTDEF_CELL,
            "[[5, 2, 0], ['0 in', 0, 0]]",
            json!([[5, 2, 0], ["0 in", 0, 0]]),
            true,
        ),
        (&ATTDEF_CELL, "[[0, 0, 0]]", json!([[0, 0, 0]]), false),
        (
            &ATTDEF_CELL,
            "[[0, 0, 0], [1, 1, 0], [2, 2, 0]]",
            json!([[0, 0, 0], [1, 1, 0], [2, 2, 0]]),
            false,
        ),
        (
            &ATTDEF_CELL,
            "[[0, 0], [1, 1]]",
            json!([[0, 0], [1, 1]]),
            false,
        ),
        (&ATTDEF_CELL, "[0, 0, 1, 1]", json!([0, 0, 1, 1]), false),
        (
            &TEXT_CELL,
            "[[-1, -1, 0], [5, 2, 0]]",
            json!([[-1, -1, 0], [5, 2, 0]]),
            true,
        ),
        (&TEXT_CELL, "box", json!("box"), false),
        // Multiline text takes no cell.
        (
            &MTEXT_CELL,
            "[[0, 0, 0], [5, 2, 0]]",
            json!([[0, 0, 0], [5, 2, 0]]),
            false,
        ),
        // A tag: capitals, no spaces.
        (&TAG, "TAG", json!("TAG"), true),
        (&TAG, "'TAG_2'", json!("TAG_2"), true),
        (&TAG, "ÉTAT", json!("ÉTAT"), true),
        (&TAG, "'100'", json!("100"), true),
        (&TAG, "100", json!(100), true),
        (&TAG, "TRUE", json!(true), true),
        (&TAG, "Tag", json!("Tag"), false),
        (&TAG, "état", json!("état"), false),
        (&TAG, "'A B'", json!("A B"), false),
        (&TAG, "''", json!(""), false),
        // invisible: true or false, quoted or not.
        (&INVISIBLE, "true", json!(true), true),
        (&INVISIBLE, "'false'", json!("false"), true),
        (&INVISIBLE, "yes", json!("yes"), false),
        (&INVISIBLE, "'True'", json!("True"), false),
        // A default: one line of text, which may be empty.
        (&DEFAULT, "''", json!(""), true),
        (&DEFAULT, "A-1", json!("A-1"), true),
        (&DEFAULT, "5", json!(5), true),
        (&DEFAULT, "'\\U+0041'", json!("\\U+0041"), false),
        (&DEFAULT, "\"A\\tB\"", json!("A\tB"), false),
        // A prompt: one line of text.
        (&PROMPT, "Enter the tag", json!("Enter the tag"), true),
        (&PROMPT, "\"A\\nB\"", json!("A\nB"), false),
        // A block's name: as a layer's name.
        (&BLOCK_NAME, "MARKER", json!("MARKER"), true),
        (&BLOCK_NAME, "'TAG BUBBLE'", json!("TAG BUBBLE"), true),
        (&BLOCK_NAME, "'A/B'", json!("A/B"), false),
        (&BLOCK_NAME, "' A'", json!(" A"), false),
        // A base point: a point.
        (&BASE_POINT, "[1, 2, 0]", json!([1, 2, 0]), true),
        (&BASE_POINT, "['1 in', 0, 0]", json!(["1 in", 0, 0]), true),
        (&BASE_POINT, "[1, 2]", json!([1, 2]), false),
        // A block's objects: at least one, and no inserts.
        (
            &BLOCK_OBJECTS,
            "[{circle: {center: [0, 0, 0], radius: 1}}]",
            json!([{"circle": {"center": [0, 0, 0], "radius": 1}}]),
            true,
        ),
        (&BLOCK_OBJECTS, "[]", json!([]), false),
        (
            &BLOCK_OBJECTS,
            "[{insert: {block: B, position: [0, 0, 0]}}]",
            json!([{"insert": {"block": "B", "position": [0, 0, 0]}}]),
            false,
        ),
        // A block's objects take no ID or name: the insert is the object on a sheet.
        (
            &BLOCK_OBJECTS,
            "[{circle: {center: [0, 0, 0], radius: 1, id: a}}]",
            json!([{"circle": {"center": [0, 0, 0], "radius": 1, "id": "a"}}]),
            false,
        ),
        (
            &BLOCK_OBJECTS,
            "[{circle: {center: [0, 0, 0], radius: 1, name: a}}]",
            json!([{"circle": {"center": [0, 0, 0], "radius": 1, "name": "a"}}]),
            false,
        ),
    ]);
    cases
}

#[test]
fn the_block_library_schema_accepts_exactly_the_values_the_reader_accepts() {
    assert_reader_and_schema_agree(&block_library_schema(), library_cases(), |yaml| {
        parse_block_library(yaml).map(|_| ())
    });
}

/// Asserts that the reader and the schema both accept exactly the cases
/// marked accepted, and lists every case where either disagrees.
fn assert_reader_and_schema_agree(
    schema: &Value,
    cases: Vec<Case>,
    read: impl Fn(&str) -> Result<(), Error>,
) {
    let validator = jsonschema::validator_for(schema).expect("the schema is a valid JSON Schema");
    let mut disagreements = Vec::new();
    for (place, yaml_value, json_value, accepted) in cases {
        let yaml = place.yaml.replace('V', yaml_value);
        let json: Value = serde_json::from_str(&place.json.replace('V', &json_value.to_string()))
            .expect("the JSON case is JSON");
        let reader = read(&yaml);
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
