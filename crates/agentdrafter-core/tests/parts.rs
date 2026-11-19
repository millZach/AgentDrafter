//! A row's value inside other text (#102): a template's text or mtext
//! contents, or an insert's attribute value, written as a list of parts,
//! plain text and values from the row joined in order. Built, refused
//! where a row has no place for it, edited by matching the text to the
//! parts or by naming one row value, undone, told by `--values`, and found
//! by a query, on a made-up set.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, display_list_with, parse_sheet_description, query_with, read_dwg,
    undo, Answer, ChangeReport, ChangeRequest, DisplayListReport, DisplayListRequest, ErrorCode,
    Matching, Object, Query, QueryRequest, Reply, Selection, SetBuildRequest, Settings,
    UndoRequest,
};
use serde_json::{json, Value};

#[path = "support/snapshot.rs"]
mod snapshot;

const BLOCKS: &str = "units: mm
blocks:
  CARD:
    objects:
      - polyline: {points: [[0, 0, 0], [40, 0, 0], [40, 10, 0], [0, 10, 0]], closed: true}
      - attdef: {tag: TITLE, position: [2, 6, 0], height: 2}
      - attdef: {tag: NOTE, position: [2, 3, 0], height: 1}
      - attdef: {tag: NOTE, position: [2, 1, 0], height: 1}
";

/// A made-up card sheet: the title block's TITLE, the heading, the note
/// and the code joined from parts; the block's NOTE, which it defines
/// twice, a list of its values; and the area, a value from the row whole.
const TEMPLATE: &str = r"units: mm
block_library: ../blocks.yaml
objects:
  - insert: {block: CARD, position: [50, 50, 0], id: card, attributes: {TITLE: [{row: AREA}, ' PANEL ', {row: PANEL}], NOTE: [{row: AREA}, FIXED]}}
  - text: {contents: ['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}], position: [0, 40, 0], height: 3, id: heading}
  - mtext: {contents: ['SLOT ', {row: SLOT}, '\P{\LPANEL} ', {row: PANEL}], position: [0, 60, 0], height: 3, width: 50, id: note}
  - text: {contents: [{row: PANEL}, {row: SLOT}], position: [0, 80, 0], height: 3, id: code}
  - text: {contents: {row: AREA}, position: [0, 90, 0], height: 3, id: area}
";

const ROWS: &str = "rows:
  - sheet: S-1
    template: templates/t.yaml
    values: {AREA: AREA 1, PANEL: P1, SLOT: '3'}
  - sheet: S-2
    template: templates/t.yaml
    values: {AREA: AREA 2, PANEL: P2, SLOT: '7'}
";

/// The made-up set, with `rows` as its rows file, in a folder of its own
/// under Cargo's scratch folder in `target/`.
fn set_with(test: &str, rows: &str) -> PathBuf {
    let set = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-parts")
        .join(test);
    let _ = fs::remove_dir_all(&set);
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(set.join("blocks.yaml"), BLOCKS).unwrap();
    fs::write(set.join("templates/t.yaml"), TEMPLATE).unwrap();
    fs::write(set.join("rows.yaml"), rows).unwrap();
    set
}

/// The made-up set, built.
fn built_set(test: &str) -> PathBuf {
    let set = set_with(test, ROWS);
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built);
    set
}

fn apply(set: &Path, operations: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set: json!({ "operations": operations }),
            dry_run: false,
        },
        &Settings::default(),
    )
}

/// The object a sheet's DWG holds at this ID.
fn in_dwg(set: &Path, sheet: &str, id: &str) -> Object {
    let read = read_dwg(&set.join("dwg").join(format!("{sheet}.dwg"))).unwrap();
    read.ids_and_names
        .into_iter()
        .zip(read.objects)
        .find(|(i, _)| i.id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no {id} in {sheet}"))
        .1
}

fn contents(object: &Object) -> &str {
    match object {
        Object::Text(text) => &text.contents,
        Object::MText(text) => &text.contents,
        other => panic!("not a text: {other:?}"),
    }
}

fn attributes(object: &Object) -> Vec<(&str, &str)> {
    let Object::Insert(insert) = object else {
        panic!("not an insert: {object:?}")
    };
    insert
        .attributes
        .iter()
        .map(|a| (a.tag.as_str(), a.value.as_str()))
        .collect()
}

#[test]
fn a_list_of_parts_builds_joined_where_the_template_puts_it() {
    let set = built_set("build");
    assert_eq!(
        contents(&in_dwg(&set, "S-1", "S-1/heading")),
        "PANEL P1 SLOT 3"
    );
    // Nothing inside a string is read for values: mtext's codes stay.
    assert_eq!(
        contents(&in_dwg(&set, "S-1", "S-1/note")),
        "SLOT 3\\P{\\LPANEL} P1"
    );
    assert_eq!(contents(&in_dwg(&set, "S-2", "S-2/code")), "P27");
    // TITLE, which the block defines once, joins its parts; NOTE, which it
    // defines twice, takes its list as its two values.
    assert_eq!(
        attributes(&in_dwg(&set, "S-2", "S-2/card")),
        [
            ("TITLE", "AREA 2 PANEL P2"),
            ("NOTE", "AREA 2"),
            ("NOTE", "FIXED")
        ]
    );
}

#[test]
fn a_row_lacking_a_part_is_missing_a_value() {
    let rows = ROWS.replace("PANEL: P2, SLOT: '7'", "PANEL: P2");
    let set = set_with("missing", &rows);
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    let report = built.result.unwrap();
    assert_eq!((report.built, report.failed), (1, 1));
    let e = report.sheets[1].error.as_ref().unwrap();
    assert_eq!(e.code, ErrorCode::MissingValue);
    assert_eq!(e.field.as_deref(), Some("rows[1].values.SLOT"));
    for place in [
        "objects[1].text.contents",
        "objects[2].mtext.contents",
        "objects[3].text.contents",
    ] {
        assert!(e.message.contains(place), "{place}: {}", e.message);
    }
}

#[test]
fn a_list_of_parts_with_a_value_from_the_row_is_refused_in_a_rows_object_and_a_sheet_on_its_own() {
    let rows = ROWS.replace(
        "    values: {AREA: AREA 1, PANEL: P1, SLOT: '3'}\n",
        "    values: {AREA: AREA 1, PANEL: P1, SLOT: '3'}\n    objects:\n      - text: {contents: ['A ', {row: PANEL}], position: [0, 0, 0], height: 3}\n",
    );
    let set = set_with("refused", &rows);
    let built = build_set_with(
        &SetBuildRequest {
            set,
            overwrite: false,
        },
        &Settings::default(),
    );
    let e = built.result.unwrap().sheets[0].error.clone().unwrap();
    assert_eq!(e.code, ErrorCode::WrongType);
    assert_eq!(
        e.field.as_deref(),
        Some("rows[0].objects[0].text.contents[1]")
    );
    assert!(e.message.contains("belongs in a template"), "{}", e.message);

    let e = parse_sheet_description(
        "units: mm\nobjects:\n  - text: {contents: ['A ', {row: PANEL}], position: [0, 0, 0], height: 3}\n",
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::WrongType);
    assert_eq!(e.field.as_deref(), Some("objects[0].text.contents[1]"));
    assert!(
        e.message.contains("build the set's folder"),
        "{}",
        e.message
    );
}

#[test]
fn an_edit_matched_to_the_parts_sets_each_row_value_on_its_sheet_alone() {
    let set = built_set("matched");
    let template = fs::read(set.join("templates/t.yaml")).unwrap();
    let other = fs::read(set.join("dwg/S-2.dwg")).unwrap();
    let reply = apply(
        &set,
        json!([{"op": "change", "target": {"id": "S-1/heading"}, "set": {"contents": "PANEL P9 SLOT 5"}}]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let report = reply.result.unwrap();
    let op = serde_json::to_value(&report.operations[0]).unwrap();
    assert_eq!(op["sheets"], json!(["S-1"]));
    assert_eq!(op.get("row_value"), None);
    let fills = |ids: &[(&str, &str)]| -> Value {
        ids.iter()
            .map(|(id, at)| json!({"id": format!("S-1/{id}"), "at": at}))
            .collect()
    };
    assert_eq!(
        op["row_values"],
        json!([
            {"sheet": "S-1", "value": "PANEL", "was": "P1", "now": "P9", "fills": fills(&[("card", "attributes.TITLE"), ("heading", "contents"), ("note", "contents"), ("code", "contents")])},
            {"sheet": "S-1", "value": "SLOT", "was": "3", "now": "5", "fills": fills(&[("heading", "contents"), ("note", "contents"), ("code", "contents")])}
        ])
    );
    assert!(
        op["message"]
            .as_str()
            .unwrap()
            .contains("matched to the parts ['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}]"),
        "{}",
        op["message"]
    );
    // The row's values are set, the template is not, and every place each
    // fills on the sheet follows; the other sheet is as it was.
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    assert!(
        rows.contains("{AREA: AREA 1, PANEL: P9, SLOT: 5}"),
        "{rows}"
    );
    assert_eq!(fs::read(set.join("templates/t.yaml")).unwrap(), template);
    assert_eq!(contents(&in_dwg(&set, "S-1", "S-1/code")), "P95");
    assert_eq!(
        attributes(&in_dwg(&set, "S-1", "S-1/card"))[0],
        ("TITLE", "AREA 1 PANEL P9")
    );
    assert_eq!(fs::read(set.join("dwg/S-2.dwg")).unwrap(), other);

    // An attribute's value is matched the same way: AREA fills the title,
    // the first NOTE and the area.
    let reply = apply(
        &set,
        json!([{"op": "set_attribute", "target": {"id": "S-1/card"}, "tag": "TITLE", "value": "AREA 9 PANEL P8"}]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let op = serde_json::to_value(&reply.result.unwrap().operations[0]).unwrap();
    assert_eq!(op["row_values"][0]["value"], "AREA");
    assert_eq!(
        op["row_values"][0]["fills"],
        fills(&[
            ("card", "attributes.TITLE"),
            ("card", "attributes.NOTE[0]"),
            ("area", "contents")
        ])
    );
    assert_eq!(op["row_values"][1]["now"], "P8");
    assert_eq!(
        attributes(&in_dwg(&set, "S-1", "S-1/card")),
        [
            ("TITLE", "AREA 9 PANEL P8"),
            ("NOTE", "AREA 9"),
            ("NOTE", "FIXED")
        ]
    );
}

#[test]
fn an_edit_whose_fixed_text_differs_or_that_splits_two_ways_is_refused_naming_the_pattern() {
    let set = built_set("refusals");
    let before = snapshot::snapshot(&set);
    for (target, text, says) in [
        (
            "S-1/heading",
            "BOARD P9 SLOT 5",
            "does not match the parts ['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}]",
        ),
        (
            "S-1/code",
            "P15",
            "splits more than one way into the parts [{row: PANEL}, {row: SLOT}]",
        ),
    ] {
        let reply = apply(
            &set,
            json!([{"op": "change", "target": {"id": target}, "set": {"contents": text}}]),
        );
        assert!(!reply.ok, "{text}");
        let report = reply.result.unwrap();
        assert_eq!((report.landed, report.failed), (0, 1));
        let e = report.operations[0].error.as_ref().unwrap();
        assert_eq!(e.code, ErrorCode::BadValue);
        assert_eq!(e.field.as_deref(), Some("operations[0].set.contents"));
        assert!(e.message.contains(says), "{}", e.message);
        assert!(e.message.contains("row_value"), "{}", e.message);
    }
    assert_eq!(
        snapshot::snapshot(&set),
        before,
        "a refusal writes nothing but the journal"
    );
}

#[test]
fn row_value_sets_one_part_alone() {
    let set = built_set("row-value");
    let reply = apply(
        &set,
        json!([{"op": "change", "target": {"id": "S-1/code"}, "set": {"contents": "P4"}, "row_value": "PANEL"}]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let op = serde_json::to_value(&reply.result.unwrap().operations[0]).unwrap();
    assert_eq!(op["row_value"]["value"], "PANEL");
    assert_eq!(
        (&op["row_value"]["was"], &op["row_value"]["now"]),
        (&json!("P1"), &json!("P4"))
    );
    assert_eq!(op["row_value"]["fills"].as_array().unwrap().len(), 4);
    assert_eq!(op.get("row_values"), None);
    assert_eq!(contents(&in_dwg(&set, "S-1", "S-1/code")), "P43");

    // A name the parts do not hold is refused, naming those they do.
    let reply = apply(
        &set,
        json!([{"op": "change", "target": {"id": "S-1/code"}, "set": {"contents": "X"}, "row_value": "AREA"}]),
    );
    let e = reply.result.unwrap().operations[0].error.clone().unwrap();
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("operations[0].row_value"));
    assert!(e.message.contains("they are PANEL, SLOT"), "{}", e.message);
}

#[test]
fn undo_puts_a_matched_edit_back_byte_for_byte() {
    let set = built_set("undo");
    let before = snapshot::snapshot(&set);
    let reply = apply(
        &set,
        json!([{"op": "set_attribute", "target": {"id": "S-1/card"}, "tag": "TITLE", "value": "AREA 5 PANEL P5"}]),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let entry = reply.result.unwrap().journal_entry.unwrap();
    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: entry,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(snapshot::snapshot(&set), before);
}

/// An object's `values`, as JSON, by its ID.
fn values_of(report: &DisplayListReport, id: &str) -> Value {
    let object = report
        .objects
        .iter()
        .find(|o| o.id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no object {id}"));
    serde_json::to_value(object.values.as_ref()).unwrap()
}

#[test]
fn with_values_a_value_joined_from_parts_names_each_row_value_and_its_places() {
    let set = built_set("values");
    let mut request = DisplayListRequest::new(&set);
    request.values = true;
    let told = display_list_with(&request, &Settings::default()).unwrap();
    assert_eq!(
        values_of(&told, "S-1/heading"),
        json!({"contents": {"value": "PANEL P1 SLOT 3", "written": "row_values", "row_values": [{"name": "PANEL", "fills": 4}, {"name": "SLOT", "fills": 3}]}})
    );
    assert_eq!(
        values_of(&told, "S-1/card"),
        json!({"attributes": [
            {"tag": "TITLE", "value": "AREA 1 PANEL P1", "written": "row_values", "row_values": [{"name": "AREA", "fills": 3}, {"name": "PANEL", "fills": 4}]},
            {"tag": "NOTE", "occurrence": 1, "value": "AREA 1", "written": "row_value", "row_value": "AREA", "fills": 3},
            {"tag": "NOTE", "occurrence": 2, "value": "FIXED", "written": "own"}
        ]})
    );
    // A whole value from the row is told as before.
    assert_eq!(
        values_of(&told, "S-1/area"),
        json!({"contents": {"value": "AREA 1", "written": "row_value", "row_value": "AREA", "fills": 3}})
    );
}

#[test]
fn a_query_finds_the_joined_text() {
    let set = built_set("query");
    let find = |selection: Selection| {
        let report = query_with(
            &QueryRequest {
                source: set.clone(),
                sheet: None,
                query: Query::Find(selection),
            },
            &Settings::default(),
        )
        .unwrap();
        let Answer::Find { objects, .. } = report.answer else {
            panic!("a find's answer")
        };
        objects.into_iter().map(|o| o.id).collect::<Vec<_>>()
    };
    assert_eq!(
        find(Selection {
            text: Some("PANEL P2 SLOT 7".to_owned()),
            matching: Matching::Exact,
            ..Selection::default()
        }),
        ["S-2/heading"]
    );
    assert_eq!(
        find(Selection {
            tag: Some("TITLE".to_owned()),
            value: Some("AREA 1 PANEL P1".to_owned()),
            matching: Matching::Exact,
            ..Selection::default()
        }),
        ["S-1/card"]
    );
}
