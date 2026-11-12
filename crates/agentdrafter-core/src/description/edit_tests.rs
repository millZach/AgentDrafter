//! Editing a record in place: only what an edit touches is rewritten.

use std::path::Path;

use serde_json::{json, Map, Value};

use super::edit::{AttributeSet, ObjectList, RecordFile};
use crate::units::Unit;

const TEMPLATE: &str = include_str!("../../../../examples/demo-set/templates/ansi-b.yaml");
const ROWS: &str = include_str!("../../../../examples/demo-set/rows.yaml");

fn file(text: &str) -> RecordFile {
    RecordFile::from_text(Path::new("record.yaml"), text.to_owned()).unwrap()
}

fn keys(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("an object"),
    }
}

/// The lines of `after` that are not in `before`, and those of `before`
/// that are not in `after`, in order.
fn diff(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    let gone = b.iter().filter(|l| !a.contains(l)).map(|l| l.to_string());
    let new = a.iter().filter(|l| !b.contains(l)).map(|l| l.to_string());
    (new.collect(), gone.collect())
}

#[test]
fn nothing_edited_writes_nothing() {
    assert_eq!(file(TEMPLATE).new_text().unwrap(), None);
}

#[test]
fn moving_an_object_rewrites_its_line_alone() {
    let mut record = file(TEMPLATE);
    record
        .move_by(ObjectList::Template, 1, [0.25, 0.0, 0.0], Unit::In, 0)
        .unwrap();
    let after = record.new_text().unwrap().unwrap();
    let (new, gone) = diff(TEMPLATE, &after);
    assert_eq!(
        new,
        ["  - polyline: {points: [[0.75, 0.5, 0], [16.75, 0.5, 0], [16.75, 10.5, 0], [0.75, 10.5, 0]], closed: true, layer: border, id: border}"]
    );
    assert_eq!(gone.len(), 1);
    assert_eq!(after.lines().count(), TEMPLATE.lines().count());
    assert!(after.starts_with("# A made-up 11 x 17 inch sheet"));
}

#[test]
fn a_block_object_keeps_its_layout_and_its_quotes() {
    let mut record = file(TEMPLATE);
    record
        .move_by(ObjectList::Template, 4, [0.0, -0.5, 0.0], Unit::In, 0)
        .unwrap();
    let after = record.new_text().unwrap().unwrap();
    let (new, gone) = diff(TEMPLATE, &after);
    assert_eq!(new, ["      position: [10, 4, 0]"]);
    assert_eq!(gone, ["      position: [10, 4.5, 0]"]);
}

#[test]
fn an_attribute_the_row_fills_is_left_to_the_row() {
    let mut record = file(TEMPLATE);
    let set = record
        .set_attribute(ObjectList::Template, 2, "TITLE", "NEW", 0)
        .unwrap();
    assert_eq!(set, AttributeSet::FromRow("TITLE".to_owned()));
    assert_eq!(record.new_text().unwrap(), None);
    let set = record
        .set_attribute(ObjectList::Template, 2, "DRAWN", "ZM", 0)
        .unwrap();
    assert_eq!(set, AttributeSet::Written);
    let after = record.new_text().unwrap().unwrap();
    let (new, gone) = diff(TEMPLATE, &after);
    assert_eq!(new, ["        DRAWN: ZM"]);
    assert_eq!(gone, ["        DRAWN: AD"]);
    assert_eq!(
        record.row_value_places("TITLE"),
        [
            (2, "attributes.TITLE".to_owned()),
            (3, "contents".to_owned())
        ]
    );
}

#[test]
fn a_row_value_is_set_where_it_is_written() {
    let mut record = file(ROWS);
    assert_eq!(record.row_value(1, "REV").as_deref(), Some("0"));
    record.set_row_value(1, "REV", "B").unwrap();
    record.set_row_value(2, "TITLE", "A: TITLE").unwrap();
    let after = record.new_text().unwrap().unwrap();
    let expected = ROWS
        .replace(
            "SHEET: 2 OF 3\n      REV: '0'",
            "SHEET: 2 OF 3\n      REV: B",
        )
        .replace("TITLE: AREA 300 GENERAL ARRANGEMENT", "TITLE: 'A: TITLE'");
    assert_eq!(after, expected);
}

#[test]
fn an_object_added_to_a_row_gives_it_objects() {
    let mut record = file(ROWS);
    let line = keys(json!({"to": [2, 2, 0], "from": [1, 1, 0], "layer": "notes", "id": "line-1"}));
    let index = record.add(ObjectList::Row(1), "line", &line, 0).unwrap();
    assert_eq!(index, 0);
    let after = record.new_text().unwrap().unwrap();
    let expected = ROWS.replace(
        "      DATE: 2026-11-11\n  - sheet: DEMO-003",
        "      DATE: 2026-11-11\n    objects:\n      - line: {from: [1, 1, 0], to: [2, 2, 0], layer: notes, id: line-1}\n  - sheet: DEMO-003",
    );
    assert_eq!(after, expected);
    let listed = record.objects(ObjectList::Row(1));
    assert_eq!(listed[0].id, "line-1");
    assert!(listed[0].written);
}

#[test]
fn a_second_object_goes_after_the_first() {
    let mut record = file(ROWS);
    let a = keys(json!({"center": [1, 1, 0], "radius": 0.5, "id": "a"}));
    let b = keys(json!({"center": [2, 1, 0], "radius": "0.5 in", "id": "b"}));
    record.add(ObjectList::Row(2), "circle", &a, 0).unwrap();
    record.add(ObjectList::Row(2), "circle", &b, 1).unwrap();
    let after = record.new_text().unwrap().unwrap();
    assert!(after.ends_with(
        "      DATE: 2026-11-11\n    objects:\n      - circle: {center: [1, 1, 0], radius: 0.5, id: a}\n      - circle: {center: [2, 1, 0], radius: 0.5 in, id: b}\n"
    ));
    let mut again = file(&after);
    let c = keys(json!({"center": [3, 1, 0], "radius": 1, "id": "c"}));
    again.add(ObjectList::Row(2), "circle", &c, 0).unwrap();
    let third = again.new_text().unwrap().unwrap();
    assert!(third.ends_with("id: b}\n      - circle: {center: [3, 1, 0], radius: 1, id: c}\n"));
}

#[test]
fn deleting_an_object_writes_the_places_of_those_after_it_that_have_none() {
    let mut record = file(TEMPLATE);
    assert_eq!(record.objects(ObjectList::Template)[2].id, "objects[2]");
    let kept = record.delete(ObjectList::Template, 0, 0);
    assert_eq!(kept, ["objects[2]"]);
    let listed = record.objects(ObjectList::Template);
    let ids: Vec<&str> = listed.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, ["border", "objects[2]", "heading", "notes"]);
    let after = record.new_text().unwrap().unwrap();
    assert!(!after.contains("layer: trim}"));
    assert!(after.contains("        DRAWN: AD\n      id: objects[2]\n  - text:"));
    let reread = file(&after);
    assert_eq!(reread.objects(ObjectList::Template)[1].id, "objects[2]");
}

#[test]
fn an_id_or_a_name_that_is_a_bare_null_is_left_out() {
    let text = "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1}\n  - circle: {center: [5, 0, 0], radius: 1, id: null, name: ~}\n  - circle: {center: [9, 0, 0], radius: 1, id: 'null'}\n";
    let mut record = file(text);
    let listed = record.objects(ObjectList::Template);
    let ids: Vec<(&str, bool)> = listed.iter().map(|l| (l.id.as_str(), l.written)).collect();
    assert_eq!(
        ids,
        [("objects[0]", false), ("objects[1]", false), ("null", true)]
    );
    assert_eq!(listed[1].name, None);
    // Deleting the first writes the second's place in its `id`, in place
    // of the bare null, so its ID does not move.
    assert_eq!(record.delete(ObjectList::Template, 0, 0), ["objects[1]"]);
    let after = record.new_text().unwrap().unwrap();
    assert!(
        after.contains("radius: 1, id: 'objects[1]', name: ~}"),
        "{after}"
    );
    assert!(after.contains("id: 'null'}"), "{after}");
}

#[test]
fn a_list_in_brackets_becomes_a_list_of_items() {
    let text = "units: mm\nobjects: []  # none yet\n";
    let mut record = file(text);
    let line = keys(json!({"from": [0, 0, 0], "to": [1, 0, 0], "id": "l"}));
    record.add(ObjectList::Template, "line", &line, 0).unwrap();
    assert_eq!(
        record.new_text().unwrap().unwrap(),
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0], id: l}  # none yet\n"
    );
}

#[test]
fn deleting_the_last_object_leaves_an_empty_list() {
    let text = "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 0, 0]}\n# end\n";
    let mut record = file(text);
    record.delete(ObjectList::Template, 0, 0);
    assert_eq!(
        record.new_text().unwrap().unwrap(),
        "units: mm\nobjects: []\n# end\n"
    );
}

#[test]
fn a_change_sets_keys_and_takes_them_away() {
    let mut record = file(TEMPLATE);
    let set = keys(json!({"height": 0.3, "layer": null, "name": "heading-text"}));
    let changed = record.change(ObjectList::Template, 3, &set, 0).unwrap();
    assert!(changed.edited);
    assert_eq!(changed.from_row, None);
    let after = record.new_text().unwrap().unwrap();
    let (new, _) = diff(TEMPLATE, &after);
    assert_eq!(
        new,
        ["  - text: {contents: {row: TITLE}, position: [1, 10, 0], height: 0.3, style: notes, id: heading, name: heading-text}"]
    );
    let contents = keys(json!({"contents": "NEW HEADING"}));
    let changed = record
        .change(ObjectList::Template, 3, &contents, 1)
        .unwrap();
    assert!(!changed.edited);
    assert_eq!(
        changed.from_row,
        Some(("TITLE".to_owned(), "NEW HEADING".to_owned()))
    );
    let id = record
        .change(ObjectList::Template, 3, &keys(json!({"id": "x"})), 2)
        .unwrap_err();
    assert!(id.message.contains("fixed for its life"), "{}", id.message);
    let unknown = record
        .change(ObjectList::Template, 3, &keys(json!({"colour": 1})), 2)
        .unwrap_err();
    assert!(
        unknown.message.contains(
            "contents, position, height, style, width_factor, rotation, justify, layer, name"
        ),
        "{}",
        unknown.message
    );
}

#[test]
fn multiline_contents_are_written_as_a_literal_block() {
    let mut record = file(TEMPLATE);
    let set = keys(json!({"contents": "NOTES:\n1. CHANGED.\n"}));
    record.change(ObjectList::Template, 4, &set, 0).unwrap();
    let after = record.new_text().unwrap().unwrap();
    assert!(after.contains(
        "      contents: |\n        NOTES:\n        1. CHANGED.\n      position: [10, 4.5, 0]\n"
    ));
}

#[test]
fn a_hatch_moves_every_loop() {
    let text = "units: mm\nobjects:\n  - hatch:\n      pattern: SOLID\n      boundary:\n        - polyline: {points: [[0, 0, 0], [10, 0, 0], [10, 10, 0]]}\n        - edges:\n            - line: {from: [2, 2, 0], to: [4, 2, 0]}\n            - arc: {center: [3, 2, 0], radius: 1, start_angle: 0, end_angle: 180}\n";
    let mut record = file(text);
    record
        .move_by(ObjectList::Template, 0, [1.0, 0.0, 0.0], Unit::Mm, 0)
        .unwrap();
    let after = record.new_text().unwrap().unwrap();
    assert!(
        after.contains("[[1, 0, 0], [11, 0, 0], [11, 10, 0]]"),
        "{after}"
    );
    assert!(
        after.contains("{from: [3, 2, 0], to: [5, 2, 0]}"),
        "{after}"
    );
    assert!(after.contains("{center: [4, 2, 0], radius: 1"), "{after}");
}

#[test]
fn a_crlf_file_keeps_its_line_breaks() {
    let text = "units: mm\r\nobjects:\r\n  - line: {from: [0, 0, 0], to: [1, 0, 0]}\r\n";
    let mut record = file(text);
    let circle = keys(json!({"center": [0, 0, 0], "radius": 1, "id": "c"}));
    record
        .add(ObjectList::Template, "circle", &circle, 0)
        .unwrap();
    assert_eq!(
        record.new_text().unwrap().unwrap(),
        "units: mm\r\nobjects:\r\n  - line: {from: [0, 0, 0], to: [1, 0, 0]}\r\n  - circle: {center: [0, 0, 0], radius: 1, id: c}\r\n"
    );
}

#[test]
fn a_moved_coordinate_with_its_own_unit_is_written_in_the_declared_unit() {
    let text = "units: mm\nobjects:\n  - circle: {center: [1 in, 0, 0], radius: 1}\n";
    let mut record = file(text);
    record
        .move_by(ObjectList::Template, 0, [1.0, 2.0, 0.0], Unit::Mm, 0)
        .unwrap();
    assert!(record
        .new_text()
        .unwrap()
        .unwrap()
        .contains("{center: [26.4, 2, 0], radius: 1}"));
}
