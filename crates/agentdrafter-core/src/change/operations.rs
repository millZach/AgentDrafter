//! The operations a change set holds: their names, their AutoCAD aliases,
//! their schemas, and how one sent as plain data is read.
//!
//! Every object a sheet description holds has an operation that adds it,
//! named after the object and aliased to the AutoCAD command that draws it.
//! `change`, `move`, `delete` and `set_attribute` edit an object an agent
//! names by its ID or by its name.

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::description::schema::{block_library_schema, sheet_description_schema};
use crate::description::{
    article, objects_table, rows_schema, LAYER_KEYS, LINETYPE_KEYS, TEXT_STYLE_KEYS,
};
use crate::error::{Error, ErrorCode};
use crate::units::{parse_length, LengthError, Unit};

/// What an operation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Adds an object of the type it names.
    Add(&'static str),
    Change,
    Move,
    Delete,
    SetAttribute,
    /// Adds, changes, renames or deletes a layer, a linetype or a text
    /// style of a template.
    Table(TableOp, Table),
    /// Adds or redefines a block of a block library, or brings the inserts
    /// of its blocks into line with their definitions.
    Block(BlockOp),
}

/// What a block operation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockOp {
    /// Adds a block: AutoCAD's BLOCK.
    Add,
    /// Replaces a block's whole definition, then brings its inserts into
    /// line: AutoCAD's BEDIT.
    Redefine,
    /// Brings inserts into line with their block's definition alone:
    /// AutoCAD's ATTSYNC.
    Sync,
    /// Imports a drawing's blocks, each with every block it nests: AutoCAD's
    /// DesignCenter, ADCENTER.
    Import,
}

/// A table of a template's definitions that an operation edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Table {
    Layer,
    Linetype,
    TextStyle,
}

impl Table {
    /// The key the table sits under in a template, such as `layers`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Table::Layer => "layers",
            Table::Linetype => "linetypes",
            Table::TextStyle => "text_styles",
        }
    }

    /// One entry, in words, such as `text style`.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Table::Layer => "layer",
            Table::Linetype => "linetype",
            Table::TextStyle => "text style",
        }
    }

    /// The definition of one entry in the sheet description's schema.
    fn def(self) -> &'static str {
        match self {
            Table::Layer => "layer",
            Table::Linetype => "linetype",
            Table::TextStyle => "text_style",
        }
    }

    /// The keys one entry takes, in the order a sheet description writes
    /// them.
    pub(crate) fn entry_keys(self) -> &'static [&'static str] {
        match self {
            Table::Layer => &LAYER_KEYS,
            Table::Linetype => &LINETYPE_KEYS,
            Table::TextStyle => &TEXT_STYLE_KEYS,
        }
    }

    /// The names every drawing has in the table, which are never renamed
    /// or deleted: layer `0`, `Continuous` (with AutoCAD's `ByLayer` and
    /// `ByBlock`) and `Standard`.
    pub(crate) fn built_in(self) -> &'static [&'static str] {
        match self {
            Table::Layer => &["0"],
            Table::Linetype => &["Continuous", "ByLayer", "ByBlock"],
            Table::TextStyle => &["Standard"],
        }
    }

    /// The key an object, or a layer, names an entry by: `layer`,
    /// `linetype` or `style`.
    pub(crate) fn named_by(self) -> &'static str {
        match self {
            Table::Layer => "layer",
            Table::Linetype => "linetype",
            Table::TextStyle => "style",
        }
    }
}

/// What a table operation does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TableOp {
    Add,
    Change,
    Rename,
    Delete,
}

/// One operation: its name, the AutoCAD command it is aliased to, and what
/// it does.
#[derive(Debug)]
pub(crate) struct Spec {
    pub name: &'static str,
    pub alias: &'static str,
    pub kind: Kind,
}

/// Every operation, in the order they are listed.
pub(crate) const SPECS: [Spec; 32] = [
    add("add_line", "LINE", "line"),
    add("add_polyline", "PLINE", "polyline"),
    add("add_arc", "ARC", "arc"),
    add("add_circle", "CIRCLE", "circle"),
    add("add_ellipse", "ELLIPSE", "ellipse"),
    add("add_spline", "SPLINE", "spline"),
    add("add_hatch", "HATCH", "hatch"),
    add("add_text", "TEXT", "text"),
    add("add_mtext", "MTEXT", "mtext"),
    add("add_donut", "DONUT", "donut"),
    add("add_solid", "SOLID", "solid"),
    add("insert_block", "INSERT", "insert"),
    Spec {
        name: "change",
        alias: "CHANGE",
        kind: Kind::Change,
    },
    Spec {
        name: "move",
        alias: "MOVE",
        kind: Kind::Move,
    },
    Spec {
        name: "delete",
        alias: "ERASE",
        kind: Kind::Delete,
    },
    Spec {
        name: "set_attribute",
        alias: "ATTEDIT",
        kind: Kind::SetAttribute,
    },
    table("add_layer", "LAYER", TableOp::Add, Table::Layer),
    table("change_layer", "", TableOp::Change, Table::Layer),
    table("rename_layer", "RENAME", TableOp::Rename, Table::Layer),
    table("delete_layer", "LAYDEL", TableOp::Delete, Table::Layer),
    table("add_linetype", "LINETYPE", TableOp::Add, Table::Linetype),
    table("change_linetype", "", TableOp::Change, Table::Linetype),
    table("rename_linetype", "", TableOp::Rename, Table::Linetype),
    table("delete_linetype", "", TableOp::Delete, Table::Linetype),
    table("add_text_style", "STYLE", TableOp::Add, Table::TextStyle),
    table("change_text_style", "", TableOp::Change, Table::TextStyle),
    table("rename_text_style", "", TableOp::Rename, Table::TextStyle),
    table("delete_text_style", "", TableOp::Delete, Table::TextStyle),
    block("add_block", "BLOCK", BlockOp::Add),
    block("redefine_block", "BEDIT", BlockOp::Redefine),
    block("attsync", "ATTSYNC", BlockOp::Sync),
    block("import_blocks", "ADCENTER", BlockOp::Import),
];

const fn block(name: &'static str, alias: &'static str, op: BlockOp) -> Spec {
    Spec {
        name,
        alias,
        kind: Kind::Block(op),
    }
}

const fn table(name: &'static str, alias: &'static str, op: TableOp, table: Table) -> Spec {
    Spec {
        name,
        alias,
        kind: Kind::Table(op, table),
    }
}

const fn add(name: &'static str, alias: &'static str, object: &'static str) -> Spec {
    Spec {
        name,
        alias,
        kind: Kind::Add(object),
    }
}

/// The keys of a change set.
const CHANGE_SET_KEYS: [&str; 2] = ["operations", "fixes"];

/// The key every operation names itself by.
const OP_KEY: &str = "op";

/// One operation as an agent lists them: its name, its AutoCAD alias, what
/// it does and its schema.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OperationInfo {
    pub name: String,
    /// The AutoCAD command it does the work of, which names it too.
    pub alias: String,
    pub does: String,
    /// A JSON Schema (draft 2020-12) of the operation as a change set holds
    /// it, with the definitions it refers to.
    pub schema: Value,
}

/// Every operation, with how a change set holds them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OperationList {
    /// How a change set is sent, and how an operation names the object it
    /// edits.
    pub how: String,
    pub operations: Vec<OperationInfo>,
}

/// Every operation a change set can hold, with its alias, what it does and
/// its schema.
pub fn operations() -> OperationList {
    let defs = definitions();
    OperationList {
        how: HOW.to_owned(),
        operations: SPECS
            .iter()
            .map(|spec| OperationInfo {
                name: spec.name.to_owned(),
                alias: spec.alias.to_owned(),
                does: does(spec),
                schema: with_definitions(operation_schema(spec, &defs), &defs),
            })
            .collect(),
    }
}

const HOW: &str = "A change set is {\"operations\": [...]}: a list of operations, each a mapping \
                   that names its operation under op, by its name or its AutoCAD alias in any \
                   case, applied in order to the drawing set's working revision, its own files. \
                   An operation that edits an object names it under target, by its ID, \
                   {\"id\": \"DEMO-001/border\"}, or by its name, {\"name\": \"title-block\", \
                   \"sheet\": \"DEMO-001\"}, where sheet may be left out when the name is on one \
                   sheet only; never by coordinates. An object of a template is on every sheet \
                   built from it, and editing it edits the template, so every one of those \
                   sheets follows. An object added with sheet goes in that sheet's row, and \
                   with template in the template. Each sheet takes its part fully or not at \
                   all, and a template edit lands only if every sheet built from the template \
                   takes its whole part. A change set that lands on some sheets and fails on \
                   others marks the set inconsistent until a later change set names the failed \
                   sheets under fixes and lands on them. Lengths are in the set's declared unit \
                   and may carry their own. Send it with `agentdrafter change <set> \
                   <change-set.json>`, and add --dry-run to see what it would do without \
                   writing.";

/// What `spec` does, in words.
fn does(spec: &Spec) -> String {
    match spec.kind {
        Kind::Add(object) => format!(
            "Adds {} {object}, written with the keys {} {object} takes in a sheet description, \
             to one sheet's row with sheet, or to a template with template, so that it is on \
             every sheet built from it. It is drawn after the objects already there. Without \
             an id it is given one, {object}-1 or the next number free, so that its ID is \
             fixed for its life. A key sent as null is read as a sheet description reads a \
             bare null: it leaves an optional text key, such as layer, out. AutoCAD's {}.",
            article(object),
            article(object),
            spec.alias
        ),
        Kind::Change => "Sets keys of an object: each key under set takes its new value, as a \
                         sheet description writes it, and null takes a key away. It changes \
                         neither the object's type nor its id, which is fixed for its life; an \
                         insert's attributes are set with set_attribute. A text's contents its \
                         template takes from the row are set in that sheet's row. AutoCAD's \
                         CHANGE."
            .to_owned(),
        Kind::Move => "Moves an object by by, [dx, dy, dz]: every point it is drawn through \
                       moves, and an ellipse's major axis, an arc's angles and the like stay \
                       as they are. AutoCAD's MOVE."
            .to_owned(),
        Kind::Delete => "Deletes an object. Each later object of its file that has no id is \
                         given its place as its id, such as objects[3], so that its ID does not \
                         move. AutoCAD's ERASE."
            .to_owned(),
        Kind::Table(op, table) => {
            let word = table.word();
            let what = match op {
                TableOp::Add => format!(
                    "Adds a {word} named name to a template, written with the keys a {word} takes \
                     in a sheet description, so that every sheet built from the template has it"
                ),
                TableOp::Change => format!(
                    "Sets keys of the {word} named name in a template: each key under set takes \
                     its new value, and null takes a key away"
                ),
                TableOp::Rename => format!(
                    "Renames the {word} named name in a template to to, and everything in the \
                     template, and in the rows of the sheets built from it, that names it"
                ),
                TableOp::Delete => format!(
                    "Deletes the {word} named name from a template; refused while anything in the \
                     template, or in the rows of the sheets built from it, uses it, and the error \
                     lists what does"
                ),
            };
            let builtin = match (op, table) {
                (TableOp::Rename | TableOp::Delete, _) => format!(
                    " {} is every drawing's own and is never renamed or deleted.",
                    table.built_in()[0]
                ),
                _ => String::new(),
            };
            let alias = if spec.alias.is_empty() {
                String::new()
            } else {
                format!(" AutoCAD's {}.", spec.alias)
            };
            format!(
                "{what}. The template is named by template, as the rows write its path, or by \
                 sheet, a sheet built from it; every sheet built from it follows, all or \
                 nothing.{builtin}{alias}"
            )
        }
        Kind::Block(op) => {
            let what = match op {
                BlockOp::Add => {
                    "Adds a block named name to a block library: its base_point, which may be \
                     left out for [0, 0, 0], and its objects, each written as a block library \
                     writes a block's objects, attribute definitions (attdef) and inserts of the \
                     library's other blocks among them. A name the library has already, in any \
                     case, is refused, since AutoCAD ignores case in names. AutoCAD's BLOCK."
                }
                BlockOp::Redefine => {
                    "Replaces the whole definition of the block named name in a block library: \
                     its base_point, [0, 0, 0] when left out, and its full list of objects, \
                     written as for add_block. As it lands it brings every insert of the block \
                     into line, as attsync does: a value an insert gives for a tag the block no \
                     longer carries is removed from its record, and the reply lists each under \
                     attributes_removed. AutoCAD's BEDIT."
                }
                BlockOp::Sync => {
                    "Brings every insert of the block named name, or of every block of the \
                     library when name is left out, into line with the block's definition as \
                     the library writes it now, such as after the library was edited by hand: a \
                     value an insert gives for a tag the block no longer carries, because it no \
                     longer defines it or defines it constant, is removed from its record, in a \
                     template, a row or another block of the library; a new tag takes its \
                     default. The reply lists the inserts it reached under inserts and each \
                     value removed under attributes_removed. AutoCAD's ATTSYNC."
                }
                BlockOp::Import => {
                    "Imports the blocks of a DWG or a DXF, from, its path relative to the set's \
                     folder or absolute, into a block library, as AutoCAD's DesignCenter does: \
                     every named block, or those blocks names, each with every block it nests, \
                     with their objects, attribute definitions, and the layers, linetypes, hatch \
                     patterns and text styles they use that the library lacks; one it defines \
                     already it keeps as it defines it. The drawing is only read, and is in the \
                     set's unit, or states none and units names it. A block the library holds \
                     already is refused, unless replace, which replaces its whole definition and \
                     brings its inserts into line as redefine_block does. What the core cannot \
                     carry, such as a dimension, is left out and counted block by block under \
                     imported.not_carried. AutoCAD's ADCENTER."
                }
            };
            format!(
                "{what} The library is named by library, its path in the set as the rows write \
                 a template's path, such as blocks.yaml, which may be left out when every \
                 template of the set names the same library. It reaches every sheet whose \
                 template names the library, all or nothing."
            )
        }
        Kind::SetAttribute => "Sets the value of the attribute tag of an insert. When the \
                               insert's block defines the tag more than once, occurrence names \
                               which, counted from 1 in the order the block defines them, and \
                               is needed: without it the operation fails, listing each \
                               occurrence with its value. A tag written in lowercase is taken \
                               in capitals, as AutoCAD keeps it. When the insert's template \
                               takes that attribute from the row, the sheet's row value is set \
                               instead, on that sheet alone, and the reply names every place on \
                               the sheet the value fills. AutoCAD's ATTEDIT."
            .to_owned(),
    }
}

/// The change set's schema: a JSON Schema (draft 2020-12) of every key a
/// change set may hold, with what each means.
pub fn change_set_schema() -> Value {
    let defs = definitions();
    let choices: Vec<Value> = SPECS
        .iter()
        .map(|spec| operation_schema(spec, &defs))
        .collect();
    let [operations_key, fixes_key] = CHANGE_SET_KEYS;
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AgentDrafter change set",
        "description": HOW,
        "type": "object",
        "required": [operations_key],
        "additionalProperties": false,
        "properties": {
            operations_key: {
                "description": "The operations, applied in order.",
                "type": "array",
                "items": { "oneOf": choices },
            },
            fixes_key: {
                "description": "The sheets of the set's inconsistent mark this change set fixes. \
                                Each is cleared from the mark when the change set lands on it, \
                                whether or not an operation edits it; the sheet is built again.",
                "type": "array",
                "items": { "$ref": "#/$defs/sheet_name" },
            },
        },
    });
    with_definitions(schema, &defs)
}

/// Every definition an operation's schema may refer to: the sheet
/// description's, a sheet's name and a target.
fn definitions() -> Map<String, Value> {
    let mut defs = match sheet_description_schema()["$defs"].take() {
        Value::Object(defs) => defs,
        _ => Map::new(),
    };
    defs.insert(
        "sheet_name".into(),
        rows_schema()["$defs"]["sheet_name"].take(),
    );
    defs.insert(
        "template_path".into(),
        json!({
            "description": "A template of the set, as its rows write its path, such as \
                            templates/ansi-b.yaml.",
            "type": "string",
            "minLength": 1,
        }),
    );
    add_block_definitions(&mut defs);
    defs.insert(
        "target".into(),
        json!({
            "description": "The object an operation edits: by its ID, or by its name and, when \
                            the name is on more than one sheet, its sheet. Never by coordinates.",
            "oneOf": [
                {
                    "type": "object",
                    "required": ["id"],
                    "additionalProperties": false,
                    "properties": {
                        "id": {
                            "description": "The object's ID: its sheet's name, /, and the id \
                                            its record gives it or its place, such as \
                                            DEMO-001/border or DEMO-001/objects[2]. Matched in \
                                            any case.",
                            "type": "string",
                            "pattern": "^[^/]+/.+$",
                        },
                    },
                },
                {
                    "type": "object",
                    "required": ["name"],
                    "additionalProperties": false,
                    "properties": {
                        "name": {
                            "description": "The object's name, matched in any case.",
                            "type": "string",
                            "minLength": 1,
                        },
                        "sheet": { "$ref": "#/$defs/sheet_name" },
                    },
                },
            ],
        }),
    );
    defs
}

/// Adds the block library's definition of a block to `defs`, as
/// `block_definition`, with each object a block holds as the library's
/// schema defines it, under `block_` and its type, such as `block_line`,
/// and the cell. A block's objects differ from a sheet's: they carry no
/// `id` or `name`, and a text or an attribute definition takes a cell.
fn add_block_definitions(defs: &mut Map<String, Value>) {
    let mut library = match block_library_schema()["$defs"].take() {
        Value::Object(defs) => defs,
        _ => Map::new(),
    };
    let mut block = library.remove("block").unwrap_or(Value::Null);
    let mut types = Vec::new();
    if let Some(choices) = block["properties"]["objects"]["items"]["oneOf"].as_array_mut() {
        for choice in choices {
            let Some(properties) = choice["properties"].as_object_mut() else {
                continue;
            };
            for (kind, object) in properties.iter_mut() {
                object["$ref"] = Value::String(format!("#/$defs/block_{kind}"));
                types.push(kind.clone());
            }
        }
    }
    defs.insert("block_definition".into(), block);
    for kind in types {
        if let Some(def) = library.remove(&kind) {
            defs.insert(format!("block_{kind}"), def);
        }
    }
    if let Some(cell) = library.remove("cell") {
        defs.insert("cell".into(), cell);
    }
}

/// The schema of one operation, referring to `defs`.
fn operation_schema(spec: &Spec, defs: &Map<String, Value>) -> Value {
    let op = if spec.alias.is_empty() {
        json!({
            "description": format!("The operation: {}.", spec.name),
            "const": spec.name,
        })
    } else {
        json!({
            "description": format!(
                "The operation: {}, or its AutoCAD alias {} in any case.",
                spec.name, spec.alias
            ),
            "anyOf": [
                { "const": spec.name },
                { "type": "string", "pattern": format!("^{}$", any_case(spec.alias)) },
            ],
        })
    };
    let target = json!({ "$ref": "#/$defs/target" });
    let mut schema = match spec.kind {
        Kind::Add(object) => {
            let mut schema = defs[object].clone();
            let properties = schema["properties"]
                .as_object_mut()
                .expect("an object's definition has properties");
            properties.insert(
                "sheet".into(),
                json!({
                    "description": "The sheet to add it to, in its row: it is on this sheet \
                                    alone. Give sheet or template.",
                    "$ref": "#/$defs/sheet_name",
                }),
            );
            properties.insert(
                "template".into(),
                json!({
                    "description": "The template to add it to: it is on every sheet built \
                                    from it. Give sheet or template.",
                    "$ref": "#/$defs/template_path",
                }),
            );
            let place =
                json!({ "oneOf": [{ "required": ["sheet"] }, { "required": ["template"] }] });
            let own = schema.as_object_mut().expect("an object's definition");
            let mut all_of = vec![place];
            if let Some(one_of) = own.remove("oneOf") {
                all_of.push(json!({ "oneOf": one_of }));
            }
            own.insert("allOf".into(), Value::Array(all_of));
            schema
        }
        Kind::Change => json!({
            "type": "object",
            "required": ["target", "set"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "set": {
                    "description": "The keys to set, each to its new value as a sheet \
                                    description writes it, such as {\"height\": 0.3} or \
                                    {\"layer\": \"notes\"}; null takes an optional key away. \
                                    Any key the object's type takes, and name; not id, nor an \
                                    insert's attributes.",
                    "type": "object",
                    "minProperties": 1,
                    "propertyNames": { "not": { "enum": ["id"] } },
                },
            },
        }),
        Kind::Move => json!({
            "type": "object",
            "required": ["target", "by"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "by": {
                    "description": "How far to move it, [dx, dy, dz], as lengths in the set's \
                                    declared unit or carrying their own.",
                    "$ref": "#/$defs/point",
                },
            },
        }),
        Kind::Delete => json!({
            "type": "object",
            "required": ["target"],
            "additionalProperties": false,
            "properties": { "target": target },
        }),
        Kind::Table(op, table) => table_schema(op, table, defs),
        Kind::Block(op) => block_schema(op, defs),
        Kind::SetAttribute => json!({
            "type": "object",
            "required": ["target", "tag", "value"],
            "additionalProperties": false,
            "properties": {
                "target": target,
                "tag": { "$ref": "#/$defs/attribute_tag" },
                "occurrence": {
                    "description": "Which of the attributes with this tag to set, counted from 1 \
                                    in the order the insert's block defines them: needed when \
                                    the block defines the tag more than once, and 1 or left out \
                                    when it defines it once.",
                    "type": "integer",
                    "minimum": 1,
                },
                "value": { "$ref": "#/$defs/attribute_value" },
            },
        }),
    };
    schema["description"] = json!(does(spec));
    schema["properties"][OP_KEY] = op;
    let required = schema["required"].as_array_mut().expect("a list");
    required.insert(0, json!(OP_KEY));
    schema
}

/// The schema of a table operation, before its `op`.
fn table_schema(op: TableOp, table: Table, defs: &Map<String, Value>) -> Value {
    let word = table.word();
    let mut properties = Map::new();
    properties.insert(
        "name".into(),
        json!({
            "description": format!("The {word}'s name, written exactly as the template writes it."),
            "type": "string",
            "minLength": 1,
        }),
    );
    properties.insert(
        "sheet".into(),
        json!({
            "description": "A sheet built from the template to edit: the template is that \
                            sheet's. Give sheet or template.",
            "$ref": "#/$defs/sheet_name",
        }),
    );
    properties.insert(
        "template".into(),
        json!({
            "description": "The template to edit. Give sheet or template.",
            "$ref": "#/$defs/template_path",
        }),
    );
    let mut required = vec![json!("name")];
    match op {
        TableOp::Add => {
            let def = &defs[table.def()];
            if let Some(own) = def["properties"].as_object() {
                for (key, value) in own {
                    properties.insert(key.clone(), value.clone());
                }
            }
            if let Some(own) = def["required"].as_array() {
                required.extend(own.iter().cloned());
            }
        }
        TableOp::Change => {
            properties.insert(
                "set".into(),
                json!({
                    "description": format!(
                        "The keys to set, each to its new value as a sheet description writes it; \
                         null takes a key away. Any key a {word} takes: {}.",
                        table.entry_keys().join(", ")
                    ),
                    "type": "object",
                    "minProperties": 1,
                    "propertyNames": { "enum": table.entry_keys() },
                }),
            );
            required.push(json!("set"));
        }
        TableOp::Rename => {
            properties.insert(
                "to".into(),
                json!({
                    "description": format!("The {word}'s new name."),
                    "type": "string",
                    "minLength": 1,
                }),
            );
            required.push(json!("to"));
        }
        TableOp::Delete => {}
    }
    json!({
        "type": "object",
        "required": required,
        "additionalProperties": false,
        "properties": properties,
        "oneOf": [{ "required": ["sheet"] }, { "required": ["template"] }],
    })
}

/// The schema of a block operation, before its `op`. A block's objects are
/// those the block library's schema gives a block, whose text and
/// attribute definitions take a cell, as `block_text` and `block_attdef`.
fn block_schema(op: BlockOp, defs: &Map<String, Value>) -> Value {
    let block = &defs["block_definition"];
    let mut properties = Map::new();
    properties.insert(
        "library".into(),
        json!({
            "description": "The block library, by its path in the set, relative to the set's \
                            folder as the rows write a template's path, such as blocks.yaml. It \
                            may be left out when every template of the set names the same \
                            library.",
            "type": "string",
            "minLength": 1,
        }),
    );
    if op == BlockOp::Import {
        properties.insert(
            "from".into(),
            json!({
                "description": "The DWG or DXF to import the blocks of, told apart by its \
                                extension: its path relative to the set's folder, or absolute.",
                "type": "string",
                "minLength": 1,
            }),
        );
        properties.insert(
            "blocks".into(),
            json!({
                "description": "Only the blocks of these names, in any case, each with every \
                                block it nests. Without them, every named block of the drawing.",
                "type": "array",
                "items": { "type": "string", "minLength": 1 },
            }),
        );
        properties.insert(
            "replace".into(),
            json!({
                "description": "Replace the whole definition of a block the library holds \
                                already, rather than refuse.",
                "type": "boolean",
            }),
        );
        properties.insert(
            "units".into(),
            json!({
                "description": "The unit the drawing's blocks are drawn in, for a drawing that \
                                states none a block library takes: the set's.",
                "enum": Unit::ALL.map(Unit::symbol),
            }),
        );
        return json!({
            "type": "object",
            "required": ["from"],
            "additionalProperties": false,
            "properties": properties,
        });
    }
    let name_description = match op {
        BlockOp::Add => "The new block's name, which the library does not have in any case.",
        BlockOp::Redefine => "The block to redefine, written exactly as the library writes it.",
        BlockOp::Sync => {
            "The block whose inserts to bring into line, written exactly as the library writes \
             it. Without it, every block of the library."
        }
        BlockOp::Import => unreachable!("handled above"),
    };
    properties.insert(
        "name".into(),
        json!({ "description": name_description, "$ref": "#/$defs/name" }),
    );
    let mut required = Vec::new();
    if op != BlockOp::Sync {
        for key in ["base_point", "objects"] {
            properties.insert(key.into(), block["properties"][key].clone());
        }
        required = vec![json!("name"), json!("objects")];
    }
    json!({
        "type": "object",
        "required": required,
        "additionalProperties": false,
        "properties": properties,
    })
}

/// `schema` with the definitions it refers to, from `defs`, under `$defs`.
fn with_definitions(mut schema: Value, defs: &Map<String, Value>) -> Value {
    let mut wanted: Vec<String> = Vec::new();
    let mut queue = vec![schema.clone()];
    while let Some(value) = queue.pop() {
        refs(&value, &mut |name| {
            if !wanted.iter().any(|w| w == name) {
                wanted.push(name.to_owned());
                if let Some(def) = defs.get(name) {
                    queue.push(def.clone());
                }
            }
        });
    }
    wanted.sort();
    let mut used = Map::new();
    for name in wanted {
        if let Some(def) = defs.get(&name) {
            used.insert(name, def.clone());
        }
    }
    schema["$defs"] = Value::Object(used);
    schema
}

/// Calls `found` with the name of every definition `value` refers to.
fn refs(value: &Value, found: &mut dyn FnMut(&str)) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(target)) = map.get("$ref") {
                if let Some(name) = target.strip_prefix("#/$defs/") {
                    found(name);
                }
            }
            map.values().for_each(|v| refs(v, found));
        }
        Value::Array(items) => items.iter().for_each(|v| refs(v, found)),
        _ => {}
    }
}

/// A regular expression matching `word` in any case.
fn any_case(word: &str) -> String {
    word.chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_uppercase(), c.to_ascii_lowercase())
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// One operation of a change set, read.
#[derive(Clone, Debug)]
pub(crate) struct Operation {
    /// Its place in the change set.
    pub index: usize,
    pub spec: &'static Spec,
    pub body: Body,
    /// The operation as it was sent.
    pub sent: Value,
}

#[derive(Clone, Debug)]
pub(crate) enum Body {
    Add {
        to: AddTo,
        keys: Map<String, Value>,
    },
    Change {
        target: Target,
        set: Map<String, Value>,
    },
    Move {
        target: Target,
        by: [f64; 3],
    },
    Delete {
        target: Target,
    },
    SetAttribute {
        target: Target,
        tag: String,
        /// Which of the attributes with the tag, counted from 1, when the
        /// block defines it more than once.
        occurrence: Option<usize>,
        value: String,
    },
    /// A table operation: on the entry `name` of the template `to` names,
    /// with the keys it adds or sets, and the new name a rename gives.
    Table {
        op: TableOp,
        table: Table,
        to: AddTo,
        name: String,
        keys: Map<String, Value>,
        new_name: Option<String>,
    },
    /// A block operation: on the block `name`, or every block for an
    /// attsync without one, of the block library `library` names, with the
    /// definition an add or a redefinition writes, as sent.
    Block {
        op: BlockOp,
        library: Option<String>,
        name: Option<String>,
        base_point: Option<Value>,
        objects: Option<Vec<Value>>,
        /// What an import_blocks imports.
        import: Option<Import>,
    },
}

/// What an import_blocks operation imports: the blocks of the drawing at
/// `from`, as sent, those `blocks` names or every one, replacing those the
/// library holds when `replace`, in `units` when the drawing states none.
#[derive(Clone, Debug)]
pub(crate) struct Import {
    pub from: String,
    pub blocks: Vec<String>,
    pub replace: bool,
    pub units: Option<Unit>,
}

/// Where an object is added.
#[derive(Clone, Debug)]
pub(crate) enum AddTo {
    /// To the sheet's row, by the sheet's name.
    Sheet(String),
    /// To the template, as the rows write its path.
    Template(String),
}

/// How an operation names the object it edits.
#[derive(Clone, Debug)]
pub(crate) enum Target {
    /// By its ID: the sheet's name and the ID in its file.
    Id { sheet: String, id: String },
    /// By its name, on the sheet given or on the one sheet it is on.
    Name { name: String, sheet: Option<String> },
}

impl Target {
    pub(crate) fn words(&self) -> String {
        match self {
            Target::Id { sheet, id } => format!("`{sheet}/{id}`"),
            Target::Name {
                name,
                sheet: Some(sheet),
            } => format!("the object named `{name}` on sheet `{sheet}`"),
            Target::Name { name, sheet: None } => format!("the object named `{name}`"),
        }
    }
}

/// A change set as sent: its operations and the sheets it fixes.
pub(crate) struct ChangeSet {
    pub operations: Vec<Operation>,
    pub fixes: Vec<(String, usize)>,
}

/// Reads a change set sent as plain data, with lengths in `units`. A change
/// set that is wrong in itself, such as one naming an operation that does
/// not exist, is refused whole: the error names the operation and lists the
/// valid choices.
pub(crate) fn read_change_set(value: &Value, units: Unit) -> Result<ChangeSet, Error> {
    let Value::Object(map) = value else {
        return Err(Error::new(
            ErrorCode::WrongType,
            "a change set is a mapping, {\"operations\": [...]}",
        ));
    };
    if let Some(key) = map.keys().find(|k| !CHANGE_SET_KEYS.contains(&k.as_str())) {
        return Err(Error::new(
            ErrorCode::UnknownKey,
            format!(
                "unknown key `{key}`; a change set takes {}",
                CHANGE_SET_KEYS.join(", ")
            ),
        )
        .at_field(key.clone()));
    }
    let Some(list) = map.get("operations") else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            "missing key `operations`; a change set lists its operations under it",
        )
        .at_field("operations"));
    };
    let Value::Array(items) = list else {
        return Err(
            Error::new(ErrorCode::WrongType, "expected a list of operations")
                .at_field("operations"),
        );
    };
    // What each object an add writes requires, as the schema states it.
    let objects = sheet_description_schema()["$defs"].take();
    let operations = items
        .iter()
        .enumerate()
        .map(|(index, item)| operation(index, item, units, &objects))
        .collect::<Result<_, _>>()?;
    let fixes = match map.get("fixes") {
        None => Vec::new(),
        Some(Value::Array(sheets)) => sheets
            .iter()
            .enumerate()
            .map(|(i, sheet)| match sheet {
                Value::String(name) => Ok((name.clone(), i)),
                _ => Err(Error::new(ErrorCode::WrongType, "expected a sheet's name")
                    .at_field(format!("fixes[{i}]"))),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(
                Error::new(ErrorCode::WrongType, "expected a list of sheets' names")
                    .at_field("fixes"),
            )
        }
    };
    Ok(ChangeSet { operations, fixes })
}

/// What an import_blocks operation sent as `map` imports, or the code,
/// message and key of what is wrong with it.
fn read_import(map: &Map<String, Value>) -> Result<Import, (ErrorCode, String, &'static str)> {
    let from = match map.get("from") {
        Some(Value::String(from)) if !from.is_empty() => from.clone(),
        Some(_) => {
            return Err((
                ErrorCode::WrongType,
                "expected the drawing's path, a DWG or a DXF, relative to the set's folder or \
                 absolute"
                    .to_owned(),
                "from",
            ))
        }
        None => {
            return Err((
                ErrorCode::MissingKey,
                "missing key `from`, the DWG or DXF to import the blocks of".to_owned(),
                "from",
            ))
        }
    };
    let blocks = match map.get("blocks") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(name) if !name.is_empty() => Ok(name.clone()),
                _ => Err((
                    ErrorCode::WrongType,
                    "expected a list of the blocks' names".to_owned(),
                    "blocks",
                )),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err((
                ErrorCode::WrongType,
                "expected a list of the blocks' names".to_owned(),
                "blocks",
            ))
        }
    };
    let replace = match map.get("replace") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err((
                ErrorCode::WrongType,
                "expected true or false".to_owned(),
                "replace",
            ))
        }
    };
    let units = match map.get("units") {
        None => None,
        Some(Value::String(symbol)) if Unit::from_symbol(symbol).is_some() => {
            Unit::from_symbol(symbol)
        }
        Some(_) => {
            return Err((
                ErrorCode::UnknownUnit,
                format!("expected a unit, one of {}", Unit::symbols()),
                "units",
            ))
        }
    };
    Ok(Import {
        from,
        blocks,
        replace,
        units,
    })
}

/// The error for operation `index`, named `name`, at `field` within it.
pub(crate) fn op_error(
    index: usize,
    name: &str,
    code: ErrorCode,
    message: impl AsRef<str>,
    field: &str,
) -> Error {
    let path = if field.is_empty() {
        format!("operations[{index}]")
    } else {
        format!("operations[{index}].{field}")
    };
    Error::new(
        code,
        format!("operation {index} (`{name}`): {}", message.as_ref()),
    )
    .at_field(path)
}

/// Reads operation `index`, whose lengths are in `units`. `objects` holds
/// the schema of each object an add writes, whose required keys an add
/// must give: an operation that leaves one out is wrong in itself, so the
/// whole change set is refused before anything is applied.
fn operation(
    index: usize,
    value: &Value,
    units: Unit,
    objects: &Value,
) -> Result<Operation, Error> {
    let Value::Object(map) = value else {
        return Err(Error::new(
            ErrorCode::WrongType,
            format!("operation {index} is not a mapping; an operation names itself under op"),
        )
        .at_field(format!("operations[{index}]")));
    };
    let listed = || {
        SPECS
            .iter()
            .map(|s| {
                if s.alias.is_empty() {
                    s.name.to_owned()
                } else {
                    format!("{} ({})", s.name, s.alias)
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let name = match map.get(OP_KEY) {
        Some(Value::String(name)) => name,
        _ => {
            return Err(Error::new(
                ErrorCode::MissingKey,
                format!(
                    "operation {index} names no operation under op; the operations: {}",
                    listed()
                ),
            )
            .at_field(format!("operations[{index}].op")))
        }
    };
    let Some(spec) = SPECS
        .iter()
        .find(|s| s.name == name || (!s.alias.is_empty() && s.alias.eq_ignore_ascii_case(name)))
    else {
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!(
                "operation {index}: there is no operation `{name}`; the operations, with their \
                 AutoCAD aliases: {}",
                listed()
            ),
        )
        .at_field(format!("operations[{index}].op")));
    };
    let fail = |code: ErrorCode, message: String, field: &str| {
        op_error(index, spec.name, code, message, field)
    };
    let keys = |allowed: &[&str]| -> Result<(), Error> {
        match map
            .keys()
            .find(|k| *k != OP_KEY && !allowed.contains(&k.as_str()))
        {
            Some(key) => Err(fail(
                ErrorCode::UnknownKey,
                format!(
                    "unknown key `{key}`; {} takes op, {}",
                    spec.name,
                    allowed.join(", ")
                ),
                key,
            )),
            None => Ok(()),
        }
    };
    let target = || -> Result<Target, Error> {
        let value = map.get("target").ok_or_else(|| {
            fail(
                ErrorCode::MissingKey,
                "missing key `target`, the object to edit, by its ID, {\"id\": \
                 \"DEMO-001/border\"}, or by its name, {\"name\": \"title-block\", \"sheet\": \
                 \"DEMO-001\"}"
                    .to_owned(),
                "target",
            )
        })?;
        read_target(value).map_err(|(code, message, field)| fail(code, message, &field))
    };
    let body = match spec.kind {
        Kind::Add(object) => {
            let own: Vec<&str> = objects_table(object);
            let mut allowed = vec!["sheet", "template"];
            allowed.extend(&own);
            keys(&allowed)?;
            if let Some(missing) = missing_required(&objects[object], map) {
                return Err(fail(
                    ErrorCode::MissingKey,
                    format!(
                        "missing key `{}`; {} {object} needs {}",
                        missing.first,
                        article(object),
                        missing.needs
                    ),
                    &missing.first,
                ));
            }
            let to =
                match (map.get("sheet"), map.get("template")) {
                    (Some(Value::String(sheet)), None) => AddTo::Sheet(sheet.clone()),
                    (None, Some(Value::String(template))) => AddTo::Template(template.clone()),
                    (Some(_), Some(_)) => {
                        return Err(fail(
                            ErrorCode::Usage,
                            "give sheet or template, not both: an object added to a sheet goes in \
                         its row, and one added to a template is on every sheet built from it"
                                .to_owned(),
                            "",
                        ))
                    }
                    (None, None) => return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `sheet`: name the sheet to add it to, in its row, or give \
                         template to add it to a template"
                            .to_owned(),
                        "sheet",
                    )),
                    (Some(_), None) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected a sheet's name".to_owned(),
                            "sheet",
                        ))
                    }
                    (None, Some(_)) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected a template's path".to_owned(),
                            "template",
                        ))
                    }
                };
            // A key sent as `null` is kept: the record writes it as a bare
            // null, and the sheet's reader reads it as it reads one in any
            // file (rule 8 of the YAML subset), so it leaves an optional text
            // or path field out and is an error anywhere else.
            let keys: Map<String, Value> = map
                .iter()
                .filter(|(k, _)| !matches!(k.as_str(), OP_KEY | "sheet" | "template"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            Body::Add { to, keys }
        }
        Kind::Change => {
            keys(&["target", "set"])?;
            let target = target()?;
            let set = match map.get("set") {
                Some(Value::Object(set)) if !set.is_empty() => set.clone(),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a mapping of at least one key to its new value, such as \
                         {\"height\": 0.3}"
                            .to_owned(),
                        "set",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `set`, the keys to set and their new values".to_owned(),
                        "set",
                    ))
                }
            };
            Body::Change { target, set }
        }
        Kind::Move => {
            keys(&["target", "by"])?;
            let target = target()?;
            let by = match map.get("by") {
                Some(Value::Array(items)) if items.len() == 3 => {
                    let mut by = [0.0; 3];
                    for (axis, item) in items.iter().enumerate() {
                        by[axis] = length(item, units).map_err(|message| {
                            fail(ErrorCode::BadNumber, message, &format!("by[{axis}]"))
                        })?;
                    }
                    by
                }
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected how far to move it, three lengths [dx, dy, dz]".to_owned(),
                        "by",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `by`, how far to move it, [dx, dy, dz]".to_owned(),
                        "by",
                    ))
                }
            };
            Body::Move { target, by }
        }
        Kind::Delete => {
            keys(&["target"])?;
            Body::Delete { target: target()? }
        }
        Kind::SetAttribute => {
            keys(&["target", "tag", "occurrence", "value"])?;
            let target = target()?;
            let occurrence =
                match map.get("occurrence") {
                    None => None,
                    // A whole number, written `1` or `1.0`, as JSON Schema's
                    // integer takes it.
                    Some(Value::Number(n)) if whole(n).is_some_and(|n| n >= 1) => {
                        whole(n).and_then(|n| usize::try_from(n).ok())
                    }
                    Some(_) => return Err(fail(
                        ErrorCode::WrongType,
                        "expected the occurrence, a whole number from 1: which of the attributes \
                         with this tag to set, in the order the block defines them"
                            .to_owned(),
                        "occurrence",
                    )),
                };
            let tag = match map.get("tag") {
                Some(Value::String(tag)) => tag.clone(),
                _ => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `tag`, the attribute tag to set, such as REV".to_owned(),
                        "tag",
                    ))
                }
            };
            let value = match map.get("value") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::Bool(b)) => b.to_string(),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected the attribute's value, one line of text".to_owned(),
                        "value",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `value`, the attribute's new value".to_owned(),
                        "value",
                    ))
                }
            };
            Body::SetAttribute {
                target,
                tag,
                occurrence,
                value,
            }
        }
        Kind::Block(op) => {
            let allowed: &[&str] = match op {
                BlockOp::Sync => &["library", "name"],
                BlockOp::Add | BlockOp::Redefine => &["library", "name", "base_point", "objects"],
                BlockOp::Import => &["library", "from", "blocks", "replace", "units"],
            };
            keys(allowed)?;
            let library = match map.get("library") {
                None => None,
                Some(Value::String(path)) if !path.is_empty() => Some(path.clone()),
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected the block library's path in the set, such as blocks.yaml"
                            .to_owned(),
                        "library",
                    ))
                }
            };
            let name = match map.get("name") {
                Some(Value::String(name)) if !name.is_empty() => Some(name.clone()),
                None if matches!(op, BlockOp::Sync | BlockOp::Import) => None,
                Some(_) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected the block's name".to_owned(),
                        "name",
                    ))
                }
                None => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `name`, the block's name".to_owned(),
                        "name",
                    ))
                }
            };
            let objects =
                match (op, map.get("objects")) {
                    (BlockOp::Sync | BlockOp::Import, _) => None,
                    (_, Some(Value::Array(items))) if !items.is_empty() => Some(items.clone()),
                    (_, Some(Value::Array(_))) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "a block holds at least one object".to_owned(),
                            "objects",
                        ))
                    }
                    (_, Some(_)) => return Err(fail(
                        ErrorCode::WrongType,
                        "expected the block's objects, a list of mappings each with one key, its \
                         type, such as {\"line\": {\"from\": [0, 0, 0], \"to\": [1, 0, 0]}}"
                            .to_owned(),
                        "objects",
                    )),
                    (_, None) => return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `objects`, the block's objects; a block holds at least one"
                            .to_owned(),
                        "objects",
                    )),
                };
            let import = if op == BlockOp::Import {
                Some(
                    read_import(map)
                        .map_err(|(code, message, field)| fail(code, message, field))?,
                )
            } else {
                None
            };
            Body::Block {
                op,
                library,
                name,
                base_point: map.get("base_point").cloned(),
                objects,
                import,
            }
        }
        Kind::Table(op, table) => {
            let mut allowed = vec!["sheet", "template", "name"];
            match op {
                TableOp::Add => allowed.extend(table.entry_keys()),
                TableOp::Change => allowed.push("set"),
                TableOp::Rename => allowed.push("to"),
                TableOp::Delete => {}
            }
            keys(&allowed)?;
            let to = match (map.get("sheet"), map.get("template")) {
                (Some(Value::String(sheet)), None) => AddTo::Sheet(sheet.clone()),
                (None, Some(Value::String(template))) => AddTo::Template(template.clone()),
                (Some(_), Some(_)) => {
                    return Err(fail(
                        ErrorCode::Usage,
                        "give sheet or template, not both: either names the template to edit"
                            .to_owned(),
                        "",
                    ))
                }
                (None, None) => {
                    return Err(fail(
                        ErrorCode::MissingKey,
                        "missing key `template`: name the template to edit, or a sheet built \
                         from it with sheet"
                            .to_owned(),
                        "template",
                    ))
                }
                (Some(_), None) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a sheet's name".to_owned(),
                        "sheet",
                    ))
                }
                (None, Some(_)) => {
                    return Err(fail(
                        ErrorCode::WrongType,
                        "expected a template's path".to_owned(),
                        "template",
                    ))
                }
            };
            let text = |key: &str, what: &str| -> Result<String, Error> {
                match map.get(key) {
                    Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
                    Some(_) => Err(fail(
                        ErrorCode::WrongType,
                        format!("expected {what}, a name"),
                        key,
                    )),
                    None => Err(fail(
                        ErrorCode::MissingKey,
                        format!("missing key `{key}`, {what}"),
                        key,
                    )),
                }
            };
            let name = text("name", &format!("the {}'s name", table.word()))?;
            let new_name = match op {
                TableOp::Rename => Some(text("to", &format!("the {}'s new name", table.word()))?),
                _ => None,
            };
            let keys: Map<String, Value> = match op {
                TableOp::Add => {
                    if let Some(missing) = missing_required(&objects[table.def()], map) {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            format!(
                                "missing key `{}`; a {} needs {}",
                                missing.first,
                                table.word(),
                                missing.needs
                            ),
                            &missing.first,
                        ));
                    }
                    map.iter()
                        .filter(|(k, _)| table.entry_keys().contains(&k.as_str()))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                }
                TableOp::Change => match map.get("set") {
                    Some(Value::Object(set)) if !set.is_empty() => {
                        if let Some(key) = set
                            .keys()
                            .find(|k| !table.entry_keys().contains(&k.as_str()))
                        {
                            return Err(fail(
                                ErrorCode::UnknownKey,
                                format!(
                                    "unknown key `{key}`; a {} takes {}",
                                    table.word(),
                                    table.entry_keys().join(", ")
                                ),
                                &format!("set.{key}"),
                            ));
                        }
                        set.clone()
                    }
                    Some(_) => {
                        return Err(fail(
                            ErrorCode::WrongType,
                            "expected a mapping of at least one key to its new value".to_owned(),
                            "set",
                        ))
                    }
                    None => {
                        return Err(fail(
                            ErrorCode::MissingKey,
                            "missing key `set`, the keys to set and their new values".to_owned(),
                            "set",
                        ))
                    }
                },
                TableOp::Rename | TableOp::Delete => Map::new(),
            };
            Body::Table {
                op,
                table,
                to,
                name,
                keys,
                new_name,
            }
        }
    };
    Ok(Operation {
        index,
        spec,
        body,
        sent: value.clone(),
    })
}

/// The required keys an object leaves out: the first one missing, and what
/// the object needs, in words.
struct Missing {
    first: String,
    needs: String,
}

/// What `keys` leaves out of what the object's schema `object` requires:
/// its required keys, and one of the sets of keys it offers a choice of,
/// such as a spline's `fit_points` or `control_points`.
fn missing_required(object: &Value, keys: &Map<String, Value>) -> Option<Missing> {
    let names = |list: Option<&Value>| -> Vec<String> {
        list.and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let required = names(object.get("required"));
    if let Some(first) = required.iter().find(|k| !keys.contains_key(*k)) {
        return Some(Missing {
            first: first.clone(),
            needs: required.join(", "),
        });
    }
    let choices: Vec<Vec<String>> = object
        .get("oneOf")
        .and_then(Value::as_array)
        .map(|options| options.iter().map(|o| names(o.get("required"))).collect())
        .unwrap_or_default();
    let chosen = choices
        .iter()
        .any(|choice| choice.iter().all(|k| keys.contains_key(k)));
    if choices.is_empty() || chosen {
        return None;
    }
    let first = choices
        .iter()
        .flat_map(|choice| choice.iter().find(|k| !keys.contains_key(*k)))
        .next()?
        .clone();
    let needs = choices
        .iter()
        .map(|choice| choice.join(" and "))
        .collect::<Vec<_>>()
        .join(" or ");
    Some(Missing { first, needs })
}

/// Reads a target: `{"id": "SHEET/ID"}` or `{"name": ..., "sheet": ...}`.
fn read_target(value: &Value) -> Result<Target, (ErrorCode, String, String)> {
    let Value::Object(map) = value else {
        return Err((
            ErrorCode::WrongType,
            "expected a target, {\"id\": \"DEMO-001/border\"} or {\"name\": \"title-block\", \
             \"sheet\": \"DEMO-001\"}"
                .to_owned(),
            "target".to_owned(),
        ));
    };
    if let Some(key) = map
        .keys()
        .find(|k| !matches!(k.as_str(), "id" | "name" | "sheet"))
    {
        return Err((
            ErrorCode::UnknownKey,
            format!("unknown key `{key}`; a target takes id, or name and sheet"),
            format!("target.{key}"),
        ));
    }
    let text = |key: &str| match map.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err((
            ErrorCode::WrongType,
            format!("expected text under {key}"),
            format!("target.{key}"),
        )),
    };
    match (text("id")?, text("name")?, text("sheet")?) {
        (Some(id), None, None) => match id.split_once('/') {
            Some((sheet, rest)) if !sheet.is_empty() && !rest.is_empty() => Ok(Target::Id {
                sheet: sheet.to_owned(),
                id: rest.to_owned(),
            }),
            _ => Err((
                ErrorCode::BadValue,
                format!(
                    "`{id}` is not an object's ID: an ID is its sheet's name, `/`, and the id \
                     its record gives it or its place, such as DEMO-001/border"
                ),
                "target.id".to_owned(),
            )),
        },
        (Some(_), None, Some(_)) => Err((
            ErrorCode::Usage,
            "an ID names its sheet already; give id alone, or name and sheet".to_owned(),
            "target".to_owned(),
        )),
        (None, Some(name), sheet) => Ok(Target::Name { name, sheet }),
        (Some(_), Some(_), _) => Err((
            ErrorCode::Usage,
            "a target names its object by its ID or by its name, not both".to_owned(),
            "target".to_owned(),
        )),
        (None, None, _) => Err((
            ErrorCode::MissingKey,
            "a target names its object by its ID, id, or by its name, name".to_owned(),
            "target".to_owned(),
        )),
    }
}

/// A length sent as a number or as text with its own unit, in `units`.
fn length(value: &Value, units: Unit) -> Result<f64, String> {
    let text = match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return Err("expected a length such as 1 or \"4 in\"".to_owned()),
    };
    match parse_length(&text) {
        Ok((value, None)) => Ok(value),
        Ok((value, Some(own))) => {
            let converted = own.convert(value, units);
            if converted.is_finite() {
                Ok(converted)
            } else {
                Err(format!("`{text}` is too large once converted to {units}"))
            }
        }
        Err(LengthError::UnknownUnit(unit)) => Err(format!(
            "unknown unit `{unit}`; known units: {}",
            Unit::symbols()
        )),
        Err(LengthError::OutOfRange) => Err(format!("`{text}` is too large")),
        Err(LengthError::NotANumber) => Err(format!(
            "expected a length such as 1 or \"4 in\", found `{text}`"
        )),
    }
}

/// `n` as a whole number from 0, whether JSON writes it `2` or `2.0`.
fn whole(n: &serde_json::Number) -> Option<u64> {
    n.as_u64().or_else(|| {
        n.as_f64()
            .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64)
            .map(|f| f as u64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::description::object_types;

    #[test]
    fn every_object_a_sheet_holds_has_an_operation_that_adds_it() {
        let added: Vec<&str> = SPECS
            .iter()
            .filter_map(|s| match s.kind {
                Kind::Add(object) => Some(object),
                _ => None,
            })
            .collect();
        assert_eq!(added, object_types());
    }

    #[test]
    fn every_operation_has_its_own_name_and_alias() {
        for (i, a) in SPECS.iter().enumerate() {
            for b in &SPECS[i + 1..] {
                assert_ne!(a.name, b.name);
                // An operation with no AutoCAD alias has none to share.
                assert!(a.alias.is_empty() || !a.alias.eq_ignore_ascii_case(b.alias));
            }
        }
    }

    #[test]
    fn every_reference_in_a_listed_schema_resolves_in_it() {
        fn walk(value: &Value, defs: &Value) {
            refs(value, &mut |name| {
                assert!(defs.get(name).is_some(), "{name} resolves")
            });
        }
        for info in operations().operations {
            walk(&info.schema, &info.schema["$defs"]);
        }
        let schema = change_set_schema();
        walk(&schema, &schema["$defs"]);
    }
}
