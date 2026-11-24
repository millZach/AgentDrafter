//! Sheet-description reading, JSON validation, evaluation post-pass and JSON
//! Schema definitions for AutoCAD fields (`FIELD`) and `custom_properties`
//! (`DWGPROPS`) (#87).

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::blocks::tag_in_capitals;
use super::ids::ObjectRecord;
use super::strict_yaml::{index_path, key_path, Kind, Node};
use super::{at, bad_value, choice, scalar, sequence, wrong_type, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::field::{
    default_field_template, evaluate_text_field, is_default_field_template, EvalContext, FieldItem,
    FieldKind, TextField, UNEVALUATED_FIELD,
};
use crate::model::{Object, Sheet};

/// Top-level sheet key for custom drawing properties (`DWGPROPS`).
pub(crate) const CUSTOM_PROPERTIES_KEY: &str = "custom_properties";

/// Key on a `text` or `mtext` object attaching an AutoCAD field.
pub(crate) const FIELD_KEY: &str = "field";

/// Key on an `insert` object attaching AutoCAD fields to carried attributes.
pub(crate) const ATTRIBUTE_FIELDS_KEY: &str = "attribute_fields";

/// Keys allowed in a single-item field mapping.
const ITEM_KEYS: [&str; 6] = ["kind", "property", "object", "code", "objects", "format"];

/// Keys allowed in a shorthand single-item field (item keys plus optional
/// `template` and `value`).
const SHORTHAND_KEYS: [&str; 8] = [
    "kind", "property", "object", "code", "objects", "format", "value", "template",
];

/// Keys allowed in a single item inside `items: [...]`.
const LIST_ITEM_KEYS: [&str; 7] = [
    "kind", "property", "object", "code", "objects", "format", "value",
];

/// Keys allowed in a multi-item field mapping.
const MULTI_KEYS: [&str; 2] = ["template", "items"];

/// The build mark property name reserved by AgentDrafter itself.
const RESERVED_PROPERTY: &str = "AgentDrafter";

/// Parses top-level `custom_properties: { Name: Value, ... }` in a sheet
/// description.
pub(super) fn custom_properties(node: &Node) -> Result<BTreeMap<String, String>, Error> {
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(
            node,
            CUSTOM_PROPERTIES_KEY,
            "a mapping from custom drawing property names to values",
        ));
    };
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut seen: Vec<(String, String, usize)> = Vec::new();
    for entry in entries {
        let entry_path = key_path(CUSTOM_PROPERTIES_KEY, &entry.key);
        let key = entry.key.trim();
        if key.is_empty() {
            return Err(Error::new(
                ErrorCode::BadValue,
                "a custom drawing property name cannot be empty",
            )
            .at_field(entry_path)
            .at_position(entry.key_at.line, entry.key_at.column));
        }
        if key.eq_ignore_ascii_case(RESERVED_PROPERTY) {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`{RESERVED_PROPERTY}` is reserved for AgentDrafter's own build mark and \
                     cannot be set under `{CUSTOM_PROPERTIES_KEY}`"
                ),
            )
            .at_field(entry_path)
            .at_position(entry.key_at.line, entry.key_at.column));
        }
        if let Some(c) = entry.key.chars().find(|c| c.is_control()) {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "a custom drawing property name cannot hold the control character `{}`",
                    c.escape_default()
                ),
            )
            .at_field(entry_path)
            .at_position(entry.key_at.line, entry.key_at.column));
        }
        let lower = entry.key.to_lowercase();
        if let Some((_, first_key, first_line)) = seen.iter().find(|(l, _, _)| *l == lower) {
            return Err(Error::new(
                ErrorCode::DuplicateKey,
                format!(
                    "custom drawing property `{}` is given twice, as `{first_key}` at line \
                     {first_line} and as `{}` here; AutoCAD treats custom property names \
                     case-insensitively",
                    entry.key, entry.key
                ),
            )
            .at_field(entry_path)
            .at_position(entry.key_at.line, entry.key_at.column));
        }
        seen.push((lower, entry.key.clone(), entry.key_at.line));
        let val = if entry.value.is_bare_null() {
            String::new()
        } else {
            let text = scalar(
                &entry.value,
                &entry_path,
                "a custom drawing property value, one line of text",
            )?;
            if text.contains(['\n', '\r']) {
                return Err(bad_value(
                    &entry.value,
                    &entry_path,
                    "a custom drawing property value is one line",
                ));
            }
            if let Some(c) = text.chars().find(|c| c.is_control()) {
                return Err(bad_value(
                    &entry.value,
                    &entry_path,
                    format!(
                        "a custom drawing property value cannot hold the control character `{}`",
                        c.escape_default()
                    ),
                ));
            }
            text.to_owned()
        };
        map.insert(entry.key.clone(), val);
    }
    Ok(map)
}

/// Parses a `field` mapping from a strict YAML node (accepting both the
/// single-item shorthand `{ kind: ... }` and the multi-item form
/// `{ template: "...", items: [...] }`).
pub(super) fn parse_field(node: &Node, path: &str) -> Result<TextField, Error> {
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(node, path, "a field, a mapping"));
    };
    let has_items = entries.iter().any(|e| e.key == "items");
    let has_kind = entries.iter().any(|e| e.key == "kind");
    if has_items && has_kind {
        return Err(bad_value(
            node,
            path,
            "a field gives either `kind` (for one field item) or `items` (for a list of field \
             items), not both",
        ));
    }
    let field = if has_items {
        let f = Fields::of(node, path, &MULTI_KEYS, "a field")?;
        let template = match f.text("template") {
            Some(tn) => {
                let tpath = f.path_of("template");
                let text = scalar(tn, &tpath, "a field template string such as `{0}`")?;
                validate_template_text(text, tn, &tpath)?;
                text.to_owned()
            }
            None => default_field_template(),
        };
        let items_node = f.require("items")?;
        let items_path = f.path_of("items");
        let list = sequence(items_node, &items_path, "a non-empty list of field items")?;
        if list.is_empty() {
            return Err(bad_value(
                items_node,
                &items_path,
                "a field's `items` list needs at least one item",
            ));
        }
        let mut items = Vec::with_capacity(list.len());
        for (i, item_node) in list.iter().enumerate() {
            items.push(parse_item(item_node, &index_path(&items_path, i), false)?);
        }
        TextField { template, items }
    } else {
        let f = Fields::of(node, path, &SHORTHAND_KEYS, "a field")?;
        let template = match f.text("template") {
            Some(tn) => {
                let tpath = f.path_of("template");
                let text = scalar(tn, &tpath, "a field template string such as `{0}`")?;
                validate_template_text(text, tn, &tpath)?;
                text.to_owned()
            }
            None => default_field_template(),
        };
        let item = parse_item(node, path, true)?;
        TextField {
            template,
            items: vec![item],
        }
    };
    if let Err(msg) = field.validate_slots() {
        return Err(bad_value(node, path, msg));
    }
    Ok(field)
}

fn validate_template_text(text: &str, node: &Node, path: &str) -> Result<(), Error> {
    if let Some(c) = text.chars().find(|c| c.is_control() && *c != '\n') {
        return Err(bad_value(
            node,
            path,
            format!(
                "a field template cannot hold the control character `{}`",
                c.escape_default()
            ),
        ));
    }
    Ok(())
}

fn parse_item(node: &Node, path: &str, shorthand: bool) -> Result<FieldItem, Error> {
    let allowed: &[&str] = if shorthand {
        &SHORTHAND_KEYS
    } else {
        &LIST_ITEM_KEYS
    };
    let f = Fields::of(node, path, allowed, "a field item")?;
    let kind_node = f.require_text("kind")?;
    let kind_path = f.path_of("kind");
    let choices = format!("field kinds: {}", FieldKind::choices());
    let kind_str = choice(kind_node, &kind_path, "a field kind", &choices)?;
    let Some(kind) = FieldKind::from_name(kind_str) else {
        return Err(bad_value(
            kind_node,
            &kind_path,
            format!("unknown field kind `{kind_str}`; {choices}"),
        ));
    };
    let property = match f.text("property") {
        Some(n) => {
            let p = f.path_of("property");
            let s = scalar(n, &p, "a property name")?;
            if s.trim().is_empty() {
                return Err(bad_value(n, &p, "`property` cannot be empty"));
            }
            Some(s.to_owned())
        }
        None => None,
    };
    let object = match f.text("object") {
        Some(n) => {
            let p = f.path_of("object");
            let s = scalar(n, &p, "an object ID on this sheet")?;
            if s.trim().is_empty() {
                return Err(bad_value(n, &p, "`object` cannot be empty"));
            }
            Some(s.to_owned())
        }
        None => None,
    };
    let code = match f.text("code") {
        Some(n) => {
            let p = f.path_of("code");
            let s = scalar(n, &p, "an AutoCAD field code")?;
            if s.trim().is_empty() {
                return Err(bad_value(n, &p, "`code` cannot be empty"));
            }
            Some(s.to_owned())
        }
        None => None,
    };
    let objects = match f.get("objects") {
        Some(n) => {
            let p = f.path_of("objects");
            let list = sequence(n, &p, "a list of referenced object IDs")?;
            let mut out = Vec::with_capacity(list.len());
            for (i, item) in list.iter().enumerate() {
                let ip = index_path(&p, i);
                let s = scalar(item, &ip, "an object ID")?;
                if s.trim().is_empty() {
                    return Err(bad_value(item, &ip, "an object ID cannot be empty"));
                }
                out.push(s.to_owned());
            }
            out
        }
        None => Vec::new(),
    };
    let format = match f.text("format") {
        Some(n) => {
            let p = f.path_of("format");
            let s = scalar(n, &p, "an AutoCAD field format string")?;
            if s.contains(['\n', '\r', '"']) {
                return Err(bad_value(
                    n,
                    &p,
                    "a field `format` string cannot contain line breaks or `\"`",
                ));
            }
            Some(s.to_owned())
        }
        None => None,
    };
    let value = match f.text("value") {
        Some(n) => {
            let p = f.path_of("value");
            let s = scalar(n, &p, "the field's display value")?;
            if let Some(c) = s.chars().find(|c| c.is_control() && *c != '\n') {
                return Err(bad_value(
                    n,
                    &p,
                    format!(
                        "a field value cannot hold the control character `{}`",
                        c.escape_default()
                    ),
                ));
            }
            Some(s.to_owned())
        }
        None => None,
    };
    let item = FieldItem {
        kind,
        property,
        object,
        code,
        objects,
        format,
        value,
    };
    validate_item_requirements(&item, |key, code, msg| {
        let key_path = f.path_of(key);
        let pos = f.get(key).map_or(node.at, |n| n.at);
        at(Error::new(code, msg), &key_path, pos)
    })?;
    Ok(item)
}

fn validate_item_requirements(
    item: &FieldItem,
    make_err: impl Fn(&str, ErrorCode, String) -> Error,
) -> Result<(), Error> {
    match item.kind {
        FieldKind::CustomProperty => {
            if item.property.is_none() {
                return Err(make_err(
                    "property",
                    ErrorCode::MissingKey,
                    "a `custom_property` field requires `property`, the custom drawing property \
                     name"
                        .to_owned(),
                ));
            }
            if item.object.is_some() {
                return Err(make_err(
                    "object",
                    ErrorCode::BadValue,
                    "`object` goes with `object_property`, not `custom_property`".to_owned(),
                ));
            }
            if item.code.is_some() {
                return Err(make_err(
                    "code",
                    ErrorCode::BadValue,
                    "`code` goes with `code`, not `custom_property`".to_owned(),
                ));
            }
            if !item.objects.is_empty() {
                return Err(make_err(
                    "objects",
                    ErrorCode::BadValue,
                    "`objects` goes with `code`, not `custom_property`".to_owned(),
                ));
            }
        }
        FieldKind::ObjectProperty => {
            if item.object.is_none() {
                return Err(make_err(
                    "object",
                    ErrorCode::MissingKey,
                    "an `object_property` field requires `object`, the ID of the object on this \
                     sheet"
                        .to_owned(),
                ));
            }
            if item.property.is_none() {
                return Err(make_err(
                    "property",
                    ErrorCode::MissingKey,
                    "an `object_property` field requires `property` (such as `Length`, `Area`, \
                     `Radius`, `Diameter`, `Circumference`, `Layer`, `Color` or `Linetype`)"
                        .to_owned(),
                ));
            }
            if item.code.is_some() {
                return Err(make_err(
                    "code",
                    ErrorCode::BadValue,
                    "`code` goes with `code`, not `object_property`".to_owned(),
                ));
            }
            if !item.objects.is_empty() {
                return Err(make_err(
                    "objects",
                    ErrorCode::BadValue,
                    "`objects` goes with `code`; use `object` for `object_property`".to_owned(),
                ));
            }
        }
        FieldKind::Code => {
            if item.code.is_none() {
                return Err(make_err(
                    "code",
                    ErrorCode::MissingKey,
                    "a `code` field requires `code`, the AutoCAD field expression (such as \
                     `\\AcVar Author`)"
                        .to_owned(),
                ));
            }
            if item.property.is_some() {
                return Err(make_err(
                    "property",
                    ErrorCode::BadValue,
                    "`property` goes with `custom_property` or `object_property`, not `code`"
                        .to_owned(),
                ));
            }
            if item.object.is_some() {
                return Err(make_err(
                    "object",
                    ErrorCode::BadValue,
                    "use `objects` (a list of IDs) with `code`, not `object`".to_owned(),
                ));
            }
        }
        FieldKind::Filename
        | FieldKind::SaveDate
        | FieldKind::PlotDate
        | FieldKind::CreateDate
        | FieldKind::Date
        | FieldKind::SheetNumber => {
            let kind_name = item.kind.name();
            if item.property.is_some() {
                return Err(make_err(
                    "property",
                    ErrorCode::BadValue,
                    format!("`{kind_name}` takes no `property`"),
                ));
            }
            if item.object.is_some() {
                return Err(make_err(
                    "object",
                    ErrorCode::BadValue,
                    format!("`{kind_name}` takes no `object`"),
                ));
            }
            if item.code.is_some() {
                return Err(make_err(
                    "code",
                    ErrorCode::BadValue,
                    format!("`{kind_name}` takes no `code`"),
                ));
            }
            if !item.objects.is_empty() {
                return Err(make_err(
                    "objects",
                    ErrorCode::BadValue,
                    format!("`{kind_name}` takes no `objects`"),
                ));
            }
        }
    }
    Ok(())
}

/// Parses a [`TextField`] from a JSON [`Value`] (for `set_field` change-set
/// operations and original-DWG revisions).
pub(crate) fn parse_field_json(value: &Value, path: &str) -> Result<TextField, Error> {
    let Some(obj) = value.as_object() else {
        return Err(
            Error::new(ErrorCode::WrongType, "expected a field object").at_field(path.to_owned())
        );
    };
    let has_items = obj.contains_key("items");
    let has_kind = obj.contains_key("kind");
    if has_items && has_kind {
        return Err(Error::new(
            ErrorCode::BadValue,
            "a field gives either `kind` (for one item) or `items` (for a list), not both",
        )
        .at_field(path.to_owned()));
    }
    let field = if has_items {
        for key in obj.keys() {
            if !MULTI_KEYS.contains(&key.as_str()) {
                return Err(Error::new(
                    ErrorCode::UnknownKey,
                    format!(
                        "unknown key `{key}` in field; a multi-item field takes {}",
                        MULTI_KEYS.join(", ")
                    ),
                )
                .at_field(key_path(path, key)));
            }
        }
        let template = match obj.get("template") {
            Some(Value::String(s)) => {
                if let Some(c) = s.chars().find(|c| c.is_control() && *c != '\n') {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!(
                            "a field template cannot hold the control character `{}`",
                            c.escape_default()
                        ),
                    )
                    .at_field(key_path(path, "template")));
                }
                s.clone()
            }
            Some(Value::Null) | None => default_field_template(),
            Some(_) => {
                return Err(
                    Error::new(ErrorCode::WrongType, "field `template` must be a string")
                        .at_field(key_path(path, "template")),
                );
            }
        };
        let Some(arr) = obj.get("items").and_then(Value::as_array) else {
            return Err(Error::new(
                ErrorCode::WrongType,
                "field `items` must be a non-empty list of field items",
            )
            .at_field(key_path(path, "items")));
        };
        if arr.is_empty() {
            return Err(
                Error::new(ErrorCode::BadValue, "field `items` needs at least one item")
                    .at_field(key_path(path, "items")),
            );
        }
        let mut items = Vec::with_capacity(arr.len());
        for (i, item_val) in arr.iter().enumerate() {
            items.push(parse_item_json(
                item_val,
                &index_path(&key_path(path, "items"), i),
                false,
            )?);
        }
        TextField { template, items }
    } else {
        for key in obj.keys() {
            if !SHORTHAND_KEYS.contains(&key.as_str()) {
                return Err(Error::new(
                    ErrorCode::UnknownKey,
                    format!(
                        "unknown key `{key}` in field; a field takes {}",
                        SHORTHAND_KEYS.join(", ")
                    ),
                )
                .at_field(key_path(path, key)));
            }
        }
        let template = match obj.get("template") {
            Some(Value::String(s)) => {
                if let Some(c) = s.chars().find(|c| c.is_control() && *c != '\n') {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!(
                            "a field template cannot hold the control character `{}`",
                            c.escape_default()
                        ),
                    )
                    .at_field(key_path(path, "template")));
                }
                s.clone()
            }
            Some(Value::Null) | None => default_field_template(),
            Some(_) => {
                return Err(
                    Error::new(ErrorCode::WrongType, "field `template` must be a string")
                        .at_field(key_path(path, "template")),
                );
            }
        };
        let item = parse_item_json(value, path, true)?;
        TextField {
            template,
            items: vec![item],
        }
    };
    if let Err(msg) = field.validate_slots() {
        return Err(Error::new(ErrorCode::BadValue, msg).at_field(path.to_owned()));
    }
    Ok(field)
}

fn parse_item_json(value: &Value, path: &str, shorthand: bool) -> Result<FieldItem, Error> {
    let Some(obj) = value.as_object() else {
        return Err(
            Error::new(ErrorCode::WrongType, "expected a field item object")
                .at_field(path.to_owned()),
        );
    };
    let allowed: &[&str] = if shorthand {
        &SHORTHAND_KEYS
    } else {
        &LIST_ITEM_KEYS
    };
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(Error::new(
                ErrorCode::UnknownKey,
                format!(
                    "unknown key `{key}` in field item; allowed keys: {}",
                    allowed.join(", ")
                ),
            )
            .at_field(key_path(path, key)));
        }
    }
    let Some(kind_str) = obj.get("kind").and_then(Value::as_str) else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            format!("missing key `kind`; field kinds: {}", FieldKind::choices()),
        )
        .at_field(key_path(path, "kind")));
    };
    let Some(kind) = FieldKind::from_name(kind_str) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "unknown field kind `{kind_str}`; field kinds: {}",
                FieldKind::choices()
            ),
        )
        .at_field(key_path(path, "kind")));
    };
    let opt_str = |key: &str| -> Result<Option<String>, Error> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => {
                if s.trim().is_empty() && key != "value" {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        format!("`{key}` cannot be empty"),
                    )
                    .at_field(key_path(path, key)));
                }
                Ok(Some(s.clone()))
            }
            Some(_) => Err(
                Error::new(ErrorCode::WrongType, format!("`{key}` must be a string"))
                    .at_field(key_path(path, key)),
            ),
        }
    };
    let property = opt_str("property")?;
    let object = opt_str("object")?;
    let code = opt_str("code")?;
    let format = opt_str("format")?;
    if let Some(fmt) = &format {
        if fmt.contains(['\n', '\r', '"']) {
            return Err(Error::new(
                ErrorCode::BadValue,
                "a field `format` string cannot contain line breaks or `\"`",
            )
            .at_field(key_path(path, "format")));
        }
    }
    let val = opt_str("value")?;
    if let Some(v) = &val {
        if let Some(c) = v.chars().find(|c| c.is_control() && *c != '\n') {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "a field value cannot hold the control character `{}`",
                    c.escape_default()
                ),
            )
            .at_field(key_path(path, "value")));
        }
    }
    let objects = match obj.get("objects") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                let ip = index_path(&key_path(path, "objects"), i);
                let Some(s) = v.as_str().filter(|s| !s.trim().is_empty()) else {
                    return Err(Error::new(
                        ErrorCode::WrongType,
                        "each entry in `objects` must be a non-empty object ID string",
                    )
                    .at_field(ip));
                };
                out.push(s.to_owned());
            }
            out
        }
        Some(_) => {
            return Err(Error::new(
                ErrorCode::WrongType,
                "`objects` must be a list of object ID strings",
            )
            .at_field(key_path(path, "objects")));
        }
    };
    let item = FieldItem {
        kind,
        property,
        object,
        code,
        objects,
        format,
        value: val,
    };
    validate_item_requirements(&item, |key, code, msg| {
        Error::new(code, msg).at_field(key_path(path, key))
    })?;
    Ok(item)
}

/// Converts a [`TextField`] back into compact JSON suitable for writing into a
/// sheet description or template record.
pub(crate) fn text_field_to_json(tf: &TextField) -> Value {
    if tf.items.len() == 1 {
        let mut map = item_to_map(&tf.items[0]);
        if !is_default_field_template(&tf.template) {
            map.insert("template".to_owned(), Value::String(tf.template.clone()));
        }
        Value::Object(map)
    } else {
        let mut map = Map::new();
        map.insert("template".to_owned(), Value::String(tf.template.clone()));
        map.insert(
            "items".to_owned(),
            Value::Array(
                tf.items
                    .iter()
                    .map(|it| Value::Object(item_to_map(it)))
                    .collect(),
            ),
        );
        Value::Object(map)
    }
}

fn item_to_map(item: &FieldItem) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(
        "kind".to_owned(),
        Value::String(item.kind.name().to_owned()),
    );
    if let Some(prop) = &item.property {
        map.insert("property".to_owned(), Value::String(prop.clone()));
    }
    if let Some(obj) = &item.object {
        map.insert("object".to_owned(), Value::String(obj.clone()));
    }
    if let Some(code) = &item.code {
        map.insert("code".to_owned(), Value::String(code.clone()));
    }
    if !item.objects.is_empty() {
        map.insert(
            "objects".to_owned(),
            Value::Array(
                item.objects
                    .iter()
                    .map(|o| Value::String(o.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(fmt) = &item.format {
        map.insert("format".to_owned(), Value::String(fmt.clone()));
    }
    if let Some(val) = &item.value {
        map.insert("value".to_owned(), Value::String(val.clone()));
    }
    map
}

/// Post-pass after reading all objects of a sheet: validates every
/// `object_property` / `code` object reference against the sheet's objects and
/// evaluates auto-filled `contents` / attribute `value` strings so rendering,
/// plotting, queries, and the viewer draw the live values deterministically.
pub(super) fn resolve_sheet_fields(
    sheet: &mut Sheet,
    records: &[ObjectRecord],
    sheet_name: Option<&str>,
) -> Result<(), Error> {
    let object_ids: Vec<String> = records.iter().map(|r| r.id.clone()).collect();

    // First validate all referenced object IDs on the sheet.
    for (i, object) in sheet.objects.iter().enumerate() {
        let obj_path = index_path("objects", i);
        match object {
            Object::Text(t) => {
                if let Some(tf) = &t.field {
                    validate_field_refs(tf, &object_ids, &key_path(&obj_path, "text.field"))?;
                }
            }
            Object::MText(t) => {
                if let Some(tf) = &t.field {
                    validate_field_refs(tf, &object_ids, &key_path(&obj_path, "mtext.field"))?;
                }
            }
            Object::Insert(ins) => {
                for att in &ins.attributes {
                    if let Some(tf) = &att.field {
                        validate_field_refs(
                            tf,
                            &object_ids,
                            &key_path(&key_path(&obj_path, "insert.attribute_fields"), &att.tag),
                        )?;
                    }
                }
            }
            _ => {}
        }
    }

    // Snapshot objects for immutable evaluation while mutating display values.
    let snapshot_objects = sheet.objects.clone();
    let default_layout = sheet.layout.as_ref().map(|l| l.name.clone());
    let layout_names: Vec<Option<String>> = (0..sheet.objects.len())
        .map(|i| {
            sheet
                .layout_of(i)
                .map(|l| l.name.clone())
                .or_else(|| default_layout.clone())
        })
        .collect();

    for (i, object) in sheet.objects.iter_mut().enumerate() {
        let ctx = EvalContext {
            sheet_name,
            layout_name: layout_names[i].as_deref(),
            custom_properties: &sheet.custom_properties,
            objects: &snapshot_objects,
            object_ids: &object_ids,
            object_handles: &[],
        };
        match object {
            Object::Text(t) => {
                if let Some(tf) = &t.field {
                    if t.contents.is_empty() {
                        let (_, display) = evaluate_text_field(tf, &ctx);
                        let one_line = display.replace(['\r', '\n'], " ");
                        t.contents = if one_line.trim().is_empty() {
                            UNEVALUATED_FIELD.to_owned()
                        } else {
                            one_line
                        };
                    }
                }
            }
            Object::MText(t) => {
                if let Some(tf) = &t.field {
                    if t.contents.is_empty() {
                        let (_, display) = evaluate_text_field(tf, &ctx);
                        t.contents = if display.trim().is_empty() {
                            UNEVALUATED_FIELD.to_owned()
                        } else {
                            display
                        };
                    }
                }
            }
            Object::Insert(ins) => {
                for att in &mut ins.attributes {
                    if let Some(tf) = &att.field {
                        if att.value.is_empty() {
                            let (_, display) = evaluate_text_field(tf, &ctx);
                            let one_line = display.replace(['\r', '\n'], " ");
                            att.value = if one_line.trim().is_empty() {
                                UNEVALUATED_FIELD.to_owned()
                            } else {
                                one_line
                            };
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_field_refs(
    tf: &TextField,
    object_ids: &[String],
    field_path: &str,
) -> Result<(), Error> {
    for item in &tf.items {
        for r in item.referenced_objects() {
            if r.starts_with('#') {
                continue;
            }
            let local = r.rsplit_once('/').map_or(r, |(_, tail)| tail);
            let exists = object_ids
                .iter()
                .any(|id| id.eq_ignore_ascii_case(r) || id.eq_ignore_ascii_case(local));
            if !exists {
                let choices = if object_ids.is_empty() {
                    "the sheet has no objects".to_owned()
                } else {
                    format!("objects on this sheet: {}", object_ids.join(", "))
                };
                return Err(Error::new(
                    ErrorCode::UnknownName,
                    format!("field references unknown object `{r}`; {choices}"),
                )
                .at_field(field_path.to_owned()));
            }
        }
    }
    Ok(())
}

/// Parses `attribute_fields` on an `insert` in a sheet description, attaching
/// the parsed [`TextField`]s to the matching attribute slots and clearing the
/// default value on slots where no explicit `attributes` value was provided so
/// the post-pass evaluates the field.
pub(super) fn parse_attribute_fields(
    fields: &Fields,
    block: &crate::model::Block,
    definitions: &[&crate::model::AttributeDefinition],
    attributes: &mut [crate::model::Attribute],
    explicit_values: &[bool],
) -> Result<(), Error> {
    let Some(node) = fields.get(ATTRIBUTE_FIELDS_KEY) else {
        return Ok(());
    };
    let path = fields.path_of(ATTRIBUTE_FIELDS_KEY);
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(
            node,
            &path,
            "a mapping from attribute tags to field definitions",
        ));
    };
    let mut given: Vec<(String, &super::strict_yaml::Entry)> = Vec::new();
    for entry in entries {
        let entry_path = key_path(&path, &entry.key);
        let tag = tag_in_capitals(&entry.key);
        if let Some((_, first)) = given.iter().find(|(t, _)| *t == tag) {
            return Err(Error::new(
                ErrorCode::DuplicateKey,
                format!(
                    "attribute field `{tag}` is given twice, as `{}` at line {} and as `{}` here",
                    first.key, first.key_at.line, entry.key
                ),
            )
            .at_field(entry_path)
            .at_position(entry.key_at.line, entry.key_at.column));
        }
        given.push((tag.clone(), entry));
        let places: Vec<usize> = (0..definitions.len())
            .filter(|&i| definitions[i].tag == tag)
            .collect();
        if places.is_empty() {
            let carried: Vec<&str> = definitions.iter().map(|d| d.tag.as_str()).collect();
            let listed = if carried.is_empty() {
                format!("block `{}` has no attributes", block.name)
            } else {
                format!(
                    "block `{}` has the attributes {}",
                    block.name,
                    carried.join(", ")
                )
            };
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("unknown attribute `{}`; {listed}", entry.key),
            )
            .at_field(entry_path)
            .at_position(entry.key_at.line, entry.key_at.column));
        }
        if entry.value.is_bare_null() {
            continue;
        }
        match (&entry.value.kind, places.as_slice()) {
            (Kind::Sequence(_), [_]) => {
                return Err(wrong_type(
                    &entry.value,
                    &entry_path,
                    &format!(
                        "one field mapping: block `{}` defines `{tag}` once, not as a list",
                        block.name
                    ),
                ));
            }
            (_, [i]) => {
                let tf = parse_field(&entry.value, &entry_path)?;
                attributes[*i].field = Some(tf);
                if !explicit_values[*i] {
                    attributes[*i].value.clear();
                }
            }
            (Kind::Sequence(items), _) => {
                if items.len() > places.len() {
                    return Err(bad_value(
                        &entry.value,
                        &entry_path,
                        format!(
                            "block `{}` defines `{tag}` {} times, so `attribute_fields.{}` takes \
                             at most {} entries; this list gives {}",
                            block.name,
                            places.len(),
                            entry.key,
                            places.len(),
                            items.len()
                        ),
                    ));
                }
                for (j, item) in items.iter().enumerate() {
                    if !item.is_bare_null() {
                        let tf = parse_field(item, &index_path(&entry_path, j))?;
                        let slot = places[j];
                        attributes[slot].field = Some(tf);
                        if !explicit_values[slot] {
                            attributes[slot].value.clear();
                        }
                    }
                }
            }
            (_, _) => {
                return Err(wrong_type(
                    &entry.value,
                    &entry_path,
                    &format!(
                        "a list of field mappings (or null for plain slots): block `{}` defines \
                         `{tag}` {} times",
                        block.name,
                        places.len()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// JSON Schema property for top-level `custom_properties`.
pub(super) fn custom_properties_property() -> Value {
    json!({
        "description": "Custom drawing properties (`DWGPROPS`), mapped from property name to \
                        single-line text value, referenced by `custom_property` fields \
                        (`\\AcVar CustomDP.<name>`). `AgentDrafter` is reserved.",
        "type": "object",
        "additionalProperties": {
            "type": ["string", "number", "boolean", "null"],
        },
    })
}

/// Adds `#/$defs/field` and `#/$defs/field_item` to a JSON Schema's `$defs`.
pub(super) fn add_field_defs(defs: &mut Map<String, Value>) {
    defs.insert("field_item".into(), field_item_schema());
    defs.insert("field".into(), field_def_schema());
}

/// Standalone JSON Schema for a field (used by the `set_field` operation).
pub(crate) fn field_schema() -> Value {
    let mut item_props = field_item_properties();
    item_props.insert(
        "template".into(),
        json!({
            "type": "string",
            "description": "Optional template with `{0}` slot for this single field item. \
                            Defaults to `{0}`.",
        }),
    );
    let _ = ITEM_KEYS;
    json!({
        "description": "An AutoCAD field (`FIELD`): either a single field item (`kind`, plus \
                        kind-specific keys and optional `template`, `format`, `value`) or a \
                        multi-field template (`template` with `{0}`, `{1}`, ... and `items`).",
        "anyOf": [
            {
                "type": "object",
                "required": ["kind"],
                "additionalProperties": false,
                "properties": item_props,
            },
            {
                "type": "object",
                "required": ["items"],
                "additionalProperties": false,
                "properties": {
                    "template": {
                        "type": "string",
                        "description": "Template text with `{0}`, `{1}`, ... placeholders for \
                                        each item in `items`. Defaults to `{0}`.",
                    },
                    "items": {
                        "type": "array",
                        "minItems": 1,
                        "items": field_item_schema(),
                    },
                },
            },
        ],
    })
}

fn field_def_schema() -> Value {
    let mut shorthand_props = field_item_properties();
    shorthand_props.insert(
        "template".into(),
        json!({
            "type": "string",
            "description": "Optional template with `{0}` slot for this single field item. \
                            Defaults to `{0}`.",
        }),
    );
    json!({
        "description": "An AutoCAD field (`FIELD`) attached to text, multiline text or an \
                        insert attribute: written as AutoCAD's ACAD_FIELD / FIELD / FIELDLIST \
                        objects so AutoCAD updates it live, and drawn with its current value \
                        through the one drawing pipeline.",
        "anyOf": [
            {
                "type": "object",
                "required": ["kind"],
                "additionalProperties": false,
                "properties": shorthand_props,
            },
            {
                "type": "object",
                "required": ["items"],
                "additionalProperties": false,
                "properties": {
                    "template": {
                        "type": "string",
                        "description": "Template text with `{0}`, `{1}`, ... placeholders for \
                                        each item in `items`. Defaults to `{0}`.",
                    },
                    "items": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "$ref": "#/$defs/field_item" },
                    },
                },
            },
        ],
    })
}

fn field_item_schema() -> Value {
    json!({
        "description": "One AutoCAD child field inside a field.",
        "type": "object",
        "required": ["kind"],
        "additionalProperties": false,
        "properties": field_item_properties(),
    })
}

fn field_item_properties() -> Map<String, Value> {
    let mut props = Map::new();
    props.insert(
        "kind".into(),
        json!({
            "description": "What value the field evaluates to: `filename`, `save_date`, \
                            `plot_date`, `create_date`, `date`, `sheet_number`, \
                            `custom_property`, `object_property`, or `code`.",
            "enum": FieldKind::ALL.map(FieldKind::name),
        }),
    );
    props.insert(
        "property".into(),
        json!({
            "type": "string",
            "minLength": 1,
            "description": "For `custom_property`, the custom drawing property name (`DWGPROPS`). \
                            For `object_property`, the AutoCAD object property name (such as \
                            `Length`, `Area`, `Radius`, `Diameter`, `Circumference`, `Layer`, \
                            `Color` or `Linetype`).",
        }),
    );
    props.insert(
        "object".into(),
        json!({
            "type": "string",
            "minLength": 1,
            "description": "For `object_property`, the local ID of the referenced object on \
                            this sheet (or `#<HEX>` handle on an original DWG).",
        }),
    );
    props.insert(
        "code".into(),
        json!({
            "type": "string",
            "minLength": 1,
            "description": "For `code`, the raw AutoCAD field expression without outer `%<...>%` \
                            (such as `\\AcVar Author` or `\\AcDiesel ...`).",
        }),
    );
    props.insert(
        "objects".into(),
        json!({
            "type": "array",
            "items": { "type": "string", "minLength": 1 },
            "description": "For `code`, optional referenced object IDs matching `\\_ObjIdx 0`, \
                            `1`, ...",
        }),
    );
    props.insert(
        "format".into(),
        json!({
            "type": "string",
            "description": "Optional AutoCAD `\\f \"...\"` format string (such as `%fn2`, \
                            `%fn6`, `yyyy-MM-dd`, `%lu2%pr2`, `%tc1`).",
        }),
    );
    props.insert(
        "value".into(),
        json!({
            "type": "string",
            "description": "Optional cached/explicit display value for this field item.",
        }),
    );
    props
}
