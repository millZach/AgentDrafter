//! The schema of table styles and tables, part of the sheet description's
//! schema ([`super::schema`]).

use serde_json::{json, Map, Value};

use super::schema::{object_def, optional_text, TEXT_TYPES};
use crate::model::{RowKind, STANDARD_TABLE_STYLE};
use crate::text::{Attachment, STANDARD};

/// The `table_styles` key of a sheet.
pub(super) fn table_styles_property() -> Value {
    json!({
        "description": format!(
            "The sheet's table styles, from name to table style: what a table's cells draw \
             with unless they give their own. Style `{STANDARD_TABLE_STYLE}` is in every \
             drawing: on a metric sheet its title rows are 6 mm high, its header and data rows \
             4.5 mm and its margins 1.5 mm, and on an imperial sheet 0.25, 0.18 and 0.06 in, \
             all in the text style `{STANDARD}`; listing it sets what it draws with. It is \
             written `{STANDARD_TABLE_STYLE}` exactly: in another case it is an error."
        ),
        "type": "object",
        "propertyNames": { "$ref": "#/$defs/name" },
        "additionalProperties": { "$ref": "#/$defs/table_style" },
    })
}

fn justify() -> Value {
    json!({
        "description": "Where the text sits in its cell, inside its margins: one of the nine \
                        attachments of multiline text.",
        "enum": Attachment::ALL.map(Attachment::name),
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

/// Adds the definitions of a table style, a table and a cell to `defs`.
pub(super) fn table_defs(defs: &mut Map<String, Value>) {
    let row = |kind: RowKind, justified: &str| {
        json!({
            "description": format!(
                "What the cells of the {} rows draw their text with, where it is not the \
                 style's own.",
                kind.name()
            ),
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "text_style": text_style_name("the style's `text_style`"),
                "text_height": {
                    "$ref": "#/$defs/length_more_than_0",
                    "description": "The height of the capitals: a length more than 0. Without \
                                    it, the style's `text_height`.",
                },
                "justify": {
                    "description": format!(
                        "Where the text sits in its cell, inside its margins. Without it, \
                         {justified}."
                    ),
                    "enum": Attachment::ALL.map(Attachment::name),
                },
            },
        })
    };
    defs.insert(
        "table_style".into(),
        json!({
            "description": "A table style, AutoCAD's TABLESTYLE: what a table's cells draw \
                            with unless they give their own.",
            "type": "object",
            "required": ["text_height"],
            "additionalProperties": false,
            "properties": {
                "text_style": text_style_name(&format!("`{STANDARD}`")),
                "text_height": {
                    "$ref": "#/$defs/length_more_than_0",
                    "description": "The height of the capitals of every kind of row's text, \
                                    unless the row gives its own: a length more than 0.",
                },
                "horizontal_margin": {
                    "$ref": "#/$defs/length_not_negative",
                    "description": "The space between a cell's sides and its text: a length of \
                                    0 or more. Without it, 1.5 mm on a metric sheet and 0.06 in \
                                    on an imperial one.",
                },
                "vertical_margin": {
                    "$ref": "#/$defs/length_not_negative",
                    "description": "The space between a cell's top and bottom and its text: a \
                                    length of 0 or more. Without it, as the horizontal margin's \
                                    default.",
                },
                "border_color": {
                    "$ref": "#/$defs/object_color",
                    "description": "The colour the borders draw in, as an object's own colour \
                                    is written. Without it, `ByBlock`: the table's own colour.",
                },
                "border_lineweight": {
                    "$ref": "#/$defs/object_lineweight",
                    "description": "The lineweight the borders draw with, as an object's own \
                                    lineweight is written. Without it, `ByBlock`: the table's \
                                    own lineweight.",
                },
                "title": row(RowKind::Title, "middle_center"),
                "header": row(RowKind::Header, "middle_center"),
                "data": row(RowKind::Data, "top_center"),
            },
        }),
    );
    defs.insert(
        "table_cell".into(),
        json!({
            "description": "A cell: its text alone, `''` or left empty for an empty cell, or a \
                            mapping that gives its text and what it draws with.",
            "anyOf": [
                { "$ref": "#/$defs/cell_text" },
                { "type": "null" },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "text": optional_text(json!({ "$ref": "#/$defs/cell_text" })),
                        "style": text_style_name("the text style of its row"),
                        "height": {
                            "$ref": "#/$defs/length_more_than_0",
                            "description": "The height of its text's capitals: a length more \
                                            than 0. Without it, its row's.",
                        },
                        "justify": justify(),
                        "merge": {
                            "description": "How many rows and columns the cell spans, `[rows, \
                                            columns]`, itself among them: two whole numbers from \
                                            1, not both 1. The cells it spans are empty, and it \
                                            keeps within the table and off other merges.",
                            "type": "array",
                            "items": {
                                "anyOf": [
                                    { "type": "integer", "minimum": 1 },
                                    { "type": "string", "pattern": "^0*[1-9][0-9]*$" },
                                ],
                            },
                            "minItems": 2,
                            "maxItems": 2,
                        },
                    },
                },
            ],
        }),
    );
    defs.insert(
        "cell_text".into(),
        json!({
            "description": "What a cell says, exactly as it shows: a line break starts a new \
                            line, and line breaks at the end are dropped. Empty for an empty \
                            cell; otherwise something that shows. No control characters but \
                            line breaks. A \\, { or } shows as itself.",
            "type": TEXT_TYPES,
        }),
    );
    defs.insert(
        "table".into(),
        object_def(
            "A table, AutoCAD's TABLE: a grid of cells with text, hung from its top left corner, \
             its rows running down and its columns across. A table belongs on a sheet, not in a \
             block.",
            &["position", "columns", "rows", "cells"],
            json!({
                "position": {
                    "$ref": "#/$defs/point",
                    "description": "The table's top left corner.",
                },
                "columns": {
                    "description": "The width of each column, left to right: at least one \
                                    length more than 0.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/length_more_than_0" },
                    "minItems": 1,
                },
                "rows": {
                    "description": "The height of each row, top to bottom: at least one length \
                                    more than 0. A row is as high as given; the checker reports \
                                    text that needs more.",
                    "type": "array",
                    "items": { "$ref": "#/$defs/length_more_than_0" },
                    "minItems": 1,
                },
                "cells": {
                    "description": "The cells, row by row from the top: one row of cells for \
                                    each of `rows`, each one cell for each of `columns`.",
                    "type": "array",
                    "items": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/table_cell" },
                    },
                    "minItems": 1,
                },
                "title": {
                    "description": "Whether the first row is a title row, drawn as the table \
                                    style's title rows are. Without it, false.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "header": {
                    "description": "Whether the row after the title, or the first row when \
                                    there is no title, is a header row. Without it, false.",
                    "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
                },
                "table_style": optional_text(json!({
                    "description": format!(
                        "The table style the table draws with: `{STANDARD_TABLE_STYLE}`, which \
                         every drawing has, or a name under `table_styles`, written exactly as \
                         it is there. Without it, `{STANDARD_TABLE_STYLE}`."
                    ),
                    "type": TEXT_TYPES,
                })),
            }),
        ),
    );
}
