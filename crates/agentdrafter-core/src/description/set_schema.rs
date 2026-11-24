//! The schemas of a drawing set's files: its settings, its rows and its
//! templates, as JSON Schemas (draft 2020-12), built from the same tables
//! `set.rs` and `template.rs` check keys against.
//!
//! As everywhere in the schemas, what JSON Schema cannot see is left to the
//! reader: whether two sheets share a name, whether a row gives every value
//! its template needs, and whether a value is allowed where
//! its template puts it.

use serde_json::{json, Value};

use super::block_schema::{relative_path_property, CONTROL};
use super::objects::OBJECT_TYPES;
use super::schema::{object_choices, optional_text, sheet_description_schema, SUBSET, TEXT_TYPES};
use super::set::{
    DEVICE_NAMES, FORBIDDEN_IN_SHEET_NAMES, LONGEST_SHEET_NAME, ROWS_FILE_KEYS, ROW_KEYS,
    SETTINGS_KEYS,
};
use super::template::ROW_KEY;
use crate::build::DrawingFormat;
use crate::units::Unit;

/// The name of a row's value, as a regular expression.
pub(crate) const VALUE_NAME: &str = "^[A-Za-z0-9_-]+$";

/// The schema of a drawing set's settings, `set.yaml`.
pub(crate) fn set_schema() -> Value {
    let [units_key, output_key, rows_key, clean_key, plot_style_key, format_key] = SETTINGS_KEYS;
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AgentDrafter drawing set settings",
        "description": format!(
            "The settings of a drawing set: the file set.yaml at the top of the set's folder, \
             beside its templates, its block library and its rows. They declare the set's unit, \
             the folder its DWGs go to and the files its rows are in. Building the folder builds \
             one DWG per row. {SUBSET}"
        ),
        "type": "object",
        "required": [units_key, output_key, rows_key],
        "additionalProperties": false,
        "properties": {
            units_key: {
                "description": "The set's declared unit: every template, sheet description and \
                                block library in the set is in it, and so is every DWG the set \
                                builds. There is no default.",
                "enum": Unit::ALL.map(Unit::symbol),
            },
            output_key: relative_path_property(
                "The folder the set's DWGs go to, relative to the set's folder, with / between \
                 folders, such as dwg or ../issued. A build makes it if it is not there. Each \
                 sheet's DWG is named after the sheet, with .dwg after it."
                    .to_owned(),
            ),
            rows_key: rows_property(),
            clean_key: {
                "description": "Whether every build and plot of the set moves its leftover \
                                outputs, the DWGs and PDFs AgentDrafter made for the set in its \
                                output folder that no row writes any more, into the folder \
                                superseded in the output folder, keeping their names; they are \
                                never deleted. Without it, or false, they are only listed, under \
                                leftovers in the reply. A run's own clean, true or false, \
                                overrides it for that run.",
                "type": "boolean",
            },
            plot_style_key: super::page_setup::plot_style_file_schema(
                "The CTB or STB plot style file name the drawing set plots with, such as \
                 `monochrome.ctb` or `plant.stb`, found in the plot style folders. A run's own \
                 `plot_style` overrides it for that run. Without it, each sheet's page setup or \
                 the built-in monochrome style.",
            ),
            format_key: {
                "description": "The drawing format the drawing set builds: `dwg` (2018 DWG, the \
                                default), `dxf` (ASCII DXF) or `dxf_binary` (binary DXF). A run's \
                                own `format` overrides it for that run.",
                "enum": DrawingFormat::ALL.map(DrawingFormat::name),
            },
        },
    })
}

/// The settings' `rows`: one path, of a rows file or a folder of them, or
/// a list of such paths.
fn rows_property() -> Value {
    let mut property = relative_path_property(
        "The files the set's rows are in, relative to the set's folder, with / between \
         folders: one rows file, such as rows.yaml; a folder of them, such as rows, which gives \
         the .yaml files in it, not those in its folders, in the byte order of their names; or \
         a list of such paths, such as [rows/loops.yaml, rows/panels.yaml]. The set builds the \
         rows of each file in turn, in the order given, and a sheet's name is given once across \
         them all. No path is listed twice and no file is reached twice, and a folder, or a \
         path in a list, is a plain folder or file in the set's folder, reached through no link. \
         Each file's schema is `agentdrafter schema rows`."
            .to_owned(),
    );
    let item = relative_path_property(
        "The path of a rows file, or of a folder of them, relative to the set's folder, such \
         as rows/loops.yaml or rows."
            .to_owned(),
    );
    property["anyOf"]
        .as_array_mut()
        .expect("a path's schema is a choice")
        .push(json!({ "type": "array", "minItems": 1, "uniqueItems": true, "items": item }));
    property
}

/// The schema of a drawing set's rows file.
pub(crate) fn rows_schema() -> Value {
    let [rows_key] = ROWS_FILE_KEYS;
    let [sheet_key, template_key, values_key, objects_key] = ROW_KEYS;
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AgentDrafter drawing set rows",
        "description": format!(
            "The rows of a drawing set, the data its sheets are drawn from: one row per sheet, \
             naming the sheet, the template it is drawn from and the values the template takes \
             from it. The set's settings name this file under rows. {SUBSET}"
        ),
        "type": "object",
        "required": [rows_key],
        "additionalProperties": false,
        "properties": {
            rows_key: {
                "description": "The rows, one per sheet, in the order the set builds them.",
                "type": "array",
                "items": { "$ref": "#/$defs/row" },
            },
        },
        "$defs": {
            "row": {
                "description": "One sheet: a template plus this row of data.",
                "type": "object",
                "required": [sheet_key, template_key],
                "additionalProperties": false,
                "properties": {
                    sheet_key: { "$ref": "#/$defs/sheet_name" },
                    template_key: relative_path_property(
                        "The sheet's template: the path of its file, relative to the set's \
                         folder, with / between folders, such as templates/border.yaml. Its \
                         schema is `agentdrafter schema template`."
                            .to_owned(),
                    ),
                    values_key: {
                        "description": "The values the template takes from this row, from name \
                                        to text: one for each {row: NAME} the template writes, \
                                        and a value it does not use is reported as a notice. A \
                                        name is ASCII letters, digits, _ and -. \
                                        Without it, the row gives no values.",
                        "type": "object",
                        "propertyNames": { "pattern": VALUE_NAME },
                        "additionalProperties": optional_text(json!({
                            "description": "The value's text, exactly as the sheet shows it \
                                            where the template puts it. Whether it is allowed \
                                            there, such as one line for an attribute, is the \
                                            sheet description's rule for that place. A bare \
                                            null leaves the value out, as if the row did not \
                                            give it.",
                            "type": TEXT_TYPES,
                        })),
                    },
                    objects_key: {
                        "description": "The sheet's own objects, on this sheet alone: drawn \
                                        after its template's, each written as a sheet \
                                        description writes an object. They name the layers, \
                                        text styles, hatch patterns and blocks the template \
                                        defines or names, and write their values themselves: \
                                        a value from the row, {row: NAME}, belongs in a \
                                        template. An object's id is given once on the sheet, \
                                        the template's included; one without an id takes its \
                                        place in this file, such as rows[1].objects[0]. An \
                                        object an operation adds to one sheet goes here. \
                                        Without it, the sheet has only its template's objects.",
                        "type": "array",
                        "items": { "oneOf": object_choices(&OBJECT_TYPES) },
                    },
                },
            },
            "sheet_name": sheet_name_def(),
        },
    });
    // A row's own objects are a sheet description's objects, with the
    // definitions they refer to.
    if let (Some(defs), Value::Object(sheet_defs)) = (
        schema["$defs"].as_object_mut(),
        sheet_description_schema()["$defs"].take(),
    ) {
        for (name, def) in sheet_defs {
            defs.entry(name).or_insert(def);
        }
    }
    schema
}

/// The schema of a sheet's name, which names its DWG.
fn sheet_name_def() -> Value {
    let forbidden = FORBIDDEN_IN_SHEET_NAMES.replace('\\', "\\\\");
    let end = format!("[^\\s{forbidden}{CONTROL}]");
    let inner = format!("[^{forbidden}{CONTROL}]");
    let devices = DEVICE_NAMES
        .map(|name| {
            name.chars()
                .map(|c| {
                    if c.is_ascii_alphabetic() {
                        format!("[{c}{}]", c.to_ascii_lowercase())
                    } else {
                        c.to_string()
                    }
                })
                .collect::<String>()
        })
        .join("|");
    json!({
        "description": format!(
            "The sheet's name, which names its DWG: the sheet is written to the set's output \
             folder as the name with .dwg after it. 1 to {LONGEST_SHEET_NAME} characters, not \
             starting or ending with a space, not ending with ., holding none of \
             {FORBIDDEN_IN_SHEET_NAMES} and no control characters, and not, up to its first ., \
             one of the names Windows keeps for devices: {}, in any case. Each name is given once \
             in a set; names that differ only in case are one name. A plain number, true or \
             false is read as its text.",
            DEVICE_NAMES.join(", ")
        ),
        "anyOf": [
            {
                "type": "string",
                "minLength": 1,
                "maxLength": LONGEST_SHEET_NAME,
                "pattern": format!("^{end}({inner}*{end})?$"),
                "not": {
                    "anyOf": [
                        { "pattern": "\\.$" },
                        { "pattern": format!("^({devices})(\\.|$)") },
                    ],
                },
            },
            { "type": ["number", "boolean"] },
        ],
    })
}

/// The schema of a template: a sheet description's, where an insert's
/// attribute value and a text's or an mtext's contents may be a value
/// from the row, `{row: NAME}`.
pub(crate) fn template_schema() -> Value {
    let mut schema = sheet_description_schema();
    schema["title"] = json!("AgentDrafter template");
    schema["description"] = json!(format!(
        "A sheet's border, title block and fixed content, waiting for data. A template holds \
         what a sheet description holds, and where a sheet's value comes from its row in the \
         drawing set it writes {{{ROW_KEY}: NAME}} in place of the value: as an insert's \
         attribute value, as a text's or an mtext's contents, as a table cell or its text, or \
         as a multileader's content's or a leader's annotation's text, or as an attribute value \
         of a multileader's block content. A text's or an mtext's contents, and the value of \
         an attribute the insert's block defines once, may also be a list of parts, plain text \
         and values from the row joined in order, such as ['PANEL ', {{{ROW_KEY}: PANEL}}]. \
         Each sheet is the template with every {{{ROW_KEY}: NAME}} replaced by its row's value \
         NAME, and is built as a sheet description is. Its declared unit is the set's. \
         {SUBSET}"
    ));
    let library = &mut schema["properties"]["block_library"]["description"];
    *library = json!(library
        .as_str()
        .unwrap_or_default()
        .replace("the sheet description's folder", "the template's folder"));
    let defs = &mut schema["$defs"];
    defs["row_value"] = json!({
        "description": format!(
            "A value from the row the sheet is built from: {{{ROW_KEY}: NAME}} stands for the \
             row's value NAME. The row must give it."
        ),
        "type": "object",
        "required": [ROW_KEY],
        "additionalProperties": false,
        "properties": { ROW_KEY: { "$ref": "#/$defs/value_name" } },
    });
    defs["value_name"] = json!({
        "description": "The name of one of the row's values: ASCII letters, digits, _ and -, \
                        such as TITLE or dwg-no. A plain whole number such as 100, true or false \
                        is read as its text, as every scalar is.",
        "anyOf": [
            { "type": "string", "pattern": VALUE_NAME },
            { "type": ["integer", "boolean"] },
        ],
    });
    defs["parts"] = json!({
        "description": format!(
            "A list of parts: plain text and values from the row, {{{ROW_KEY}: NAME}}, joined in \
             order into one text, such as ['PANEL ', {{{ROW_KEY}: PANEL}}, ' SLOT ', \
             {{{ROW_KEY}: SLOT}}]. Nothing inside a string is read for values: a brace or a \
             backslash in one is text, shown as written. It holds one value from the row at least; a \
             text with none is written as one string. The joined text is held to the rules of \
             the place it fills. An edit of the joined text is matched to the fixed parts, to \
             set each row value it holds on that sheet alone."
        ),
        "type": "array",
        "minItems": 1,
        "items": {
            "anyOf": [
                { "type": ["string", "number", "boolean"] },
                { "$ref": "#/$defs/row_value" },
            ],
        },
        "contains": { "$ref": "#/$defs/row_value" },
    });
    // A text is one line, so its fixed parts hold no line break.
    let mut one_line = defs["parts"].clone();
    one_line["description"] = json!(format!(
        "{} A text is one line and holds no control character, so no fixed part holds a \
         line break or another control character.",
        one_line["description"].as_str().unwrap_or_default()
    ));
    one_line["items"]["anyOf"][0] = json!({
        "anyOf": [
            { "type": "string", "pattern": format!("^[^{CONTROL}]*$") },
            { "type": ["number", "boolean"] },
        ],
    });
    defs["text_parts"] = one_line;
    let attributes = &mut defs["insert"]["properties"]["attributes"];
    attributes["description"] = json!(
        "The values of the block's attributes, from tag to value: text, or a value from the row, \
         {row: NAME}. A tag the block defines more than once takes a list of values, one for \
         each in the order the block defines them, which may be shorter than that, each text or \
         a value from the row. An attribute not given, or given as null, takes its \
         definition's default."
    );
    let one = json!({
        "anyOf": [
            { "$ref": "#/$defs/attribute_value" },
            { "$ref": "#/$defs/row_value" },
        ],
    });
    // An attribute's list of parts: each fixed part an attribute's value,
    // one line with no control character, and one value from the row at
    // least, as the reader takes it for a tag the block defines once. A
    // part is never null, which only a repeated tag's list of values takes.
    let mut part = one.clone();
    part["not"] = json!({ "type": "null" });
    defs["attribute_parts"] = json!({
        "description": "An attribute's value as a list of parts: plain text, each part one \
                        line with no control character as an attribute's value is, and values \
                        from the row, {row: NAME}, joined in order, one at least. No part is \
                        null.",
        "type": "array",
        "minItems": 1,
        "items": part,
        "contains": { "$ref": "#/$defs/row_value" },
    });
    let attributes = &mut defs["insert"]["properties"]["attributes"];
    attributes["additionalProperties"] = json!({
        "anyOf": [
            one.clone(),
            { "type": "array", "items": one },
        ],
    });
    // So may a multileader's block content's, each with no `%%`.
    let content = serde_json::to_string(&defs["insert"]["properties"]["attributes"])
        .expect("a schema is JSON")
        .replace("#/$defs/attribute_value", "#/$defs/content_attribute_value");
    defs["mleader"]["properties"]["content"]["anyOf"][1]["properties"]["attributes"] =
        serde_json::from_str(&content).expect("a schema is JSON");
    // An insert's, not a multileader's, may be a list of parts.
    defs["insert"]["properties"]["attributes"]["description"] = json!(
        "The values of the block's attributes, from tag to value: text, or a value from the row, \
         {row: NAME}. A tag the block defines more than once takes a list of values, one for \
         each in the order the block defines them, which may be shorter than that, each text or \
         a value from the row. A tag the block defines once may take a list of parts instead, \
         plain text and values from the row joined in order, such as ['REV ', {row: REV}], \
         holding one value from the row at least. An attribute not given, or given as null, \
         takes its definition's default."
    );
    // A table's cell, or its text, may be a value from the row.
    let row_value = json!({ "$ref": "#/$defs/row_value" });
    if let Some(choices) = defs["table_cell"]["anyOf"].as_array_mut() {
        choices.push(row_value.clone());
    }
    if let Some(choices) =
        defs["table_cell"]["anyOf"][2]["properties"]["text"]["anyOf"].as_array_mut()
    {
        choices.push(row_value);
    }
    for (kind, holder) in [("mleader", "content"), ("leader", "annotation")] {
        let held = &mut defs[kind]["properties"][holder];
        // An optional mapping is one choice of two, beside null.
        let held = if held["anyOf"].is_array() {
            &mut held["anyOf"][0]
        } else {
            held
        };
        let text = &mut held["properties"]["text"];
        let written = text.take();
        *text = json!({
            "description": format!(
                "In a template, the text, or a value from the row, {{{ROW_KEY}: NAME}}, which the \
                 text's rules apply to once it is filled in."
            ),
            "anyOf": [
                written,
                { "$ref": "#/$defs/row_value" },
            ],
        });
    }
    for kind in ["text", "mtext"] {
        let contents = &mut defs[kind]["properties"]["contents"];
        let mut written = contents.take();
        let description = written
            .as_object_mut()
            .and_then(|own| own.remove("description"))
            .and_then(|d| d.as_str().map(str::to_owned))
            .unwrap_or_default();
        *contents = json!({
            "description": format!(
                "{description} In a template, or a value from the row, {{{ROW_KEY}: NAME}}, or a \
                 list of parts, plain text and values from the row joined in order, which these \
                 rules apply to once it is filled in."
            ),
            "anyOf": [
                written,
                { "$ref": "#/$defs/row_value" },
                { "$ref": if kind == "text" { "#/$defs/text_parts" } else { "#/$defs/parts" } },
            ],
        });
    }
    schema
}
