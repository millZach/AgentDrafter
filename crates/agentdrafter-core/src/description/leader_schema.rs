//! The schema of multileader styles, multileaders and older leaders, part
//! of the sheet description's schema ([`super::schema`]).

use serde_json::{json, Map, Value};

use super::schema::{object_def, optional_text, shows_pattern, NUMBER, TEXT_TYPES};
use crate::model::{Arrowhead, BlockConnection, LeaderAttachment, STANDARD_MLEADER_STYLE};
use crate::text::STANDARD;

/// The `mleader_styles` key of a sheet.
pub(super) fn mleader_styles_property() -> Value {
    json!({
        "description": format!(
            "The sheet's multileader styles, from name to multileader style: what a \
             multileader draws with unless it gives its own. Style `{STANDARD_MLEADER_STYLE}` \
             is in every drawing, as AutoCAD 2026's templates have it: on a metric sheet text \
             and arrows 4 mm, a landing 8 mm and a gap of 2 mm, and on an imperial sheet 0.18, \
             0.36 and 0.09 in, text in the text style `{STANDARD}`, a closed filled arrowhead \
             and the landing at the middle of the top line; listing it sets what it draws \
             with. It is written `{STANDARD_MLEADER_STYLE}` exactly: in another case it is an \
             error."
        ),
        "type": "object",
        "propertyNames": { "$ref": "#/$defs/name" },
        "additionalProperties": { "$ref": "#/$defs/mleader_style" },
    })
}

fn text_style_name(without: &str) -> Value {
    optional_text(json!({
        "description": format!(
            "A text style: `{STANDARD}`, which every drawing has, or a name under \
             `text_styles`, written exactly as it is there. Without it, {without}."
        ),
        "type": TEXT_TYPES,
    }))
}

fn arrowhead(without: &str) -> Value {
    json!({
        "description": format!(
            "The arrowhead, of AutoCAD's named arrowheads that AgentDrafter draws exactly: \
             closed_filled, a filled triangle; dot, a filled dot as wide as the arrow is long; \
             open, the triangle's sides; none. Without it, {without}."
        ),
        "enum": Arrowhead::ALL.map(Arrowhead::name),
    })
}

fn attachment(without: &str) -> Value {
    json!({
        "description": format!(
            "Where the landing meets the text, up the text: the top of its box, the middle \
             of its top line, the middle of the text, the middle of its bottom line, or the \
             bottom of its box. AutoCAD's attachments that underline the text wait. Without \
             it, {without}."
        ),
        "enum": LeaderAttachment::ALL.map(LeaderAttachment::name),
    })
}

fn leader_points(description: &str) -> Value {
    json!({
        "description": description,
        "type": "array",
        "items": { "$ref": "#/$defs/point" },
        "minItems": 2,
    })
}

fn leader_text() -> Value {
    json!({
        "description": "What the text says, exactly as it shows: something that shows, so not \
                        empty and not only white space. A line break starts a new line, and \
                        line breaks at the end are dropped. No control characters but line \
                        breaks. A \\, { or } shows as itself.",
        "type": TEXT_TYPES,
        "pattern": shows_pattern(),
    })
}

/// A multileader's block content.
fn block_content() -> Value {
    json!({
        "description": "Block content: a block of the sheet's block library, AutoCAD's block \
                        content, placed at the landing's end as AutoCAD places it and drawn as \
                        an insert of it draws. The block holds lines, polylines, arcs, circles, \
                        ellipses, 2D solids, donuts, hatches, inserts of such blocks and \
                        attributes, but no text, multiline text, constant attribute or spline. A \
                        multileader with block content takes no `gap`.",
        "type": "object",
        "required": ["block"],
        "additionalProperties": false,
        "properties": {
            "block": {
                "description": "The name of a block in the block library, written exactly as it \
                                is there.",
                "type": TEXT_TYPES,
            },
            "attributes": {
                "description": "The values of the block's attributes, from tag to value, as an \
                                insert's are written. A tag the block defines more than once \
                                takes a list of values; a tag it defines once takes one value. \
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
            "connection": {
                "description": "How the block meets the landing, as AutoCAD's block connection \
                                type says: extents, by the centre of the block's extents, \
                                AutoCAD's default; or insertion, by its base point. Either way \
                                the block sits level with the landing, slid along it until the \
                                first place its outline crosses the landing's line is at the \
                                landing's end. Without it, extents.",
                "enum": BlockConnection::ALL.map(BlockConnection::name),
            },
        },
    })
}

/// Adds the definitions of a multileader style, a multileader and an older
/// leader to `defs`.
pub(super) fn leader_defs(defs: &mut Map<String, Value>) {
    defs.insert(
        "mleader_style".into(),
        json!({
            "description": "A multileader style, AutoCAD's MLEADERSTYLE: what a multileader \
                            draws with unless it gives its own. Each key left out is as the \
                            sheet's unit's Standard style has it.",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "text_style": text_style_name(&format!("`{STANDARD}`")),
                "text_height": {
                    "$ref": "#/$defs/length_more_than_0",
                    "description": "The height of the text's capitals: a length more than 0.",
                },
                "arrowhead": arrowhead("closed_filled"),
                "arrow_size": {
                    "$ref": "#/$defs/length_more_than_0",
                    "description": "How long the arrowhead is: a length more than 0.",
                },
                "landing": {
                    "$ref": "#/$defs/length_not_negative",
                    "description": "How long the landing is, the short horizontal line the \
                                    leader lines meet: a length of 0 or more, 0 for none.",
                },
                "landing_gap": {
                    "$ref": "#/$defs/length_not_negative",
                    "description": "The space between the landing's end and the text: a length \
                                    of 0 or more.",
                },
                "attachment": attachment("middle_of_top_line"),
                "leader_color": {
                    "$ref": "#/$defs/object_color",
                    "description": "The colour the leader lines, landing and arrowheads draw \
                                    in, as an object's own colour is written. Without it, \
                                    `ByBlock`: the multileader's own colour.",
                },
                "leader_lineweight": {
                    "$ref": "#/$defs/object_lineweight",
                    "description": "The lineweight they draw with, as an object's own \
                                    lineweight is written. Without it, `ByBlock`: the \
                                    multileader's own lineweight.",
                },
            },
        }),
    );
    defs.insert(
        "mleader".into(),
        object_def(
            "A multileader, AutoCAD's MULTILEADER: one or more leader lines from their arrows to \
             a landing, and multiline text or a block beyond it. The landing runs right when \
             the first leader line's last segment runs right or straight up or down, and left \
             when it runs left; text sits a gap beyond it, attached at its top left or top \
             right corner, where the attachment says, and a block at its end, where AutoCAD \
             places block content. An arrow is placed at a point: pointing it at \
             an object by its ID or name waits for object snaps. A multileader belongs on a \
             sheet, not in a block.",
            &["leaders", "content"],
            json!({
                "leaders": {
                    "description": "The leader lines: at least one, each a list of at least two \
                                    points from its arrow's point to where it meets the landing, \
                                    each point apart from the one before. Every leader line ends \
                                    at the same point, where the landing starts, and its first \
                                    segment is at least as long as its arrow, unless the \
                                    arrowhead is none.",
                    "type": "array",
                    "items": leader_points("A leader line's points, its arrow's point first."),
                    "minItems": 1,
                },
                "content": {
                    "description": "The multileader's content: multiline text, under `text`, or \
                                    a block of the sheet's block library, under `block`, not \
                                    both.",
                    "anyOf": [
                        {
                            "description": "Text content: multiline text.",
                            "type": "object",
                            "required": ["text"],
                            "additionalProperties": false,
                            "properties": {
                                "text": leader_text(),
                                "style": text_style_name("the multileader style's `text_style`"),
                                "height": {
                                    "$ref": "#/$defs/length_more_than_0",
                                    "description": "The height of the text's capitals: a length \
                                                    more than 0. Without it, the multileader \
                                                    style's `text_height`.",
                                },
                                "attachment": attachment("the multileader style's"),
                            },
                        },
                        block_content(),
                    ],
                },
                "landing": {
                    "$ref": "#/$defs/length_not_negative",
                    "description": "How long the landing is: a length of 0 or more, 0 for none. \
                                    Without it, the multileader style's.",
                },
                "arrowhead": arrowhead("the multileader style's"),
                "arrow_size": {
                    "$ref": "#/$defs/length_more_than_0",
                    "description": "How long the arrowhead is: a length more than 0. Without \
                                    it, the multileader style's.",
                },
                "gap": {
                    "$ref": "#/$defs/length_not_negative",
                    "description": "The space between the landing's end and the text: a length \
                                    of 0 or more. Without it, the multileader style's \
                                    `landing_gap`.",
                },
                "mleader_style": optional_text(json!({
                    "description": format!(
                        "The multileader style it draws with: `{STANDARD_MLEADER_STYLE}`, which \
                         every drawing has, or a name under `mleader_styles`, written exactly as \
                         it is there. Without it, `{STANDARD_MLEADER_STYLE}`."
                    ),
                    "type": TEXT_TYPES,
                })),
            }),
        ),
    );
    defs.insert(
        "leader".into(),
        object_def(
            "An older leader, AutoCAD's LEADER: one leader line, arrow first, with an optional \
             multiline text annotation a gap beyond its end, attached at the middle of its left \
             side, or of its right side when the last segment runs left. With an annotation and \
             a last segment more than 15° from the horizontal, AutoCAD hooks the end: a \
             horizontal segment as long as the arrow, towards the annotation. A leader belongs \
             on a sheet, not in a block.",
            &["points"],
            json!({
                "points": leader_points(
                    "The leader's points: at least two, its arrow's point first, each apart from \
                     the one before. Its first segment is at least as long as its arrow, unless \
                     the arrowhead is none."
                ),
                "arrowhead": arrowhead("closed_filled"),
                "arrow_size": {
                    "$ref": "#/$defs/length_more_than_0",
                    "description": "How long the arrowhead, and the hook, are: a length more \
                                    than 0. Without it, 2.5 mm on a metric sheet and 0.18 in on \
                                    an imperial one.",
                },
                "gap": {
                    "$ref": "#/$defs/length_not_negative",
                    "description": "The space between the leader's end and its annotation: a \
                                    length of 0 or more. Without it, 0.625 mm on a metric sheet \
                                    and 0.09 in on an imperial one.",
                },
                "annotation": optional_text(json!({
                    "description": "The leader's annotation: multiline text. Left empty or \
                                    `null`, the leader has none.",
                    "type": "object",
                    "required": ["text"],
                    "additionalProperties": false,
                    "properties": {
                        "text": leader_text(),
                        "style": text_style_name(&format!("`{STANDARD}`")),
                        "height": {
                            "$ref": "#/$defs/length_more_than_0",
                            "description": "The height of the text's capitals: a length more \
                                            than 0. Without it, 2.5 mm on a metric sheet and \
                                            0.18 in on an imperial one.",
                        },
                    },
                })),
            }),
        ),
    );
}
