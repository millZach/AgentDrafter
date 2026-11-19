//! Quick properties' edits, natively: the fields each object shows, what
//! each says of where its value is written, and the change set an edit
//! gives, its operation, its sheets and the template's question.

use super::*;
use serde_json::json;

/// A made-up sheet like a demo plant loop sheet, on a template three
/// sheets share: the template's title, a wire number from the row, a title
/// block and an arc, and the row's own device, circle and note.
fn context() -> Context {
    let object = |id: Option<&str>, kind: &str, template: bool, values: Value| ObjectInfo {
        id: id.map(|id| format!("DP-LD-101/{id}")),
        name: None,
        kind: kind.to_owned(),
        block: None,
        layer: "notes".to_owned(),
        values: (!values.is_null()).then_some(values),
        from_template: Some(template),
        through_viewport: false,
    };
    Context {
        sheet: "DP-LD-101".to_owned(),
        units: Some("in".to_owned()),
        objects: vec![
            object(
                Some("title"),
                "text",
                true,
                json!({"contents": {"value": "FIELD", "written": "own"}}),
            ),
            object(
                Some("objects[19]"),
                "text",
                true,
                json!({"contents": {"value": "101+", "written": "row_value", "row_value": "WIRE_1", "fills": 4}}),
            ),
            object(
                Some("title-block"),
                "insert",
                true,
                json!({"attributes": [
                    {"tag": "REV", "value": "0", "written": "row_value", "row_value": "REV", "fills": 2},
                    {"tag": "DRAWN", "value": "AD", "written": "own"},
                    {"tag": "CHECKED", "value": "", "written": "unset"},
                    {"tag": "NOTE", "occurrence": 1, "value": "A", "written": "own"},
                    {"tag": "NOTE", "occurrence": 2, "value": "", "written": "unset"}
                ]}),
            ),
            object(
                Some("device"),
                "insert",
                false,
                json!({"attributes": [
                    {"tag": "FUNC", "value": "FT", "written": "own"},
                    {"tag": "BY", "value": "AD", "written": "unset"}
                ]}),
            ),
            object(Some("washer"), "circle", false, json!({"radius": 0.25})),
            object(Some("bend"), "arc", true, json!({"radius": 0.5})),
            object(
                Some("note"),
                "mtext",
                false,
                json!({"contents": {"value": "ONE\nTWO", "written": "own"}}),
            ),
            object(None, "circle", false, json!({"radius": 1.0})),
            object(Some("border"), "polyline", true, Value::Null),
        ],
        template: Some(Reach {
            path: "templates/loop-2-wire.yaml".to_owned(),
            sheets: vec![
                "DP-LD-101".to_owned(),
                "DP-LD-102".to_owned(),
                "DP-LD-104".to_owned(),
            ],
        }),
        past: false,
        space: None,
        layout: None,
    }
}

const TITLE: usize = 0;
const WIRE: usize = 1;
const TITLE_BLOCK: usize = 2;
const DEVICE: usize = 3;
const WASHER: usize = 4;
const BEND: usize = 5;
const NOTE: usize = 6;
const NO_ID: usize = 7;
const BORDER: usize = 8;

fn field<'a>(fields: &'a [Field], key: &str) -> &'a Field {
    fields
        .iter()
        .find(|f| f.key == key)
        .unwrap_or_else(|| panic!("no field {key}"))
}

fn sent(edited: &Edited) -> &Send {
    assert_eq!(edited.error, None);
    edited.send.as_ref().expect("a change set to send")
}

fn operation(edited: &Edited) -> Value {
    sent(edited).change_set["operations"][0].clone()
}

#[test]
fn a_value_the_row_fills_says_its_row_value_and_its_places_before_it_is_edited() {
    let context = context();
    let fields = fields(&context, WIRE);
    let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["layer", "contents"]);
    let contents = field(&fields, "contents");
    assert_eq!(contents.value, "101+");
    assert!(contents.editable && !contents.multiline && !contents.numeric);
    assert_eq!(
        contents.chip,
        Some(Chip {
            words: "4 places".to_owned(),
            tone: Tone::RowValue
        })
    );
    assert_eq!(
        contents.note.as_deref(),
        Some(
            "This sheet\u{2019}s row value WIRE_1. It fills 4 places on this sheet, and they \
             all change. Only this sheet changes."
        )
    );
    // Its layer is the template's.
    let layer = field(&fields, "layer");
    assert_eq!(layer.value, "notes");
    assert_eq!(layer.chip.as_ref().unwrap().words, "3 sheets");
    assert_eq!(layer.chip.as_ref().unwrap().tone, Tone::Template);
    assert!(layer
        .note
        .as_deref()
        .unwrap()
        .starts_with("Part of the template loop-2-wire.yaml: changing it changes all 3 sheets."));

    // Its edit sets the row's value, on this sheet alone: nothing is asked.
    let edited = edit(&context, WIRE, "contents", "101A+");
    assert_eq!(
        operation(&edited),
        json!({"op": "change", "target": {"id": "DP-LD-101/objects[19]"}, "set": {"contents": "101A+"}})
    );
    assert_eq!(sent(&edited).sheets, ["DP-LD-101"]);
    assert_eq!(sent(&edited).template, None);
    assert_eq!(sent(&edited).what, "text text set to \u{201c}101A+\u{201d}");
}

#[test]
fn a_value_joined_from_parts_names_its_row_values_and_sends_its_whole_text() {
    let mut context = context();
    context.objects.push(ObjectInfo {
        id: Some("DP-LD-101/slot".to_owned()),
        name: None,
        kind: "text".to_owned(),
        block: None,
        layer: "notes".to_owned(),
        values: Some(json!({"contents": {
            "value": "PANEL P1 SLOT 3",
            "written": "row_values",
            "row_values": [{"name": "PANEL", "fills": 2}, {"name": "SLOT", "fills": 1}]
        }})),
        from_template: Some(true),
        through_viewport: false,
    });
    let slot = context.objects.len() - 1;
    let fields = fields(&context, slot);
    let contents = field(&fields, "contents");
    assert_eq!(
        contents.chip,
        Some(Chip {
            words: "2 row values".to_owned(),
            tone: Tone::RowValue
        })
    );
    assert_eq!(
        contents.note.as_deref(),
        Some(
            "Joined from this sheet\u{2019}s row values PANEL and SLOT and the template\u{2019}s \
             fixed text. Edit the whole text, keeping the fixed text as it is, and each value is \
             set. PANEL fills 2 places on this sheet, and they all change. Only this sheet \
             changes."
        )
    );
    // The whole text goes in the change the page sends now, on this sheet
    // alone, with nothing asked: the core matches it to the parts.
    let edited = edit(&context, slot, "contents", "PANEL P2 SLOT 4");
    assert_eq!(
        operation(&edited),
        json!({"op": "change", "target": {"id": "DP-LD-101/slot"}, "set": {"contents": "PANEL P2 SLOT 4"}})
    );
    assert_eq!(sent(&edited).sheets, ["DP-LD-101"]);
    assert_eq!(sent(&edited).template, None);
}

#[test]
fn a_templates_value_asks_first_naming_every_sheet() {
    let context = context();
    let edited = edit(&context, TITLE, "contents", "FIELD DEVICES");
    let send = sent(&edited);
    assert_eq!(send.sheets, ["DP-LD-101", "DP-LD-102", "DP-LD-104"]);
    let ask = send.template.as_ref().expect("asked first");
    assert_eq!(ask.title, "This is part of the template.");
    assert_eq!(ask.detail, "Changing its text changes 3 sheets.");
    assert_eq!(ask.path, "templates/loop-2-wire.yaml");
    assert_eq!(send.what, "title text set to \u{201c}FIELD DEVICES\u{201d}");

    // Its layer too.
    let edited = edit(&context, TITLE, "layer", "wire-number");
    assert_eq!(
        operation(&edited),
        json!({"op": "change", "target": {"id": "DP-LD-101/title"}, "set": {"layer": "wire-number"}})
    );
    let ask = sent(&edited).template.as_ref().unwrap();
    assert_eq!(ask.detail, "Changing its layer changes 3 sheets.");
    assert_eq!(sent(&edited).what, "title moved to layer wire-number");

    // A template only this sheet is built from says so.
    let mut one = context.clone();
    one.template.as_mut().unwrap().sheets.truncate(1);
    let edited = edit(&one, TITLE, "layer", "wire-number");
    assert_eq!(
        sent(&edited).template.as_ref().unwrap().detail,
        "Changing its layer changes the template, which only this sheet is built from."
    );
}

#[test]
fn a_rows_own_object_changes_this_sheet_alone() {
    let context = context();
    let edited = edit(&context, WASHER, "layer", "wire");
    assert_eq!(sent(&edited).sheets, ["DP-LD-101"]);
    assert_eq!(sent(&edited).template, None);
    let fields = fields(&context, WASHER);
    assert_eq!(
        field(&fields, "layer").chip,
        None,
        "the row's own, as Comes from says"
    );
    assert_eq!(field(&fields, "layer").note, None);
}

#[test]
fn a_circles_radius_and_diameter_both_send_its_radius() {
    let context = context();
    let fields = fields(&context, WASHER);
    let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["layer", "radius", "diameter"]);
    assert_eq!(field(&fields, "radius").value, "0.2500");
    assert_eq!(field(&fields, "diameter").value, "0.5000");
    assert!(field(&fields, "radius").numeric && field(&fields, "diameter").numeric);

    let edited = edit(&context, WASHER, "radius", "0.5");
    assert_eq!(
        operation(&edited),
        json!({"op": "change", "target": {"id": "DP-LD-101/washer"}, "set": {"radius": 0.5}})
    );
    assert_eq!(sent(&edited).what, "washer radius set to 0.5");
    let edited = edit(&context, WASHER, "diameter", " 1 ");
    assert_eq!(
        operation(&edited),
        json!({"op": "change", "target": {"id": "DP-LD-101/washer"}, "set": {"radius": 0.5}}),
        "a diameter typed sends its radius"
    );
    assert_eq!(
        sent(&edited).what,
        "washer radius set to 0.5 (diameter 1 typed)"
    );
    let edited = edit(&context, WASHER, "diameter", "0.3");
    assert_eq!(operation(&edited)["set"]["radius"], json!(0.15));

    // The same value lands nothing; a wrong one says why.
    assert_eq!(edit(&context, WASHER, "radius", "0.25"), Edited::default());
    assert_eq!(edit(&context, WASHER, "diameter", "0.5"), Edited::default());
    assert_eq!(
        edit(&context, WASHER, "radius", "-1").error.as_deref(),
        Some("A radius is more than 0.")
    );
    assert_eq!(
        edit(&context, WASHER, "diameter", "0").error.as_deref(),
        Some("A diameter is more than 0.")
    );
    assert_eq!(
        edit(&context, WASHER, "radius", "half").error.as_deref(),
        Some("Type the radius as a number, such as 0.25.")
    );
}

#[test]
fn an_arc_has_its_radius_alone() {
    let context = context();
    let fields = fields(&context, BEND);
    let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, ["layer", "radius"]);
    assert_eq!(
        edit(&context, BEND, "diameter", "2").error.as_deref(),
        Some("An arc is edited by its radius.")
    );
    let edited = edit(&context, BEND, "radius", "0.75");
    assert_eq!(operation(&edited)["set"], json!({"radius": 0.75}));
    assert_eq!(
        sent(&edited).template.as_ref().unwrap().detail,
        "Changing its radius changes 3 sheets."
    );
}

#[test]
fn attributes_say_where_each_is_written_and_set_attribute_sets_them() {
    let context = context();
    let fields = fields(&context, TITLE_BLOCK);
    let labels: Vec<&str> = fields
        .iter()
        .filter(|f| f.attribute)
        .map(|f| f.label.as_str())
        .collect();
    assert_eq!(labels, ["REV", "DRAWN", "CHECKED", "NOTE (1)", "NOTE (2)"]);
    assert_eq!(
        field(&fields, "attribute:0").chip.as_ref().unwrap().words,
        "2 places"
    );
    assert_eq!(
        field(&fields, "attribute:1").chip.as_ref().unwrap().words,
        "3 sheets"
    );
    let checked = field(&fields, "attribute:2");
    assert_eq!(
        checked.chip,
        Some(Chip {
            words: "unset".to_owned(),
            tone: Tone::Unset
        })
    );
    assert_eq!(
        checked.note.as_deref(),
        Some(
            "The template leaves CHECKED unset, so it shows its default. Setting it sets it on \
             the template, so it lands on all 3 sheets."
        )
    );

    // A row's value: this sheet alone.
    let edited = edit(&context, TITLE_BLOCK, "attribute:0", "A");
    assert_eq!(
        operation(&edited),
        json!({"op": "set_attribute", "target": {"id": "DP-LD-101/title-block"}, "tag": "REV", "value": "A"})
    );
    assert_eq!(sent(&edited).sheets, ["DP-LD-101"]);
    assert_eq!(sent(&edited).template, None);
    assert_eq!(
        sent(&edited).what,
        "title-block REV set to \u{201c}A\u{201d}"
    );

    // The template's own value: asked first.
    let edited = edit(&context, TITLE_BLOCK, "attribute:1", "ZM");
    assert_eq!(
        sent(&edited).template.as_ref().unwrap().detail,
        "Changing its DRAWN changes 3 sheets."
    );

    // One the template leaves unset: set on the template, every sheet.
    let edited = edit(&context, TITLE_BLOCK, "attribute:2", "ZM");
    let ask = sent(&edited).template.as_ref().unwrap();
    assert_eq!(ask.title, "The template leaves CHECKED unset.");
    assert_eq!(
        ask.detail,
        "Setting it sets it on the template, so it lands on all 3 sheets."
    );
    assert_eq!(sent(&edited).sheets.len(), 3);

    // A repeated tag names its occurrence.
    let edited = edit(&context, TITLE_BLOCK, "attribute:4", "B");
    assert_eq!(
        operation(&edited),
        json!({"op": "set_attribute", "target": {"id": "DP-LD-101/title-block"}, "tag": "NOTE", "occurrence": 2, "value": "B"})
    );

    // As it shows, nothing; an unset one left at its default, nothing.
    assert_eq!(
        edit(&context, TITLE_BLOCK, "attribute:1", "AD"),
        Edited::default()
    );
    assert_eq!(
        edit(&context, TITLE_BLOCK, "attribute:2", ""),
        Edited::default()
    );
    assert_eq!(
        edit(&context, TITLE_BLOCK, "attribute:9", "X")
            .error
            .as_deref(),
        Some("There is no such field to edit.")
    );

    // The row's own insert: an unset value lands on this sheet.
    let fields = super::fields(&context, DEVICE);
    assert_eq!(
        field(&fields, "attribute:1").note.as_deref(),
        Some("This insert leaves BY unset, so it shows its default. Setting it sets it on this sheet only.")
    );
    let edited = edit(&context, DEVICE, "attribute:1", "ZM");
    assert_eq!(sent(&edited).template, None);
    assert_eq!(sent(&edited).sheets, ["DP-LD-101"]);
}

#[test]
fn contents_are_one_line_for_a_text_and_more_for_an_mtext() {
    let context = context();
    assert!(field(&fields(&context, NOTE), "contents").multiline);
    let edited = edit(&context, NOTE, "contents", "ONE\r\nTWO\nTHREE");
    assert_eq!(
        operation(&edited)["set"],
        json!({"contents": "ONE\nTWO\nTHREE"})
    );
    assert_eq!(
        sent(&edited).what,
        "note text set to \u{201c}ONE\u{2026}\u{201d}"
    );
    assert_eq!(
        edit(&context, NOTE, "contents", "ONE\nTWO"),
        Edited::default()
    );
    assert_eq!(
        edit(&context, TITLE, "contents", "A\nB").error.as_deref(),
        Some("A text is one line.")
    );
    // A lone CR is a line break, as LF is.
    assert_eq!(
        edit(&context, TITLE, "contents", "A\rB").error.as_deref(),
        Some("A text is one line.")
    );
    assert_eq!(
        edit(&context, TITLE_BLOCK, "attribute:1", "A\rB")
            .error
            .as_deref(),
        Some("An attribute's value is one line.")
    );
    assert_eq!(
        edit(&context, TITLE, "contents", "  ").error.as_deref(),
        Some("A text needs something to show.")
    );
    assert_eq!(
        edit(&context, TITLE, "contents", "FIELD"),
        Edited::default()
    );
}

#[test]
fn an_object_that_cannot_be_edited_shows_its_fields_read_only() {
    let context = context();
    // No ID.
    assert!(fields(&context, NO_ID)
        .iter()
        .all(|f| !f.editable && f.note.is_none()));
    assert_eq!(
        edit(&context, NO_ID, "radius", "2").error.as_deref(),
        Some("It has no ID, so a change set cannot name it.")
    );
    // The set as it was.
    let mut past = context.clone();
    past.past = true;
    assert!(fields(&past, WIRE).iter().all(|f| !f.editable));
    assert!(edit(&past, WIRE, "contents", "X")
        .error
        .unwrap()
        .contains("read-only"));
    // A sheet description, which a change set does not edit.
    let mut alone = context.clone();
    alone.template = None;
    for object in &mut alone.objects {
        object.from_template = None;
    }
    let fields_alone = fields(&alone, WASHER);
    assert!(fields_alone.iter().all(|f| !f.editable && f.chip.is_none()));
    assert!(edit(&alone, WASHER, "radius", "2").error.is_some());
    // An object with nothing but its layer to edit.
    let keys: Vec<String> = fields(&context, BORDER)
        .into_iter()
        .map(|f| f.key)
        .collect();
    assert_eq!(keys, ["layer"]);
    // Nothing there.
    assert!(fields(&context, 99).is_empty());
    assert_eq!(
        edit(&context, 99, "layer", "0").error.as_deref(),
        Some("Nothing is selected.")
    );
    assert_eq!(
        edit(&context, WASHER, "layer", " "),
        Edited::error("Pick a layer.")
    );
    assert_eq!(edit(&context, WASHER, "layer", "notes"), Edited::default());
}

#[test]
fn a_radius_or_diameter_as_it_shows_lands_nothing_though_the_radius_has_more_digits() {
    let mut context = context();
    context.objects[WASHER].values = Some(json!({"radius": 0.123456}));
    let fields = fields(&context, WASHER);
    assert_eq!(field(&fields, "radius").value, "0.1235");
    assert_eq!(field(&fields, "diameter").value, "0.2469");
    // Enter on the field as it shows, or put back to it, sends nothing.
    assert_eq!(
        edit(&context, WASHER, "radius", "0.1235"),
        Edited::default()
    );
    assert_eq!(
        edit(&context, WASHER, "diameter", " 0.2469 "),
        Edited::default()
    );
    // A value typed sends it.
    let edited = edit(&context, WASHER, "radius", "0.12");
    assert_eq!(operation(&edited)["set"], json!({"radius": 0.12}));
}
