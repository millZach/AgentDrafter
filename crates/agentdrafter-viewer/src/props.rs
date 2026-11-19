//! Editing in quick properties (#153): an object's layer, a text's or an
//! mtext's contents, an insert's attribute values, and a circle's radius
//! or diameter, an arc's radius. Plain computation, tested natively; `web`
//! hands the page the fields and the change set an edit gives, which the
//! page sends through `change` as it sends a drawing tool's.
//!
//! The fields come from what `display-list` gives with `values` and
//! `origins`: each value, and where it is written. Each field says so
//! before it is edited, as Zach decided on 2026-11-19: a value of the
//! template changes every sheet built from it, and the page asks first; a
//! value the sheet's row fills changes on this sheet only, in every place
//! it fills; an attribute the template leaves unset is set on the template,
//! so it lands on every sheet built from it.
//!
//! An edit is one change set with the operation the core has for it:
//! `change` for a layer, contents and a radius, `set_attribute` for an
//! attribute. A diameter typed sends its radius. What the edit means, and
//! where it is written, is the core's (ADR-0007).

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::tools::{f4, number, plural, Context, ObjectInfo, Reach, Send, TemplateEdit};

/// Where a field's value is written, as its chip shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// The template's: an edit changes every sheet built from it.
    Template,
    /// A value the sheet's row fills: an edit changes this sheet alone, in
    /// every place it fills.
    RowValue,
    /// An attribute the insert leaves unset, which shows its default.
    Unset,
}

/// The short word under a field's label.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Chip {
    pub words: String,
    pub tone: Tone,
}

/// One row of quick properties that an edit can change.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Field {
    /// What the page names it to [`edit`]: `layer`, `contents`, `radius`,
    /// `diameter` or `attribute:N`, the attribute's place.
    pub key: String,
    /// Its label: `Layer`, `Contents`, `Radius`, `Diameter`, or an
    /// attribute's tag, with which of a repeated tag's values it is.
    pub label: String,
    /// Its value, as the field shows it.
    pub value: String,
    /// Whether it can be edited here: on a drawing set's sheet as it is,
    /// an object with an ID.
    pub editable: bool,
    /// Whether it takes more than one line: an mtext's contents.
    pub multiline: bool,
    /// Whether it takes a number: a radius or a diameter.
    pub numeric: bool,
    /// Whether it is one of an insert's attributes.
    pub attribute: bool,
    /// Where its value is written, in a word.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chip: Option<Chip>,
    /// What an edit to it changes, said before it is edited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What an edit came to: what went wrong with what was typed, or the
/// change set to send; neither when the value is as it was, so nothing is
/// sent.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Edited {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send: Option<Send>,
}

impl Edited {
    fn error(words: impl Into<String>) -> Self {
        Edited {
            error: Some(words.into()),
            send: None,
        }
    }
}

/// Where a value is written, and so what an edit to it reaches.
#[derive(Clone, Debug, PartialEq)]
enum Lands<'a> {
    /// In the template: every sheet built from it.
    Template(&'a Reach),
    /// In the sheet's row, among its own objects: this sheet.
    Row,
    /// The sheet's row's value `name`, in `fills` places on this sheet.
    RowValue { name: String, fills: usize },
    /// Nowhere yet: an attribute the insert leaves unset, which an edit
    /// sets on the insert, in the template or in the row.
    Unset(Option<&'a Reach>),
    /// The core did not say: a sheet description or a DWG.
    Unknown,
}

/// Where the object itself is written.
fn object_lands<'a>(object: &ObjectInfo, context: &'a Context) -> Lands<'a> {
    match (object.from_template, context.template.as_ref()) {
        (Some(true), Some(reach)) => Lands::Template(reach),
        (Some(false), _) => Lands::Row,
        _ => Lands::Unknown,
    }
}

/// Where a value of the object is written, as `display-list` with
/// `values` gives it at `written`.
fn value_lands<'a>(object: &ObjectInfo, context: &'a Context, written: &Value) -> Lands<'a> {
    match written.get("written").and_then(Value::as_str) {
        Some("row_value") => Lands::RowValue {
            name: written
                .get("row_value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            fills: written.get("fills").and_then(Value::as_u64).unwrap_or(1) as usize,
        },
        Some("unset") => Lands::Unset(match object_lands(object, context) {
            Lands::Template(reach) => Some(reach),
            _ => None,
        }),
        _ => object_lands(object, context),
    }
}

/// The template's file's own name, as the dialog names it.
fn file_of(reach: &Reach) -> &str {
    reach.path.rsplit('/').next().unwrap_or(&reach.path)
}

/// The chip and the note of a value written where `lands` says; `tag` an
/// attribute's.
fn chip_and_note(lands: &Lands, tag: Option<&str>) -> (Option<Chip>, Option<String>) {
    let chip = |words: String, tone| Some(Chip { words, tone });
    match lands {
        Lands::Template(reach) => {
            let n = reach.sheets.len();
            let note = if n > 1 {
                format!(
                    "Part of the template {}: changing it changes all {n} sheets. You will be \
                     asked before it lands.",
                    file_of(reach)
                )
            } else {
                format!(
                    "Part of the template {}, which only this sheet is built from. You will be \
                     asked before it lands.",
                    file_of(reach)
                )
            };
            (chip(plural(n, "sheet", "sheets"), Tone::Template), Some(note))
        }
        // The default: "Comes from" says it, and a chip on each row would
        // crowd a phone's card.
        Lands::Row => (None, None),
        Lands::RowValue { name, fills } => {
            let note = if *fills > 1 {
                format!(
                    "This sheet\u{2019}s row value {name}. It fills {fills} places on this \
                     sheet, and they all change. Only this sheet changes."
                )
            } else {
                format!("This sheet\u{2019}s row value {name}. Only this sheet changes.")
            };
            (
                chip(plural(*fills, "place", "places"), Tone::RowValue),
                Some(note),
            )
        }
        Lands::Unset(reach) => {
            let tag = tag.unwrap_or_default();
            let note = match reach {
                Some(reach) if reach.sheets.len() > 1 => format!(
                    "The template leaves {tag} unset, so it shows its default. Setting it sets \
                     it on the template, so it lands on all {} sheets.",
                    reach.sheets.len()
                ),
                Some(_) => format!(
                    "The template leaves {tag} unset, so it shows its default. Setting it sets \
                     it on the template, which only this sheet is built from."
                ),
                None => format!(
                    "This insert leaves {tag} unset, so it shows its default. Setting it sets \
                     it on this sheet only."
                ),
            };
            (chip("unset".to_owned(), Tone::Unset), Some(note))
        }
        Lands::Unknown => (None, None),
    }
}

/// Why the object cannot be edited here, if it cannot.
fn not_editable(object: &ObjectInfo, context: &Context) -> Option<&'static str> {
    if context.past {
        Some("The set as it was is read-only: go back to live to edit it.")
    } else if context.template.is_none() {
        Some("Only a drawing set's sheet is edited here, through change sets.")
    } else if object.id.is_none() {
        Some("It has no ID, so a change set cannot name it.")
    } else {
        None
    }
}

/// A value the core gives under `values`, at `path`.
fn given<'a>(object: &'a ObjectInfo, key: &str) -> Option<&'a Value> {
    object.values.as_ref()?.get(key)
}

/// The fields of the object at `place` that quick properties edit, in the
/// order they show: its layer, then its contents or its radius and
/// diameter, then its attributes.
pub fn fields(context: &Context, place: usize) -> Vec<Field> {
    let Some(object) = context.objects.get(place) else {
        return Vec::new();
    };
    let editable = not_editable(object, context).is_none();
    let mut fields = Vec::new();
    let field = |key: String, label: String, value: String, lands: &Lands, tag: Option<&str>| {
        let (chip, note) = chip_and_note(lands, tag);
        Field {
            key,
            label,
            value,
            editable,
            multiline: false,
            numeric: false,
            attribute: false,
            chip,
            note: if editable { note } else { None },
        }
    };
    let own = object_lands(object, context);
    fields.push(field(
        "layer".to_owned(),
        "Layer".to_owned(),
        object.layer.clone(),
        &own,
        None,
    ));
    if let Some(contents) = given(object, "contents") {
        let lands = value_lands(object, context, contents);
        let value = contents.get("value").and_then(Value::as_str).unwrap_or("");
        let mut f = field(
            "contents".to_owned(),
            "Contents".to_owned(),
            value.to_owned(),
            &lands,
            None,
        );
        f.multiline = object.kind == "mtext";
        fields.push(f);
    }
    if let Some(radius) = given(object, "radius").and_then(Value::as_f64) {
        let mut r = field(
            "radius".to_owned(),
            "Radius".to_owned(),
            f4(radius),
            &own,
            None,
        );
        r.numeric = true;
        fields.push(r);
        // An arc is edited by its radius alone.
        if object.kind == "circle" {
            let mut d = field(
                "diameter".to_owned(),
                "Diameter".to_owned(),
                f4(radius * 2.0),
                &own,
                None,
            );
            d.numeric = true;
            // The radius's chip says it once.
            d.chip = None;
            fields.push(d);
        }
    }
    if let Some(Value::Array(attributes)) = given(object, "attributes") {
        for (i, attribute) in attributes.iter().enumerate() {
            let tag = attribute.get("tag").and_then(Value::as_str).unwrap_or("");
            let label = match attribute.get("occurrence").and_then(Value::as_u64) {
                Some(n) => format!("{tag} ({n})"),
                None => tag.to_owned(),
            };
            let lands = value_lands(object, context, attribute);
            let value = attribute.get("value").and_then(Value::as_str).unwrap_or("");
            let mut f = field(
                format!("attribute:{i}"),
                label,
                value.to_owned(),
                &lands,
                Some(tag),
            );
            f.attribute = true;
            fields.push(f);
        }
    }
    fields
}

/// What an edit of the field `key` of the object at `place` to `typed`
/// gives: the change set to send, what went wrong with what was typed, or
/// nothing when the value is as it was.
pub fn edit(context: &Context, place: usize, key: &str, typed: &str) -> Edited {
    let Some(object) = context.objects.get(place) else {
        return Edited::error("Nothing is selected.");
    };
    if let Some(why) = not_editable(object, context) {
        return Edited::error(why);
    }
    let id = object.id.clone().unwrap_or_default();
    let words = object.words();
    let own = object_lands(object, context);
    let change = |set: Value| json!({"op": "change", "target": {"id": id}, "set": set});
    let (operation, what, lands, asked): (Value, String, Lands, Asked) = match key {
        "layer" => {
            let name = typed.trim();
            if name.is_empty() {
                return Edited::error("Pick a layer.");
            }
            if name == object.layer {
                return Edited::default();
            }
            (
                change(json!({"layer": name})),
                format!("{words} moved to layer {name}"),
                own,
                Asked::Change("its layer"),
            )
        }
        "contents" => {
            let Some(contents) = given(object, "contents") else {
                return Edited::error("Only a text or an mtext has contents.");
            };
            let mtext = object.kind == "mtext";
            let text = if mtext {
                typed.replace("\r\n", "\n")
            } else {
                typed.to_owned()
            };
            if !mtext && text.contains('\n') {
                return Edited::error("A text is one line.");
            }
            if text.trim().is_empty() {
                return Edited::error("A text needs something to show.");
            }
            if contents.get("value").and_then(Value::as_str) == Some(text.as_str()) {
                return Edited::default();
            }
            (
                change(json!({"contents": text})),
                format!("{words} text set to \u{201c}{}\u{201d}", short(&text)),
                value_lands(object, context, contents),
                Asked::Change("its text"),
            )
        }
        "radius" | "diameter" => {
            let Some(radius) = given(object, "radius").and_then(Value::as_f64) else {
                return Edited::error("Only a circle or an arc has a radius.");
            };
            let diameter = key == "diameter";
            if diameter && object.kind != "circle" {
                return Edited::error("An arc is edited by its radius.");
            }
            let what = if diameter { "diameter" } else { "radius" };
            let Some(n) = number(typed) else {
                return Edited::error(format!("Type the {what} as a number, such as 0.25."));
            };
            if n <= 0.0 {
                return Edited::error(format!("A {what} is more than 0."));
            }
            let r = if diameter { n / 2.0 } else { n };
            if (r - radius).abs() <= radius.abs() * 1e-12 {
                return Edited::default();
            }
            let typed_words = if diameter {
                format!(" (diameter {} typed)", trimmed(n))
            } else {
                String::new()
            };
            (
                change(json!({"radius": r})),
                format!("{words} radius set to {}{typed_words}", trimmed(r)),
                own,
                Asked::Change("its radius"),
            )
        }
        _ => {
            let at = key
                .strip_prefix("attribute:")
                .and_then(|i| i.parse::<usize>().ok());
            let attribute = match (at, given(object, "attributes")) {
                (Some(i), Some(Value::Array(all))) => all.get(i),
                _ => None,
            };
            let Some(attribute) = attribute else {
                return Edited::error("There is no such field to edit.");
            };
            let tag = attribute
                .get("tag")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if typed.contains('\n') {
                return Edited::error("An attribute's value is one line.");
            }
            // A field left as it shows lands nothing, an unset one's
            // default among them.
            if attribute.get("value").and_then(Value::as_str) == Some(typed) {
                return Edited::default();
            }
            let unset = attribute.get("written").and_then(Value::as_str) == Some("unset");
            let mut operation = Map::new();
            operation.insert("op".to_owned(), json!("set_attribute"));
            operation.insert("target".to_owned(), json!({"id": id}));
            operation.insert("tag".to_owned(), json!(tag));
            if let Some(n) = attribute.get("occurrence").and_then(Value::as_u64) {
                operation.insert("occurrence".to_owned(), json!(n));
            }
            operation.insert("value".to_owned(), json!(typed));
            let lands = value_lands(object, context, attribute);
            let asked = if unset {
                Asked::Unset(tag.clone())
            } else {
                Asked::Attribute(tag.clone())
            };
            (
                Value::Object(operation),
                format!("{words} {tag} set to \u{201c}{}\u{201d}", short(typed)),
                lands,
                asked,
            )
        }
    };
    let (sheets, template) = match &lands {
        Lands::Template(reach) | Lands::Unset(Some(reach)) => {
            (reach.sheets.clone(), Some(asked.ask(reach)))
        }
        _ => (vec![context.sheet.clone()], None),
    };
    Edited {
        error: None,
        send: Some(Send {
            change_set: json!({"operations": [operation]}),
            what,
            sheets,
            template,
        }),
    }
}

/// What the template dialog asks of an edit.
#[derive(Clone, Debug, PartialEq)]
enum Asked {
    /// A change of the object's `what`, such as `its layer`.
    Change(&'static str),
    /// A change of an attribute the template sets.
    Attribute(String),
    /// Setting an attribute the template leaves unset.
    Unset(String),
}

impl Asked {
    fn ask(&self, reach: &Reach) -> TemplateEdit {
        let n = reach.sheets.len();
        let (title, detail) = match self {
            Asked::Unset(tag) => (
                format!("The template leaves {tag} unset."),
                if n > 1 {
                    format!("Setting it sets it on the template, so it lands on all {n} sheets.")
                } else {
                    "Setting it sets it on the template, which only this sheet is built from."
                        .to_owned()
                },
            ),
            _ => {
                let what = match self {
                    Asked::Change(what) => (*what).to_owned(),
                    Asked::Attribute(tag) => format!("its {tag}"),
                    Asked::Unset(_) => unreachable!("handled above"),
                };
                (
                    "This is part of the template.".to_owned(),
                    if n > 1 {
                        format!("Changing {what} changes {n} sheets.")
                    } else {
                        format!(
                            "Changing {what} changes the template, which only this sheet is \
                             built from."
                        )
                    },
                )
            }
        };
        TemplateEdit {
            title,
            detail,
            path: reach.path.clone(),
            sheets: reach.sheets.clone(),
        }
    }
}

/// A number in words, with no trailing zeros: `0.5`, `12`.
fn trimmed(v: f64) -> String {
    let words = format!("{v:.6}");
    let words = words.trim_end_matches('0').trim_end_matches('.');
    if words == "-0" {
        "0".to_owned()
    } else {
        words.to_owned()
    }
}

/// Text in a few words, for a line of the history: its first line, cut at
/// 40 characters.
fn short(text: &str) -> String {
    let first = text.lines().next().unwrap_or_default();
    let more = text.lines().nth(1).is_some();
    let cut: String = first.chars().take(40).collect();
    if cut.chars().count() < first.chars().count() || more {
        format!("{cut}\u{2026}")
    } else {
        cut
    }
}

#[cfg(test)]
#[path = "props_tests.rs"]
mod tests;
