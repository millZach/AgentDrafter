//! The block parts of the schemas: a sheet's `block_library` and its
//! inserts, and a block library's blocks and attribute definitions, built
//! from the same tables `blocks.rs` checks keys against.
//!
//! What JSON Schema cannot see is left to the reader, as everywhere in the
//! schema: whether the block an insert names is in the library, which tags
//! its block defines, a scale above 0 when it is written as a string, and
//! whether a name or a tag is given twice.

use serde_json::{json, Map, Value};

use super::blocks::{BLOCK_KEYS, FORBIDDEN_IN_PATHS, LONGEST_NAME, LONGEST_TAG};
use super::schema::{object_def, NUMBER};
use crate::model::LAYER_ZERO;
use crate::text::Justify;

/// Control characters, as a regular expression's character class holds them.
pub(super) const CONTROL: &str = "\\x00-\\x1f\\x7f-\\x9f";

/// The schema of a sheet's `block_library` key.
pub(super) fn block_library_property() -> Value {
    relative_path_property(format!(
        "The block library the sheet's inserts draw their blocks from: the path of its file, \
         relative to the sheet description's folder, with / between folders, such as \
         blocks.yaml or ../library/blocks.yaml. It holds none of {FORBIDDEN_IN_PATHS}. The \
         library is in the sheet's declared unit."
    ))
}

/// The schema of a path written relative to a folder, with `/` between
/// folders, as `blocks.rs` reads one, with `description`.
pub(super) fn relative_path_property(description: String) -> Value {
    let forbidden = FORBIDDEN_IN_PATHS.replace('\\', "\\\\");
    let part = format!("[^/{forbidden}{CONTROL}]+");
    json!({
        "description": description,
        "anyOf": [
            { "type": "string", "pattern": format!("^{part}(/{part})*$") },
            { "type": ["number", "boolean", "null"] },
        ],
    })
}

/// Adds the definitions blocks, attribute definitions and inserts use to `defs`.
pub(super) fn add_defs(defs: &mut Map<String, Value>) {
    defs.insert(
        "attribute_tag".into(),
        json!({
            "description": format!(
                "An attribute tag: the label an insert fills an attribute in by. 1 to \
                 {LONGEST_TAG} characters in capitals, as AutoCAD keeps it, with no spaces and no \
                 control characters. A plain number such as `100` is read as its digits."
            ),
            "anyOf": [
                {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": LONGEST_TAG,
                    "pattern": format!("^[^\\s\\p{{Lowercase}}{CONTROL}]+$"),
                },
                { "type": ["number", "boolean", "null"] },
            ],
        }),
    );
    defs.insert(
        "attribute_value".into(),
        json!({
            "description": "An attribute's value: one line of text, exactly as it shows, which \
                            may be empty. No control characters, and no \\U+ with four \
                            hexadecimal digits or \\M+ with five, which AutoCAD reads as a \
                            character code.",
            "anyOf": [
                {
                    "type": "string",
                    "pattern": format!("^[^{CONTROL}]*$"),
                    "not": { "pattern": "\\\\[Uu]\\+[0-9A-Fa-f]{4}|\\\\[Mm]\\+[0-9A-Fa-f]{5}" },
                },
                { "type": ["number", "boolean", "null"] },
            ],
        }),
    );
    let end = format!("[^\\s{CONTROL}]");
    defs.insert(
        "object_name".into(),
        json!({
            "description": format!(
                "A name an agent selects the object by later, such as jb: 1 to {LONGEST_NAME} \
                 characters, not starting or ending with a space, with no control characters. \
                 Each name is given once on a sheet; names that differ only in case are one name. \
                 A name is not an attribute tag."
            ),
            "anyOf": [
                {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": LONGEST_NAME,
                    "pattern": format!("^{end}([^{CONTROL}]*{end})?$"),
                },
                { "type": ["number", "boolean"] },
            ],
        }),
    );
    defs.insert(
        "insert".into(),
        object_def(
            "A block from the sheet's block library, placed as AutoCAD's INSERT places it: its \
             base point at the position, enlarged by the scale and turned by the rotation, \
             carrying a value for each of its attributes.",
            &["block", "position"],
            json!({
                "block": {
                    "description": "The name of a block in the block library, written exactly as \
                                    it is there.",
                    "type": ["string", "number", "boolean", "null"],
                },
                "position": {
                    "$ref": "#/$defs/point",
                    "description": "Where the block's base point sits.",
                },
                "scale": {
                    "description": "How much the block is enlarged, the same along every axis: a \
                                    number more than 0, with no unit. Without it, 1.",
                    "anyOf": [
                        { "type": "number", "exclusiveMinimum": 0 },
                        { "type": "string", "pattern": format!("^{NUMBER}$") },
                    ],
                },
                "rotation": {
                    "$ref": "#/$defs/angle",
                    "description": "Degrees anticlockwise from the X axis. Without it, 0.",
                },
                "attributes": {
                    "description": "The values of the block's attributes, from tag to value. An \
                                    attribute not given takes its definition's default.",
                    "type": "object",
                    "propertyNames": { "$ref": "#/$defs/attribute_tag" },
                    "additionalProperties": { "$ref": "#/$defs/attribute_value" },
                },
                "name": { "$ref": "#/$defs/object_name" },
            }),
        ),
    );
    defs.insert(
        "attdef".into(),
        object_def(
            "An attribute definition, AutoCAD's ATTDEF: a labelled text field each insert of the \
             block fills in. It draws as a single line of text does, moved, scaled and turned with \
             each insert.",
            &["tag", "position", "height"],
            json!({
                "tag": { "$ref": "#/$defs/attribute_tag" },
                "prompt": {
                    "description": "What AutoCAD asks for when a person inserts the block: one \
                                    line of text with no control characters. Without it, empty, \
                                    and AutoCAD asks with the tag.",
                    "anyOf": [
                        { "type": "string", "pattern": format!("^[^{CONTROL}]*$") },
                        { "type": ["number", "boolean", "null"] },
                    ],
                },
                "default": {
                    "$ref": "#/$defs/attribute_value",
                    "description": "The value an insert takes when it gives none. Without it, \
                                    empty.",
                },
                "position": {
                    "$ref": "#/$defs/point",
                    "description": "The point the attribute's text is justified at, in the \
                                    block's own coordinates.",
                },
                "height": { "$ref": "#/$defs/height" },
                "style": { "$ref": "#/$defs/style_name" },
                "width_factor": {
                    "$ref": "#/$defs/width_factor",
                    "description": "Without it, the style's width factor.",
                },
                "rotation": {
                    "$ref": "#/$defs/angle",
                    "description": "Degrees anticlockwise from the block's X axis. Without it, 0.",
                },
                "justify": {
                    "description": "Where the text sits against its position, as for a text. \
                                    Without it, left.",
                    "enum": Justify::ALL.map(Justify::name),
                },
                "invisible": {
                    "description": "Whether the attribute is hidden on every insert, as \
                                    AutoCAD's Invisible mode hides it. Without it, false.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
            }),
        ),
    );
    let [base_point_key, objects_key] = BLOCK_KEYS;
    defs.insert(
        "block".into(),
        json!({
            "description": format!(
                "A block: objects defined once and inserted many times. An object on layer \
                 `{LAYER_ZERO}` draws with the layer of the insert, as in AutoCAD."
            ),
            "type": "object",
            "required": [objects_key],
            "additionalProperties": false,
            "properties": {
                base_point_key: {
                    "$ref": "#/$defs/point",
                    "description": "The point of the block that sits at an insert's position, in \
                                    the block's own coordinates. Without it, [0, 0, 0].",
                },
                objects_key: {
                    "description": "The block's objects, in drawing order: at least one. Each is \
                                    a mapping with one key, its type. A block holds the objects a \
                                    sheet holds and attribute definitions, but no inserts.",
                    "type": "array",
                    "minItems": 1,
                    "items": { "oneOf": super::schema::object_choices(&super::objects::BLOCK_OBJECT_TYPES) },
                },
            },
        }),
    );
}
