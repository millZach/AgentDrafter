//! The sheet description's schema, as a JSON Schema (draft 2020-12) an
//! agent can read before it writes a sheet description.
//!
//! The schema is built from the same tables the reader checks keys against:
//! the object types and their keys, the units, the colour names, the
//! lineweights, the text style keys and the justifications. A test holds the two together, so a key cannot be added to
//! one and not the other.
//!
//! JSON Schema describes JSON values, and a YAML reader that guesses types
//! turns `100` into a number and `4 in` into a string, so a length here is a
//! number or a string that reads as one. Quoted and plain scalars mean the
//! same in a sheet description, so every value the reader takes quoted, such
//! as `'42'` for a colour or `'true'` for `closed`, the schema takes as a
//! string; `tests/schema.rs` feeds the same values to both. What JSON Schema
//! cannot see it leaves to the reader: a type-guessing reader turns `+5` and
//! `5` into one number, and a radius above 0 or a degree below the number of
//! control points compares one value with another. The rules of the strict
//! YAML subset (no anchors, tags or directives, every key once) are not JSON
//! Schema either; the top-level description states them.

use serde_json::{json, Map, Value};

use super::blocks::LIBRARY_KEYS;
use super::ids::{ID_PUNCTUATION, ID_SEPARATOR, LONGEST_ID, SHEET_OBJECT_KEYS};
use super::layers::{
    colour_names, COLOUR_NAMES, FORBIDDEN_IN_NAMES, LONGEST_DESCRIPTION, LONGEST_NAME,
    LONGEST_PATTERN,
};
use super::objects::{FEWEST_FIT_POINTS, HIGHEST_DEGREE, OBJECT_TYPES};
use super::{block_schema, hatch_schema, SHEET_KEYS};
use crate::model::{Lineweight, CONTINUOUS, FIT_DEGREE, LAYER_ZERO, WHITE};
use crate::text::{
    Attachment, Justify, FORBIDDEN_IN_FONT_NAMES, STANDARD, STANDARD_FONT, WIDTH_FACTORS,
};
use crate::units::Unit;

/// A plain decimal number, as `docs/sheet-description.md` defines it.
pub(super) const NUMBER: &str = "-?[0-9]+(\\.[0-9]+)?([eE][-+]?[0-9]+)?";

/// The strict YAML subset, as both schemas' descriptions state it.
pub(super) const SUBSET: &str =
    "It is written in a strict subset of YAML 1.2: one document; no anchors, \
                      aliases, tags or directives; every key is text and appears once in its \
                      mapping; a key not in this schema is an error that lists the keys allowed \
                      there. Every scalar is read as text and given its meaning here, so `yes`, \
                      `null` and `0x10` are never booleans, nulls or numbers. Comments, block \
                      style and flow style are all allowed. Lengths are in the declared unit; \
                      angles are in degrees, anticlockwise; a lineweight is in millimetres.";

/// The sheet description's schema.
pub fn sheet_description_schema() -> Value {
    let [units_key, block_library_key, linetypes_key, layers_key, hatch_patterns_key, text_styles_key, objects_key] =
        SHEET_KEYS;
    let (linetypes, layers, hatch_patterns, text_styles) = table_properties("sheet");
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AgentDrafter sheet description",
        "description": format!(
            "The text file that states what is on a new sheet; a build turns it into a DWG. \
             {SUBSET}"
        ),
        "type": "object",
        "required": [units_key, objects_key],
        "additionalProperties": false,
        "properties": {
            units_key: units_property("sheet"),
            block_library_key: block_schema::block_library_property(),
            linetypes_key: linetypes,
            layers_key: layers,
            hatch_patterns_key: hatch_patterns,
            text_styles_key: text_styles,
            objects_key: {
                "description": "The objects on the sheet, in drawing order. Each is a mapping \
                                with one key, its type.",
                "type": "array",
                "items": { "oneOf": object_choices(&OBJECT_TYPES) },
            },
        },
        "$defs": sheet_defs(),
    })
}

/// Every definition the sheet description's schema refers to: the
/// definitions both schemas share, with an object's ID among the keys of
/// every object a sheet holds. A block's objects take no ID.
fn sheet_defs() -> Map<String, Value> {
    let mut defs = defs();
    for (name, _) in OBJECT_TYPES {
        if let Some(properties) = defs
            .get_mut(name)
            .and_then(|def| def["properties"].as_object_mut())
        {
            for key in SHEET_OBJECT_KEYS {
                properties.insert(key.into(), json!({ "$ref": "#/$defs/object_id" }));
            }
        }
    }
    let punctuation = ID_PUNCTUATION
        .chars()
        .map(|c| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(", ");
    defs.insert(
        "object_id".into(),
        json!({
            "description": format!(
                "The object's ID, which a generator may write: 1 to {LONGEST_ID} ASCII letters, \
                 digits and {punctuation}, such as pump-1. The object's ID in the drawing set is \
                 the sheet's name, `{ID_SEPARATOR}` and this, such as DEMO-001/pump-1, and is \
                 written into the DWG with the object. Each ID is given once in a file, and IDs \
                 that differ only in case are one ID. Without it, the object's ID is its place \
                 in the file, such as objects[3], which moves when an object is added or taken \
                 away before it. A plain number, true, false or null is read as its text, as \
                 every scalar is."
            ),
            "anyOf": [
                {
                    "type": "string",
                    "pattern": format!("^[A-Za-z0-9{}]+$", regex_class(ID_PUNCTUATION)),
                    "maxLength": LONGEST_ID,
                },
                { "type": ["number", "boolean", "null"] },
            ],
        }),
    );
    defs
}

/// `characters` escaped for a regular expression's character class.
fn regex_class(characters: &str) -> String {
    characters
        .chars()
        .map(|c| match c {
            '-' | '\\' | ']' | '^' => format!("\\{c}"),
            c => c.to_string(),
        })
        .collect()
}

/// The block library's schema.
pub fn block_library_schema() -> Value {
    let [units_key, linetypes_key, layers_key, hatch_patterns_key, text_styles_key, blocks_key] =
        LIBRARY_KEYS;
    let (linetypes, layers, hatch_patterns, text_styles) = table_properties("block library");
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AgentDrafter block library",
        "description": format!(
            "The blocks a drawing set shares, each defined once, with the layers, linetypes, \
             hatch patterns and text styles their objects use. A sheet description names it under \
             block_library and inserts its blocks. {SUBSET}"
        ),
        "type": "object",
        "required": [units_key, blocks_key],
        "additionalProperties": false,
        "properties": {
            units_key: units_property("block library"),
            linetypes_key: linetypes,
            layers_key: layers,
            hatch_patterns_key: hatch_patterns,
            text_styles_key: text_styles,
            blocks_key: {
                "description": "The blocks, from name to block. A block name follows the rules for \
                                a layer name.",
                "type": "object",
                "propertyNames": { "$ref": "#/$defs/name" },
                "additionalProperties": { "$ref": "#/$defs/block" },
            },
        },
        "$defs": defs(),
    })
}

/// One choice of object for each of `types`: a mapping with that one key.
pub(super) fn object_choices(types: &[(&str, &[&str])]) -> Vec<Value> {
    types
        .iter()
        .map(|(name, _)| {
            json!({
                "type": "object",
                "required": [name],
                "additionalProperties": false,
                "properties": { *name: { "$ref": format!("#/$defs/{name}") } },
            })
        })
        .collect()
}

/// The `units` key of a sheet or a block library.
fn units_property(whose: &str) -> Value {
    let description = if whose == "sheet" {
        "The declared unit: every length on the sheet is in it unless it carries its own. It \
         becomes the DWG's insertion unit. There is no default."
    } else {
        "The declared unit: every length in the library is in it unless it carries its own. A \
         sheet that names the library is in the same unit. There is no default."
    };
    json!({ "description": description, "enum": Unit::ALL.map(Unit::symbol) })
}

/// The `linetypes`, `layers`, `hatch_patterns` and `text_styles` keys of a
/// sheet or a block library: `whose` they are.
fn table_properties(whose: &str) -> (Value, Value, Value, Value) {
    (
        json!({
            "description": format!(
                "The {whose}'s linetypes, from name to linetype. `{CONTINUOUS}`, `ByLayer` and \
                 `ByBlock` are in every drawing and cannot be defined again."
            ),
            "type": "object",
            "propertyNames": { "$ref": "#/$defs/name" },
            "additionalProperties": { "$ref": "#/$defs/linetype" },
        }),
        json!({
            "description": format!(
                "The {whose}'s layers, from name to layer. Layer `{LAYER_ZERO}` is in every \
                 drawing; listing it sets its colour, linetype and lineweight."
            ),
            "type": "object",
            "propertyNames": { "$ref": "#/$defs/name" },
            "additionalProperties": { "$ref": "#/$defs/layer" },
        }),
        hatch_schema::hatch_patterns_property(whose),
        json!({
            "description": format!(
                "The {whose}'s text styles, from name to text style. Style `{STANDARD}` is in \
                 every drawing, with the font `{STANDARD_FONT}`; listing it sets its font and \
                 width factor. It is written `{STANDARD}` exactly: in another case it is an error."
            ),
            "type": "object",
            "propertyNames": { "$ref": "#/$defs/name" },
            "additionalProperties": { "$ref": "#/$defs/text_style" },
        }),
    )
}

/// Every definition the two schemas refer to.
fn defs() -> Map<String, Value> {
    let units = Unit::ALL.map(Unit::symbol);
    let unit_pattern = units.join("|");
    let lineweights: Vec<f64> = Lineweight::ALL.iter().map(|w| w.millimetres()).collect();
    // A lineweight written out, as the reader takes it: any zeros before
    // the whole number and after the fraction, such as 0.50 or 1.0.
    let lineweight_pattern = Lineweight::ALL
        .map(|w| match w.to_string().split_once('.') {
            Some((whole, fraction)) => format!("0*{whole}\\.{fraction}0*"),
            None => format!("0*{w}(\\.0+)?"),
        })
        .join("|");
    let degrees = (1..=HIGHEST_DEGREE)
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("|");
    let colours = COLOUR_NAMES.map(|(name, _)| name);

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
                "A layer, linetype, text style or block name: 1 to {LONGEST_NAME} characters, not starting or \
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
            "type": ["string", "number", "boolean", "null"],
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
                "color": {
                    "description": format!(
                        "An AutoCAD colour index from 1 to 255, or one of {} (1 to 7). \
                         Without it, {WHITE} (white).",
                        colour_names()
                    ),
                    "anyOf": [
                        { "type": "integer", "minimum": 1, "maximum": 255 },
                        {
                            "type": "string",
                            "pattern": "^0*([1-9]|[1-9][0-9]|1[0-9][0-9]|2[0-4][0-9]|25[0-5])$",
                        },
                        { "enum": colours },
                    ],
                },
                "linetype": {
                    "description": format!(
                        "`{CONTINUOUS}`, the solid line every drawing has, or a name under \
                         `linetypes`, written exactly as it is there. Without it, `{CONTINUOUS}`."
                    ),
                    "type": ["string", "number", "boolean", "null"],
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
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
            }),
        ),
    );
    defs.insert(
        "arc".into(),
        object_def(
            "Part of a circle, drawn anticlockwise from its start angle to its end angle, in the \
             plane parallel to XY at its centre's z.",
            &["center", "radius", "start_angle", "end_angle"],
            json!({
                "center": { "$ref": "#/$defs/point" },
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
            &["center", "radius"],
            json!({
                "center": { "$ref": "#/$defs/point" },
                "radius": { "$ref": "#/$defs/length", "description": "A length more than 0." },
            }),
        ),
    );
    defs.insert(
        "ellipse".into(),
        object_def(
            "An ellipse in the plane parallel to XY at its centre's z. With start_angle and \
             end_angle, only the part from one to the other, anticlockwise.",
            &["center", "major_axis", "ratio"],
            json!({
                "center": { "$ref": "#/$defs/point" },
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
                        "anyOf": [
                            { "type": "integer", "minimum": 1, "maximum": HIGHEST_DEGREE },
                            { "type": "string", "pattern": format!("^0*({degrees})$") },
                        ],
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
    hatch_schema::add_defs(&mut defs);
    text_defs(&mut defs);
    block_schema::add_defs(&mut defs);
    defs
}

/// The definitions text styles, text and multiline text need.
fn text_defs(defs: &mut Map<String, Value>) {
    let (narrowest, widest) = WIDTH_FACTORS;
    let forbidden = FORBIDDEN_IN_FONT_NAMES.replace('\\', "\\\\");
    defs.insert(
        "font".into(),
        json!({
            "description": "A font, by its file name alone: an AutoCAD SHX font such as \
                            `romans.shx` or a TrueType font such as `arial.ttf`. The extension \
                            may be in any case. The DWG names the font, and AutoCAD finds it on \
                            the machine that opens it.",
            "type": "string",
            "pattern": format!("^[^{forbidden}\\x00-\\x1f]+\\.([sS][hH][xX]|[tT][tT][fF])$"),
        }),
    );
    defs.insert(
        "width_factor".into(),
        json!({
            "description": format!(
                "How wide characters are drawn, as a share of their normal width: 1 is normal, \
                 0.8 narrower. From {narrowest} to {widest}."
            ),
            "anyOf": [
                { "type": "number", "minimum": narrowest, "maximum": widest },
                { "type": "string", "pattern": format!("^{NUMBER}$") },
            ],
        }),
    );
    defs.insert(
        "text_style".into(),
        json!({
            "description": "A text style: the font text draws with, and how wide.",
            "type": "object",
            "required": ["font"],
            "additionalProperties": false,
            "properties": {
                "font": { "$ref": "#/$defs/font" },
                "width_factor": {
                    "$ref": "#/$defs/width_factor",
                    "description": "The width factor text in the style takes unless it states \
                                    its own. Without it, 1.",
                },
            },
        }),
    );
    defs.insert(
        "style_name".into(),
        json!({
            "description": format!(
                "The text style the text draws with: `{STANDARD}`, which every drawing has, or a \
                 name under `text_styles`, written exactly as it is there. Without it, \
                 `{STANDARD}`."
            ),
            "type": ["string", "number", "boolean", "null"],
        }),
    );
    defs.insert(
        "height".into(),
        json!({
            "$ref": "#/$defs/length",
            "description": "The height of the capitals: a length more than 0.",
        }),
    );
    defs.insert(
        "text".into(),
        object_def(
            "A single line of text, as AutoCAD's TEXT draws it.",
            &["contents", "position", "height"],
            json!({
                "contents": {
                    "description": "What the text says, exactly as it shows: one line, not \
                                    empty, with no control characters. `%` shows as a percent \
                                    sign, however many there are.",
                    "type": ["string", "number", "boolean", "null"],
                },
                "position": {
                    "$ref": "#/$defs/point",
                    "description": "The point the text is justified at.",
                },
                "height": { "$ref": "#/$defs/height" },
                "style": { "$ref": "#/$defs/style_name" },
                "width_factor": {
                    "$ref": "#/$defs/width_factor",
                    "description": "Without it, the style's width factor.",
                },
                "rotation": {
                    "$ref": "#/$defs/angle",
                    "description": "Degrees anticlockwise from the X axis. Without it, 0.",
                },
                "justify": {
                    "description": "Where the text sits against its position. left, center and \
                                    right sit on the baseline; middle centres the text both ways; \
                                    the others name the top, middle or bottom of the capitals \
                                    (bottom is the lowest descender). Without it, left.",
                    "enum": Justify::ALL.map(Justify::name),
                },
            }),
        ),
    );
    defs.insert(
        "mtext".into(),
        object_def(
            "Text of one or more lines in a box, as AutoCAD's MTEXT draws it.",
            &["contents", "position", "height"],
            json!({
                "contents": {
                    "description": "What the text says, exactly as it shows, not empty: a line \
                                    break starts a new line, and line breaks at the end are \
                                    dropped. No control characters but line breaks. A \\, { or } \
                                    shows as itself; formatting codes are not read.",
                    "type": ["string", "number", "boolean", "null"],
                },
                "position": {
                    "$ref": "#/$defs/point",
                    "description": "The point the text's box is attached at.",
                },
                "height": { "$ref": "#/$defs/height" },
                "width": {
                    "$ref": "#/$defs/length",
                    "description": "The width lines wrap in: a length of 0 or more. Without it, \
                                    0, and lines break only where the contents break them.",
                },
                "style": { "$ref": "#/$defs/style_name" },
                "width_factor": {
                    "$ref": "#/$defs/width_factor",
                    "description": "Without it, the style's width factor.",
                },
                "rotation": {
                    "$ref": "#/$defs/angle",
                    "description": "Degrees anticlockwise from the X axis. Without it, 0.",
                },
                "justify": {
                    "description": "Which point of the text's box sits at its position. \
                                    Without it, top_left.",
                    "enum": Attachment::ALL.map(Attachment::name),
                },
            }),
        ),
    );
}

/// The definition of one object type: its own `properties` and `layer`.
pub(super) fn object_def(description: &str, required: &[&str], properties: Value) -> Value {
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
    use crate::description::blocks::BLOCK_KEYS;
    use crate::description::layers::{LAYER_KEYS, LINETYPE_KEYS};
    use crate::description::objects::BLOCK_OBJECT_TYPES;
    use crate::description::text::TEXT_STYLE_KEYS;

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
        assert_eq!(
            keys(&defs["text_style"]["properties"]),
            sorted(&TEXT_STYLE_KEYS)
        );
        let listed: Vec<String> = schema["properties"]["objects"]["items"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| keys(&o["properties"])[0].clone())
            .collect();
        let types: Vec<String> = OBJECT_TYPES.iter().map(|(t, _)| (*t).to_owned()).collect();
        assert_eq!(listed, types);
        for (name, object_keys) in OBJECT_TYPES {
            let on_a_sheet: Vec<&str> = object_keys
                .iter()
                .chain(&SHEET_OBJECT_KEYS)
                .copied()
                .collect();
            assert_eq!(
                keys(&defs[name]["properties"]),
                sorted(&on_a_sheet),
                "the keys of {name}"
            );
        }
    }

    #[test]
    fn the_block_library_schema_and_the_reader_know_the_same_keys() {
        let schema = block_library_schema();
        assert_eq!(keys(&schema["properties"]), sorted(&LIBRARY_KEYS));
        let defs = &schema["$defs"];
        assert_eq!(keys(&defs["block"]["properties"]), sorted(&BLOCK_KEYS));
        let listed: Vec<String> = defs["block"]["properties"]["objects"]["items"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| keys(&o["properties"])[0].clone())
            .collect();
        let types: Vec<String> = BLOCK_OBJECT_TYPES
            .iter()
            .map(|(t, _)| (*t).to_owned())
            .collect();
        assert_eq!(listed, types);
        for (name, object_keys) in BLOCK_OBJECT_TYPES {
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
        for schema in [sheet_description_schema(), block_library_schema()] {
            walk(&schema, &schema["$defs"]);
        }
    }
}
