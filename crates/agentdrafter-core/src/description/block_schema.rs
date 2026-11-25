//! The block parts of the schemas: a sheet's `block_library` and its
//! inserts, and a block library's blocks and attribute definitions, built
//! from the same tables `blocks.rs` checks keys against.
//!
//! What JSON Schema cannot see is left to the reader, as everywhere in the
//! schema: whether the block an insert names is in the library, which tags
//! its block defines, a scale above 0 when it is written as a string, and
//! whether a name or a tag is given twice.

use serde_json::{json, Map, Value};

use super::blocks::{BLOCK_KEYS, CELL_KEY, FORBIDDEN_IN_PATHS, LONGEST_TAG};
use super::ids::LONGEST_NAME;
use super::schema::{object_def, optional_text, NUMBER, TEXT_TYPES};
use crate::model::LAYER_ZERO;
use crate::text::Justify;

/// Control characters, as a regular expression's character class holds them.
pub(super) const CONTROL: &str = "\\x00-\\x1f\\x7f-\\x9f";

/// The schema of a sheet's `block_library` key, which may be left out.
pub(super) fn block_library_property() -> Value {
    optional_text(relative_path_property(format!(
        "The block library the sheet's inserts draw their blocks from: the path of its file, \
         relative to the sheet description's folder, with / between folders, such as \
         blocks.yaml or ../library/blocks.yaml. It holds none of {FORBIDDEN_IN_PATHS}. The \
         library is in the sheet's declared unit."
    )))
}

/// The schema of a path written relative to a folder, with `/` between
/// folders, as `blocks.rs` reads one, with `description`, where it is
/// required.
pub(super) fn relative_path_property(description: String) -> Value {
    let forbidden = FORBIDDEN_IN_PATHS.replace('\\', "\\\\");
    let part = format!("[^/{forbidden}{CONTROL}]+");
    json!({
        "description": description,
        "anyOf": [
            { "type": "string", "pattern": format!("^{part}(/{part})*$") },
            { "type": ["number", "boolean"] },
        ],
    })
}

/// Adds to `defs` the cell a block's text and attribute definitions take,
/// for the block library's schema: a sheet's text takes none.
pub(super) fn add_cells(defs: &mut Map<String, Value>) {
    defs.insert(
        "cell".into(),
        json!({
            "description": "The box the text draws within, such as its title-block cell: two \
                            opposite corners [[x, y, z], [x, y, z]], in the block's own \
                            coordinates, which differ in x and in y. On each insert it is moved, \
                            scaled and turned with the insert, and the checker reports text that \
                            draws past it. It is seen from above, so z is not used. Only the \
                            checker reads it: a DWG is the same with it or without it. Without \
                            it, the text is not checked against a cell.",
            "type": "array",
            "items": { "$ref": "#/$defs/point" },
            "minItems": 2,
            "maxItems": 2,
        }),
    );
    for kind in ["text", "attdef"] {
        if let Some(properties) = defs
            .get_mut(kind)
            .and_then(|def| def["properties"].as_object_mut())
        {
            properties.insert(CELL_KEY.into(), json!({ "$ref": "#/$defs/cell" }));
        }
    }
}

/// Adds the definitions blocks, attribute definitions and inserts use to `defs`.
pub(super) fn add_defs(defs: &mut Map<String, Value>) {
    super::dynamic_block::add_schema_defs(defs);
    defs.insert(
        "attribute_tag".into(),
        json!({
            "description": format!(
                "An attribute tag: the label an insert fills an attribute in by. 1 to \
                 {LONGEST_TAG} characters with no spaces and no control characters; punctuation \
                 such as NOTE#1, A/B or X.Y is kept as written. AutoCAD keeps a tag in \
                 capitals, so one written in lowercase is taken in capitals, and the reply says \
                 so. A plain number such as `100` is read as its digits."
            ),
            "anyOf": [
                {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": LONGEST_TAG,
                    "pattern": format!("^[^\\s{CONTROL}]+$"),
                },
                { "type": ["number", "boolean"] },
            ],
        }),
    );
    defs.insert(
        "attribute_value".into(),
        optional_text(json!({
            "description": "An attribute's value, exactly as it shows, which may be empty: one \
                            line for a single-line attribute, or one or more lines (\\n starts a \
                            new line) for a multiline attribute. No control characters other than \
                            line breaks, and no \\U+ with four hexadecimal digits or \\M+ with \
                            five, which AutoCAD reads as a character code.",
            "anyOf": [
                {
                    "type": "string",
                    "pattern": "^[^\\x00-\\x09\\x0b\\x0c\\x0e-\\x1f\\x7f-\\x9f]*$",
                    "not": { "pattern": "\\\\[Uu]\\+[0-9A-Fa-f]{4}|\\\\[Mm]\\+[0-9A-Fa-f]{5}" },
                },
                { "type": ["number", "boolean"] },
            ],
        })),
    );
    let end = format!("[^\\s{CONTROL}]");
    defs.insert(
        "object_name".into(),
        optional_text(json!({
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
        })),
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
                    "type": TEXT_TYPES,
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
                "mirrored": {
                    "description": "Whether the block is mirrored, its X axis reversed before \
                                    it is turned, as AutoCAD's MIRROR leaves an insert, which a \
                                    DWG keeps as a negative X scale: true or false. Its \
                                    attributes still read forwards. Without it, false.",
                    "type": "boolean",
                },
                "readable_attributes": {
                    "description": "Whether its attributes read forwards, left to right or \
                                    upwards, however it is turned, as a mirror leaves them: true \
                                    or false. A mirrored insert's always do; one mirrored back \
                                    keeps them so. Without it, false: they turn with it.",
                    "type": "boolean",
                },
                "attributes": {
                    "description": "The values of the block's attributes, from tag to value. A \
                                    tag the block defines more than once takes a list of values, \
                                    one for each in the order the block defines them, which may \
                                    be shorter than that; a tag it defines once takes one value. \
                                    An attribute not given, or given as null, takes its \
                                    definition's default.",
                    "type": "object",
                    "propertyNames": { "$ref": "#/$defs/attribute_tag" },
                    "additionalProperties": {
                        "anyOf": [
                            { "$ref": "#/$defs/attribute_value" },
                            {
                                "type": "array",
                                "items": { "$ref": "#/$defs/attribute_value" },
                            },
                        ],
                    },
                },
                "attribute_fields": {
                    "description": "AutoCAD fields attached to the insert's carried attributes, \
                                    from tag to field definition (or a list of field definitions \
                                    or nulls for a tag the block defines more than once).",
                    "type": "object",
                    "propertyNames": { "$ref": "#/$defs/attribute_tag" },
                    "additionalProperties": {
                        "anyOf": [
                            { "$ref": "#/$defs/field" },
                            { "type": "null" },
                            {
                                "type": "array",
                                "items": {
                                    "anyOf": [
                                        { "$ref": "#/$defs/field" },
                                        { "type": "null" },
                                    ],
                                },
                            },
                        ],
                    },
                },
                "state": { "$ref": "#/$defs/dynamic_state" },
                "annotative": { "$ref": "#/$defs/annotative" },
                "annotation_scales": { "$ref": "#/$defs/object_annotation_scales" },
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
                "prompt": optional_text(json!({
                    "description": "What AutoCAD asks for when a person inserts the block: one \
                                    line of text with no control characters. Without it, empty, \
                                    and AutoCAD asks with the tag.",
                    "anyOf": [
                        { "type": "string", "pattern": format!("^[^{CONTROL}]*$") },
                        { "type": ["number", "boolean"] },
                    ],
                })),
                "default": {
                    "$ref": "#/$defs/attribute_value",
                    "description": "The value an insert takes when it gives none; a constant \
                                    attribute's value on every insert. Without it, empty.",
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
                "constant": {
                    "description": "Whether the attribute's value is its `default` on every \
                                    insert, as AutoCAD's Constant mode: an insert gives no value \
                                    for it. A constant attribute takes no `verify` or `preset`. \
                                    Without it, false.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "verify": {
                    "description": "Whether AutoCAD asks a person to confirm the value they \
                                    type when they insert the block, as its Verify mode does. It \
                                    changes nothing that is drawn. Without it, false.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "preset": {
                    "description": "Whether AutoCAD takes the default without asking a person \
                                    who inserts the block, as its Preset mode does. It changes \
                                    nothing that is drawn. Without it, false.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "lock_position": {
                    "description": "Whether a person in AutoCAD cannot move the attribute apart \
                                    from its insert, as its Lock position mode keeps it. It \
                                    changes nothing that is drawn. Without it, false.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "multiline": {
                    "description": "Whether the attribute is multi-line (AutoCAD's ATTDEF \
                                    Multiple lines mode): its value draws as mtext, wrapped in \
                                    `width`, with line breaks. Without it, false.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "width": {
                    "$ref": "#/$defs/length_not_negative",
                    "description": "For a multi-line attribute (`multiline: true`), the width \
                                    lines wrap in: a length of 0 or more. Without it, 0, and \
                                    lines break only where the value breaks them.",
                },
            }),
        ),
    );
    let [base_point_key, objects_key, parameters_key, annotative_key] = BLOCK_KEYS;
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
                                    sheet holds, inserts of the library's other blocks among \
                                    them, and attribute definitions.",
                    "type": "array",
                    "minItems": 1,
                    "items": { "oneOf": super::schema::object_choices(&super::objects::BLOCK_OBJECT_TYPES) },
                },
                parameters_key: {
                    "description": "Optional dynamic block parameters (visibility states, flips, \
                                    linear stretch distances, rotations, and lookup tables), \
                                    mapped from parameter name to definition.",
                    "type": "object",
                    "additionalProperties": { "$ref": "#/$defs/dynamic_parameter" },
                },
                annotative_key: super::annotation_scale::annotative_property(
                    "Whether inserts of this block are annotative by default. Without it, false."
                ),
            },
        }),
    );
}
