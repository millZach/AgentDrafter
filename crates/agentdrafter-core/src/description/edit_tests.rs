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
        .set_attribute(ObjectList::Template, 2, "TITLE", None, "NEW", 0)
        .unwrap();
    assert_eq!(set, AttributeSet::FromRow("TITLE".to_owned()));
    assert_eq!(record.new_text().unwrap(), None);
    let set = record
        .set_attribute(ObjectList::Template, 2, "DRAWN", None, "ZM", 0)
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
            "contents, position, height, style, width_factor, rotation, justify, layer, color, linetype, lineweight, linetype_scale, name"
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

/// Two tables the file did not have, added together, go in before its
/// objects in the order they were made.
#[test]
fn two_new_tables_go_in_before_the_objects_in_order() {
    let mut record = RecordFile::from_text(
        std::path::Path::new("t.yaml"),
        "units: in
objects: []
"
        .to_owned(),
    )
    .unwrap();
    let keys = |json: serde_json::Value| json.as_object().unwrap().clone();
    record
        .table_add("layers", &["color"], "notes", &keys(serde_json::json!({})))
        .unwrap();
    record
        .table_add(
            "linetypes",
            &["pattern"],
            "DASHED",
            &keys(serde_json::json!({"pattern": [1, -1]})),
        )
        .unwrap();
    let text = record.new_text().unwrap().unwrap();
    assert_eq!(
        text,
        "units: in
layers:
  notes: {}
linetypes:
  DASHED: {pattern: [1, -1]}
objects: []
"
    );
}

/// An entry added to a table the file writes an entry to a line goes after
/// its last, and every line the file wrote stays as it was, its comments
/// and its block-style entries among them; a table written in braces, or
/// one an edit changes otherwise, is written again whole.
#[test]
fn an_entry_added_to_a_table_goes_after_its_last_and_keeps_its_comments() {
    let keys = |json: serde_json::Value| json.as_object().unwrap().clone();
    let text = "units: in
layers:
  # ours
  title: {color: red}   # red, as the client asks
  frame:
    color: 3
blocks: {}
";
    let mut record = RecordFile::from_text(Path::new("t.yaml"), text.to_owned()).unwrap();
    record
        .table_add("layers", &["color"], "notes", &keys(json!({"color": 2})))
        .unwrap();
    let written = record.new_text().unwrap().unwrap();
    assert_eq!(
        written,
        text.replace("blocks: {}", "  notes: {color: 2}\nblocks: {}")
    );

    let mut record = RecordFile::from_text(Path::new("t.yaml"), text.to_owned()).unwrap();
    record
        .table_add("layers", &["color"], "notes", &keys(json!({"color": 2})))
        .unwrap();
    record
        .table_change("layers", &["color"], "frame", &keys(json!({"color": 4})))
        .unwrap();
    let written = record.new_text().unwrap().unwrap();
    assert!(!written.contains("# red"), "written again whole: {written}");

    let flow = "units: in\nlayers: {title: {color: red}}\nblocks: {}\n";
    let mut record = RecordFile::from_text(Path::new("t.yaml"), flow.to_owned()).unwrap();
    record
        .table_add("layers", &["color"], "notes", &keys(json!({})))
        .unwrap();
    assert_eq!(
        record.new_text().unwrap().unwrap(),
        "units: in\nlayers:\n  title: {color: red}\n  notes: {}\nblocks: {}\n"
    );
}

/// A table written in braces is written again an entry to a line, so one
/// whose text holds a comment is not rewritten, and nothing is lost; a `#`
/// inside a quoted key or value is no comment.
#[test]
fn a_table_in_braces_with_a_comment_is_not_rewritten() {
    let keys = |json: serde_json::Value| json.as_object().unwrap().clone();
    for text in [
        "units: in
layers: {title: {color: red}}  # the client's
objects: []
",
        "units: in
layers: {
  # ours
  title: {color: red}
}
objects: []
",
        "units: in
layers: # ours
  {title: {color: red}}
objects: []
",
    ] {
        let mut record = RecordFile::from_text(Path::new("t.yaml"), text.to_owned()).unwrap();
        record
            .table_add("layers", &["color"], "notes", &keys(json!({})))
            .unwrap();
        let error = record.new_text().unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::WriteFailed, "{text}");
        assert!(
            error
                .message
                .contains("its layers are written in braces with a comment"),
            "{error:?}"
        );
        assert!(
            error
                .message
                .contains("write the table one entry to a line"),
            "{error:?}"
        );
    }
    let quoted = "units: in
layers: {'A #1': {color: red, description: 'see #2'}, \"B #3\": {}}
objects: []
";
    let mut record = RecordFile::from_text(Path::new("t.yaml"), quoted.to_owned()).unwrap();
    record
        .table_add("layers", &["color"], "notes", &keys(json!({})))
        .unwrap();
    let written = record.new_text().unwrap().unwrap();
    assert!(
        written.contains("'see #2'") && written.contains("  notes: {}"),
        "{written}"
    );
}

/// What counts as a comment is read from the file as it was: a `#` in a
/// quoted value an edit replaces is still no comment.
#[test]
fn a_hash_in_a_quoted_value_an_edit_replaces_is_no_comment() {
    let keys = |json: serde_json::Value| json.as_object().unwrap().clone();
    let text = "units: in
layers: {old: {description: 'see #2'}}
objects: []
";
    let mut record = RecordFile::from_text(Path::new("t.yaml"), text.to_owned()).unwrap();
    record
        .table_change(
            "layers",
            &["description"],
            "old",
            &keys(json!({"description": "new"})),
        )
        .unwrap();
    assert_eq!(
        record.new_text().unwrap().unwrap(),
        "units: in
layers:
  old: {description: new}
objects: []
"
    );
}

const LIBRARY: &str = include_str!("../../../../examples/demo-set/blocks.yaml");

/// A made-up block library with comments between its blocks, and a block
/// written in braces.
const COMMENTED: &str = "# head
units: in
blocks:
  # the first
  OUTER:
    objects:
      - line: {from: [0, 0, 0], to: [1, 0, 0]}
      - insert: {block: INNER, position: [0, 0, 0], attributes: {LETTER: A, NOTE: kept}}
  # the second
  INNER: {objects: [{attdef: {tag: LETTER, position: [0, 0, 0], height: 1}}, {attdef: {tag: NOTE, position: [0, 1, 0], height: 1}}]}
  # the third
  LAST:
    base_point: [1, 1, 0]
    objects:
      - circle: {center: [0, 0, 0], radius: 2}
# tail
";

#[test]
fn a_block_added_goes_after_the_last_and_keeps_every_other_byte() {
    let mut record = file(LIBRARY);
    let objects = [
        json!({"line": {"to": [10, 0, 0], "from": [0, 0, 0]}}),
        json!({"attdef": {"height": 2, "position": [0, 5, 0], "tag": "CODE"}}),
    ];
    record
        .block_set("BADGE", Some(&json!([1, 0, 0])), &objects)
        .unwrap();
    let after = record.new_text().unwrap().unwrap();
    assert_eq!(
        after,
        format!(
            "{LIBRARY}  BADGE:
    base_point: [1, 0, 0]
    objects:
      - line: {{from: [0, 0, 0], to: [10, 0, 0]}}
      - attdef: {{tag: CODE, position: [0, 5, 0], height: 2}}
"
        )
    );
    assert_eq!(record.block_names(), ["TITLE_BLOCK", "BADGE"]);
}

#[test]
fn a_block_redefined_rewrites_its_own_lines_alone() {
    let mut record = file(COMMENTED);
    record
        .block_set(
            "INNER",
            None,
            &[json!({"attdef": {"tag": "LETTER", "position": [0, 0, 0], "height": 1}})],
        )
        .unwrap();
    let after = record.new_text().unwrap().unwrap();
    assert_eq!(
        after,
        COMMENTED.replace(
            "  INNER: {objects: [{attdef: {tag: LETTER, position: [0, 0, 0], height: 1}}, {attdef: {tag: NOTE, position: [0, 1, 0], height: 1}}]}\n",
            "  INNER:
    objects:
      - attdef: {tag: LETTER, position: [0, 0, 0], height: 1}
"
        )
    );
    assert_eq!(record.block_carried_tags("INNER").unwrap(), ["LETTER"]);

    // The last block, before the file's last comment, and one added after
    // it, keep the comment where it was.
    let mut record = file(COMMENTED);
    record
        .block_set(
            "LAST",
            Some(&json!([0, 0, 0])),
            &[json!({"circle": {"center": [0, 0, 0], "radius": 3}})],
        )
        .unwrap();
    record
        .block_set(
            "ADDED",
            None,
            &[json!({"line": {"from": [0, 0, 0], "to": [1, 1, 0]}})],
        )
        .unwrap();
    let after = record.new_text().unwrap().unwrap();
    assert_eq!(
        after,
        COMMENTED.replace(
            "    base_point: [1, 1, 0]
    objects:
      - circle: {center: [0, 0, 0], radius: 2}
",
            "    base_point: [0, 0, 0]
    objects:
      - circle: {center: [0, 0, 0], radius: 3}
  ADDED:
    objects:
      - line: {from: [0, 0, 0], to: [1, 1, 0]}
"
        )
    );
}

#[test]
fn a_nested_inserts_value_is_removed_from_its_block_alone() {
    let mut record = file(COMMENTED);
    let inserts = record.block_inserts();
    assert_eq!(inserts.len(), 1);
    let (outer, insert) = &inserts[0];
    assert_eq!(outer, "OUTER");
    assert_eq!(insert.block, "INNER");
    assert_eq!(
        pairs(&insert.values),
        [
            ("LETTER".to_owned(), "A".to_owned()),
            ("NOTE".to_owned(), "kept".to_owned())
        ]
    );
    record.fit_block_attribute("OUTER", 1, "LETTER", 0).unwrap();
    let after = record.new_text().unwrap().unwrap();
    assert_eq!(
        after,
        COMMENTED.replace(
            "attributes: {LETTER: A, NOTE: kept}",
            "attributes: {NOTE: kept}"
        )
    );
    // The last value takes `attributes` with it.
    record.fit_block_attribute("OUTER", 1, "NOTE", 0).unwrap();
    let after = record.new_text().unwrap().unwrap();
    assert_eq!(
        after,
        COMMENTED.replace(", attributes: {LETTER: A, NOTE: kept}}", "}")
    );
}

#[test]
fn a_templates_attribute_value_is_removed_and_its_row_reference_with_it() {
    let mut record = file(TEMPLATE);
    let inserts = record.inserts(ObjectList::Template);
    assert_eq!(inserts.len(), 1);
    assert_eq!(inserts[0].index, 2);
    assert!(pairs(&inserts[0].values).contains(&("REV".to_owned(), "{row: REV}".to_owned())));
    assert!(pairs(&inserts[0].values).contains(&("DRAWN".to_owned(), "AD".to_owned())));
    record
        .fit_attribute(ObjectList::Template, 2, "REV", 0, 0)
        .unwrap();
    record
        .fit_attribute(ObjectList::Template, 2, "DRAWN", 0, 0)
        .unwrap();
    let after = record.new_text().unwrap().unwrap();
    let (new, gone) = diff(TEMPLATE, &after);
    assert!(new.is_empty(), "{new:?}");
    assert_eq!(gone, ["        REV: {row: REV}", "        DRAWN: AD"]);
}

/// A library whose blocks are written `{}` takes its first block, written
/// a key to a line in their place.
#[test]
fn the_first_block_of_a_library_written_empty_in_braces_is_added() {
    let text = "# a made-up library\nunits: in\nblocks: {}\n# tail\n";
    let mut record = file(text);
    record
        .block_set(
            "BADGE",
            None,
            &[json!({"circle": {"center": [0, 0, 0], "radius": 1}})],
        )
        .unwrap();
    assert_eq!(
        record.new_text().unwrap().unwrap(),
        "# a made-up library
units: in
blocks:
  BADGE:
    objects:
      - circle: {center: [0, 0, 0], radius: 1}
# tail
"
    );
}

/// Blocks written in braces that hold a comment are not rewritten, so the
/// comment is not lost: the add is refused and nothing is written.
#[test]
fn a_library_whose_empty_braces_hold_a_comment_is_not_rewritten() {
    let text = "units: in\nblocks: { # kept\n  }\n";
    let mut record = file(text);
    let added = record
        .block_set(
            "BADGE",
            None,
            &[json!({"circle": {"center": [0, 0, 0], "radius": 1}})],
        )
        .and_then(|()| record.new_text().map(|_| ()));
    let e = added.unwrap_err();
    assert_eq!(e.code, crate::error::ErrorCode::WriteFailed, "{e:?}");
    assert!(e.message.contains("written in braces"), "{}", e.message);
}

/// Each value an insert's record gives, as its key and its text.
fn pairs(values: &[super::GivenValue]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|v| (v.key.clone(), v.text.clone()))
        .collect()
}
