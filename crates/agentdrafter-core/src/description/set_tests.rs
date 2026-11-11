//! A drawing set's settings, its rows, and templates filled in from a row;
//! and the schemas of all three, which accept exactly what the readers
//! accept.

use std::path::Path;

use serde_json::{json, Value};

use super::set::{parse_rows, parse_set_settings, Row, ROWS_FILE_KEYS, ROW_KEYS, SETTINGS_KEYS};
use super::set_schema::{rows_schema, set_schema, template_schema};
use super::template::{fill, names};
use super::{parse_block_library, parse_sheet_description};
use crate::error::{Error, ErrorCode};
use crate::model::{Object, Sheet};
use crate::units::Unit;

/// A made-up block library with a title block of three attributes.
const LIBRARY: &str = "units: mm
blocks:
  TB:
    objects:
      - polyline: {points: [[0, 0, 0], [100, 0, 0], [100, 20, 0], [0, 20, 0]], closed: true}
      - attdef: {tag: TITLE, position: [5, 12, 0], height: 3}
      - attdef: {tag: DWG_NO, position: [5, 4, 0], height: 2.5}
      - attdef: {tag: REV, default: '0', position: [80, 4, 0], height: 2.5}
";

/// A made-up template: a title block and a heading from the row.
const TEMPLATE: &str = "units: mm
block_library: blocks.yaml
objects:
  - polyline: {points: [[0, 0, 0], [420, 0, 0], [420, 297, 0], [0, 297, 0]], closed: true}
  - insert:
      block: TB
      position: [310, 10, 0]
      attributes: {TITLE: {row: TITLE}, DWG_NO: {row: DWG_NO}, REV: B}
  - text: {contents: {row: TITLE}, position: [10, 280, 0], height: 5}
";

fn rows(text: &str) -> Vec<Row> {
    parse_rows(text).unwrap_or_else(|e| panic!("{e}"))
}

fn one_row(values: &str) -> Row {
    let text = format!("rows:\n  - sheet: S-1\n    template: t.yaml\n    values: {values}\n");
    rows(&text).remove(0)
}

/// Fills `template` in from `row` as a sheet of a set in millimetres, with
/// [`LIBRARY`] as its block library.
fn fill_in(template: &str, row: &Row) -> Result<Sheet, Error> {
    fill(template, row, Path::new("rows.yaml"), Unit::Mm, &mut |_| {
        parse_block_library(LIBRARY)
    })
}

#[track_caller]
fn error(outcome: Result<impl std::fmt::Debug, Error>) -> Error {
    match outcome {
        Ok(read) => panic!("expected an error, read {read:?}"),
        Err(error) => error,
    }
}

#[test]
fn settings_declare_the_unit_the_output_folder_and_the_rows_file() {
    let settings =
        parse_set_settings("units: in\noutput: ../issued/dwg\nrows: data/rows.yaml\n").unwrap();
    assert_eq!(settings.units, Unit::In);
    assert_eq!(settings.output, "../issued/dwg");
    assert_eq!(settings.rows, "data/rows.yaml");
}

#[test]
fn settings_that_lack_a_key_or_hold_a_wrong_one_are_errors_naming_the_field() {
    let cases: [(&str, ErrorCode, &str, &str); 6] = [
        (
            "output: dwg\nrows: rows.yaml\n",
            ErrorCode::MissingUnit,
            "units",
            "mm, cm, m, in, ft",
        ),
        (
            "units: inch\noutput: dwg\nrows: rows.yaml\n",
            ErrorCode::UnknownUnit,
            "units",
            "mm, cm, m, in, ft",
        ),
        (
            "units: mm\nrows: rows.yaml\n",
            ErrorCode::MissingKey,
            "output",
            "`output`",
        ),
        (
            "units: mm\noutput: /dwg\nrows: rows.yaml\n",
            ErrorCode::BadValue,
            "output",
            "relative to the drawing set's folder",
        ),
        (
            "units: mm\noutput: dwg\nrows: 'c:rows.yaml'\n",
            ErrorCode::BadValue,
            "rows",
            "cannot hold `:`",
        ),
        (
            "units: mm\noutput: dwg\nrows: rows.yaml\npaper: A3\n",
            ErrorCode::UnknownKey,
            "paper",
            "units, output, rows",
        ),
    ];
    for (text, code, field, says) in cases {
        let e = error(parse_set_settings(text));
        assert_eq!(e.code, code, "{text}: {e:?}");
        assert_eq!(e.field.as_deref(), Some(field), "{text}: {e:?}");
        assert!(e.message.contains(says), "{text}: {e:?}");
    }
}

#[test]
fn rows_name_their_sheet_their_template_and_their_values() {
    let rows = rows(
        "rows:\n  - sheet: DEMO-001\n    template: templates/a.yaml\n    values: {TITLE: FIRST, dwg-no: '7', EMPTY: ''}\n  - {sheet: DEMO-002, template: b.yaml}\n",
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].sheet.as_str(), rows[0].template.as_str()),
        ("DEMO-001", "templates/a.yaml")
    );
    let values: Vec<(&str, &str)> = rows[0]
        .values
        .iter()
        .map(|v| (v.name.as_str(), v.text.as_str()))
        .collect();
    assert_eq!(values, [("TITLE", "FIRST"), ("dwg-no", "7"), ("EMPTY", "")]);
    assert!(rows[1].values.is_empty());
    assert!(parse_rows("rows: []\n").unwrap().is_empty());
}

#[test]
fn a_rows_file_with_a_wrong_row_is_an_error_naming_the_field() {
    let cases: [(&str, ErrorCode, &str, &str); 8] = [
        (
            "rows:\n  - {sheet: A, template: t.yaml}\n  - {sheet: a, template: t.yaml}\n",
            ErrorCode::DuplicateName,
            "rows[1].sheet",
            "line 2",
        ),
        (
            "rows:\n  - {sheet: 'A/B', template: t.yaml}\n",
            ErrorCode::BadValue,
            "rows[0].sheet",
            "cannot hold `/`",
        ),
        (
            "rows:\n  - {sheet: Con.1, template: t.yaml}\n",
            ErrorCode::BadValue,
            "rows[0].sheet",
            "CON, PRN, AUX",
        ),
        (
            "rows:\n  - {sheet: 'A.', template: t.yaml}\n",
            ErrorCode::BadValue,
            "rows[0].sheet",
            "cannot end with `.`",
        ),
        (
            "rows:\n  - {sheet: A}\n",
            ErrorCode::MissingKey,
            "rows[0].template",
            "`template`",
        ),
        (
            "rows:\n  - {sheet: A, template: t.yaml, values: {'DWG NO': 1}}\n",
            ErrorCode::BadValue,
            "rows[0].values.DWG NO",
            "ASCII letters, digits",
        ),
        (
            "rows:\n  - {sheet: A, template: t.yaml, values: {TITLE: [A, B]}}\n",
            ErrorCode::WrongType,
            "rows[0].values.TITLE",
            "text",
        ),
        (
            "rows:\n  - {sheet: A, template: t.yaml, layout: 1}\n",
            ErrorCode::UnknownKey,
            "rows[0].layout",
            "sheet, template, values",
        ),
    ];
    for (text, code, field, says) in cases {
        let e = error(parse_rows(text));
        assert_eq!(e.code, code, "{text}: {e:?}");
        assert_eq!(e.field.as_deref(), Some(field), "{text}: {e:?}");
        assert!(e.message.contains(says), "{text}: {e:?}");
    }
}

#[test]
fn a_template_takes_its_values_from_the_row() {
    let row = one_row("{TITLE: GENERAL ARRANGEMENT, DWG_NO: DEMO-001}");
    let sheet = fill_in(TEMPLATE, &row).unwrap_or_else(|e| panic!("{e}"));
    let Object::Insert(insert) = &sheet.objects[1] else {
        panic!("an insert, read {:?}", sheet.objects[1]);
    };
    let values: Vec<(&str, &str)> = insert
        .attributes
        .iter()
        .map(|a| (a.tag.as_str(), a.value.as_str()))
        .collect();
    assert_eq!(
        values,
        [
            ("TITLE", "GENERAL ARRANGEMENT"),
            ("DWG_NO", "DEMO-001"),
            ("REV", "B")
        ]
    );
    let Object::Text(text) = &sheet.objects[2] else {
        panic!("a text, read {:?}", sheet.objects[2]);
    };
    assert_eq!(text.contents, "GENERAL ARRANGEMENT");
    assert_eq!(sheet.blocks.len(), 1);
    assert_eq!(names(TEMPLATE), ["TITLE", "DWG_NO"]);
}

#[test]
fn a_row_that_lacks_a_value_is_an_error_naming_the_sheet_and_the_value() {
    let row = one_row("{DWG_NO: DEMO-001}");
    let e = error(fill_in(TEMPLATE, &row));
    assert_eq!(e.code, ErrorCode::MissingValue);
    assert_eq!(e.field.as_deref(), Some("rows[0].values.TITLE"));
    assert_eq!(e.file.as_deref(), Some("rows.yaml"));
    assert_eq!(e.line, Some(4));
    for says in [
        "sheet `S-1`",
        "no value `TITLE`",
        "template `t.yaml`",
        "`objects[1].insert.attributes.TITLE`",
        "`objects[2].text.contents`",
    ] {
        assert!(e.message.contains(says), "{says}: {e:?}");
    }

    let e = error(fill_in(TEMPLATE, &one_row("{}")));
    assert_eq!(e.code, ErrorCode::MissingValue);
    assert_eq!(e.field.as_deref(), Some("rows[0].values.TITLE"));
    assert!(e.message.contains("no values `TITLE`, `DWG_NO`"), "{e:?}");
}

#[test]
fn a_value_the_template_has_no_place_for_is_an_error_listing_the_ones_it_takes() {
    let row = one_row("{TITLE: A, DWG_NO: B, DATE: '2026-11-11'}");
    let e = error(fill_in(TEMPLATE, &row));
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("rows[0].values.DATE"));
    assert_eq!(e.file.as_deref(), Some("rows.yaml"));
    assert!(e.message.contains("sheet `S-1`"), "{e:?}");
    assert!(e.message.contains("it takes TITLE, DWG_NO"), "{e:?}");
}

#[test]
fn a_value_refused_where_the_template_puts_it_is_an_error_at_the_value() {
    // A text's contents cannot be empty; an attribute's value cannot hold a line break.
    let e = error(fill_in(TEMPLATE, &one_row("{TITLE: '', DWG_NO: B}")));
    assert_eq!(e.code, ErrorCode::BadValue);
    assert_eq!(e.field.as_deref(), Some("rows[0].values.TITLE"));
    assert_eq!(e.file.as_deref(), Some("rows.yaml"));
    assert_eq!((e.line, e.column), (Some(4), Some(21)));
    assert!(e.message.contains("sheet `S-1`"), "{e:?}");
    assert!(e.message.contains("`objects[2].text.contents`"), "{e:?}");
    assert!(e.message.contains("text needs contents"), "{e:?}");

    let e = error(fill_in(TEMPLATE, &one_row("{TITLE: A, DWG_NO: \"B\\nC\"}")));
    assert_eq!(e.field.as_deref(), Some("rows[0].values.DWG_NO"));
    assert!(e.message.contains("one line"), "{e:?}");
}

#[test]
fn an_error_in_the_template_stays_in_the_template() {
    // The tag is the template's mistake, not the row's, though its value comes from the row.
    let template = TEMPLATE.replace("DWG_NO: {row: DWG_NO}", "DWG: {row: DWG_NO}");
    let e = error(fill_in(&template, &one_row("{TITLE: A, DWG_NO: B}")));
    assert_eq!(e.code, ErrorCode::UnknownName);
    assert_eq!(e.field.as_deref(), Some("objects[1].insert.attributes.DWG"));
    assert_eq!(e.file, None, "placed in the template's file by the caller");
}

#[test]
fn a_template_in_another_unit_than_the_set_is_an_error_at_its_units() {
    let template = TEMPLATE.replace("units: mm", "units: in");
    let e = error(fill_in(&template, &one_row("{TITLE: A, DWG_NO: B}")));
    assert_eq!(e.code, ErrorCode::BadValue);
    assert_eq!(e.field.as_deref(), Some("units"));
    assert!(
        e.message
            .contains("the template is in in and the drawing set in mm"),
        "{e:?}"
    );
}

#[test]
fn a_value_from_the_row_is_written_one_way() {
    let cases: [(&str, ErrorCode, &str); 4] = [
        (
            "{row: TITLE, default: A}",
            ErrorCode::UnknownKey,
            "takes row",
        ),
        ("{}", ErrorCode::MissingKey, "`row`"),
        ("{row: 'DWG NO'}", ErrorCode::BadValue, "ASCII letters"),
        (
            "{row: [A]}",
            ErrorCode::WrongType,
            "the name of one of the row's values",
        ),
    ];
    for (written, code, says) in cases {
        let template =
            format!("units: mm\nobjects:\n  - text: {{contents: {written}, position: [0, 0, 0], height: 1}}\n");
        let e = error(fill_in(&template, &one_row("{TITLE: A}")));
        assert_eq!(e.code, code, "{written}: {e:?}");
        assert!(
            e.field
                .as_deref()
                .is_some_and(|f| f.starts_with("objects[0].text.contents")),
            "{written}: {e:?}"
        );
        assert!(e.message.contains(says), "{written}: {e:?}");
    }
}

#[test]
fn a_sheet_description_built_on_its_own_takes_no_value_from_a_row() {
    let e = error(parse_sheet_description(
        "units: mm\nobjects:\n  - text: {contents: {row: TITLE}, position: [0, 0, 0], height: 1}\n",
    ));
    assert_eq!(e.code, ErrorCode::WrongType);
    assert_eq!(e.field.as_deref(), Some("objects[0].text.contents"));
    assert!(e.message.contains("drawing set"), "{e:?}");
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn sorted(keys: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
    keys.sort();
    keys
}

#[test]
fn the_schemas_and_the_readers_know_the_same_keys() {
    assert_eq!(keys(&set_schema()["properties"]), sorted(&SETTINGS_KEYS));
    let rows = rows_schema();
    assert_eq!(keys(&rows["properties"]), sorted(&ROWS_FILE_KEYS));
    assert_eq!(keys(&rows["$defs"]["row"]["properties"]), sorted(&ROW_KEYS));
    let template = template_schema();
    assert_eq!(template["title"], "AgentDrafter template");
    assert!(template["$defs"]["row_value"].is_object());
}

/// Where a value goes: the YAML a reader reads and the same document as
/// JSON, each with `V` where the value belongs.
struct Place {
    name: &'static str,
    yaml: &'static str,
    json: &'static str,
}

/// `(place, value as YAML, the same value as JSON, accepted)`.
type Case = (&'static Place, &'static str, Value, bool);

/// Asserts that `read` and `schema` both accept exactly the cases marked
/// accepted, listing every case where either disagrees.
fn assert_reader_and_schema_agree(
    schema: &Value,
    cases: Vec<Case>,
    read: impl Fn(&str) -> Result<(), Error>,
) {
    let validator = jsonschema::validator_for(schema).expect("the schema is a valid JSON Schema");
    let mut disagreements = Vec::new();
    for (place, yaml_value, json_value, accepted) in cases {
        let yaml = place.yaml.replace('V', yaml_value);
        let json: Value = serde_json::from_str(&place.json.replace('V', &json_value.to_string()))
            .expect("the JSON case is JSON");
        let reader = read(&yaml);
        let by_schema = validator.is_valid(&json);
        if reader.is_ok() != accepted || by_schema != accepted {
            disagreements.push(format!(
                "{} {yaml_value}: expected {}, the reader {} ({}), the schema {}",
                place.name,
                if accepted { "accepted" } else { "refused" },
                if reader.is_ok() { "accepts" } else { "refuses" },
                reader.err().map(|e| e.message).unwrap_or_default(),
                if by_schema { "accepts" } else { "refuses" },
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "{} disagreements:\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

const UNITS: Place = Place {
    name: "the set's units",
    yaml: "units: V\noutput: dwg\nrows: rows.yaml\n",
    json: r#"{"units": V, "output": "dwg", "rows": "rows.yaml"}"#,
};

const OUTPUT: Place = Place {
    name: "the set's output",
    yaml: "units: mm\noutput: V\nrows: rows.yaml\n",
    json: r#"{"units": "mm", "output": V, "rows": "rows.yaml"}"#,
};

const ROWS: Place = Place {
    name: "the set's rows",
    yaml: "units: mm\noutput: dwg\nrows: V\n",
    json: r#"{"units": "mm", "output": "dwg", "rows": V}"#,
};

#[test]
fn the_set_schema_accepts_exactly_the_settings_the_reader_accepts() {
    let cases: Vec<Case> = vec![
        (&UNITS, "in", json!("in"), true),
        (&UNITS, "'mm'", json!("mm"), true),
        (&UNITS, "MM", json!("MM"), false),
        (&UNITS, "''", json!(""), false),
        (&OUTPUT, "dwg", json!("dwg"), true),
        (&OUTPUT, "../issued/dwg", json!("../issued/dwg"), true),
        (&OUTPUT, ".", json!("."), true),
        (&OUTPUT, "2026", json!(2026), true),
        (&OUTPUT, "/dwg", json!("/dwg"), false),
        (&OUTPUT, "'a//b'", json!("a//b"), false),
        (&OUTPUT, "'a\\b'", json!("a\\b"), false),
        (&OUTPUT, "dwg/", json!("dwg/"), false),
        (&OUTPUT, "''", json!(""), false),
        (&OUTPUT, "[dwg]", json!(["dwg"]), false),
        (&ROWS, "rows.yaml", json!("rows.yaml"), true),
        (&ROWS, "data/rows.yaml", json!("data/rows.yaml"), true),
        (&ROWS, "'c:rows.yaml'", json!("c:rows.yaml"), false),
    ];
    assert_reader_and_schema_agree(&set_schema(), cases, |yaml| {
        parse_set_settings(yaml).map(|_| ())
    });
}

const SHEET: Place = Place {
    name: "a row's sheet",
    yaml: "rows:\n  - {sheet: V, template: t.yaml}\n",
    json: r#"{"rows": [{"sheet": V, "template": "t.yaml"}]}"#,
};

const TEMPLATE_PATH: Place = Place {
    name: "a row's template",
    yaml: "rows:\n  - {sheet: A, template: V}\n",
    json: r#"{"rows": [{"sheet": "A", "template": V}]}"#,
};

const VALUES: Place = Place {
    name: "a row's values",
    yaml: "rows:\n  - {sheet: A, template: t.yaml, values: V}\n",
    json: r#"{"rows": [{"sheet": "A", "template": "t.yaml", "values": V}]}"#,
};

const ROW_LIST: Place = Place {
    name: "the rows",
    yaml: "rows: V\n",
    json: r#"{"rows": V}"#,
};

#[test]
fn the_rows_schema_accepts_exactly_the_rows_the_reader_accepts() {
    let longest = "A".repeat(251);
    let too_long = "A".repeat(252);
    let longest: &'static str = Box::leak(longest.into_boxed_str());
    let too_long: &'static str = Box::leak(too_long.into_boxed_str());
    let cases: Vec<Case> = vec![
        (&SHEET, "DEMO-001", json!("DEMO-001"), true),
        (&SHEET, "'Sheet 1 of 3'", json!("Sheet 1 of 3"), true),
        (&SHEET, "101", json!(101), true),
        (&SHEET, "true", json!(true), true),
        (&SHEET, "CONSOLE", json!("CONSOLE"), true),
        (&SHEET, "COM10", json!("COM10"), true),
        (&SHEET, "'.hidden'", json!(".hidden"), true),
        (&SHEET, longest, json!(longest), true),
        (&SHEET, too_long, json!(too_long), false),
        (&SHEET, "''", json!(""), false),
        (&SHEET, "' A'", json!(" A"), false),
        (&SHEET, "'A '", json!("A "), false),
        (&SHEET, "'A.'", json!("A."), false),
        (&SHEET, "'.'", json!("."), false),
        (&SHEET, "'A/B'", json!("A/B"), false),
        (&SHEET, "'A:B'", json!("A:B"), false),
        (&SHEET, "'A?'", json!("A?"), false),
        (&SHEET, "\"A\\tB\"", json!("A\tB"), false),
        (&SHEET, "NUL", json!("NUL"), false),
        (&SHEET, "con", json!("con"), false),
        (&SHEET, "lpt9.a", json!("lpt9.a"), false),
        (&SHEET, "[A]", json!(["A"]), false),
        (
            &TEMPLATE_PATH,
            "templates/a.yaml",
            json!("templates/a.yaml"),
            true,
        ),
        (&TEMPLATE_PATH, "../t.yaml", json!("../t.yaml"), true),
        (&TEMPLATE_PATH, "/t.yaml", json!("/t.yaml"), false),
        (&TEMPLATE_PATH, "''", json!(""), false),
        (
            &VALUES,
            "{TITLE: A, dwg-no: '1', REV_2: ''}",
            json!({"TITLE": "A", "dwg-no": "1", "REV_2": ""}),
            true,
        ),
        (
            &VALUES,
            "{N: 5, B: true, E: ~}",
            json!({"N": 5, "B": true, "E": null}),
            true,
        ),
        (&VALUES, "{}", json!({}), true),
        (&VALUES, "{'DWG NO': A}", json!({"DWG NO": "A"}), false),
        (&VALUES, "{'DWG.NO': A}", json!({"DWG.NO": "A"}), false),
        (&VALUES, "{ÉTAT: A}", json!({"ÉTAT": "A"}), false),
        (&VALUES, "{T: [A]}", json!({"T": ["A"]}), false),
        (&VALUES, "{T: {A: B}}", json!({"T": {"A": "B"}}), false),
        (&VALUES, "[A]", json!(["A"]), false),
        (&ROW_LIST, "[]", json!([]), true),
        (&ROW_LIST, "[{sheet: A}]", json!([{"sheet": "A"}]), false),
        (
            &ROW_LIST,
            "[{template: t.yaml}]",
            json!([{"template": "t.yaml"}]),
            false,
        ),
        (&ROW_LIST, "A", json!("A"), false),
    ];
    assert_reader_and_schema_agree(&rows_schema(), cases, |yaml| parse_rows(yaml).map(|_| ()));
}

const ATTRIBUTE_VALUE: Place = Place {
    name: "an insert's attribute value in a template",
    yaml: "units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert: {block: TB, position: [0, 0, 0], attributes: {TITLE: V}}\n",
    json: r#"{"units": "mm", "block_library": "blocks.yaml", "objects": [{"insert": {"block": "TB", "position": [0, 0, 0], "attributes": {"TITLE": V}}}]}"#,
};

const TEXT_CONTENTS: Place = Place {
    name: "a text's contents in a template",
    yaml: "units: mm\nobjects:\n  - text: {contents: V, position: [0, 0, 0], height: 1}\n",
    json: r#"{"units": "mm", "objects": [{"text": {"contents": V, "position": [0, 0, 0], "height": 1}}]}"#,
};

const MTEXT_CONTENTS: Place = Place {
    name: "an mtext's contents in a template",
    yaml: "units: mm\nobjects:\n  - mtext: {contents: V, position: [0, 0, 0], height: 1}\n",
    json: r#"{"units": "mm", "objects": [{"mtext": {"contents": V, "position": [0, 0, 0], "height": 1}}]}"#,
};

const LAYER_NAME: Place = Place {
    name: "an object's layer in a template",
    yaml: "units: mm\nobjects:\n  - circle: {center: [0, 0, 0], radius: 1, layer: V}\n",
    json: r#"{"units": "mm", "objects": [{"circle": {"center": [0, 0, 0], "radius": 1, "layer": V}}]}"#,
};

/// Reads a template as a set builds it, from a row that gives each value
/// it takes the text `A`.
fn read_template_with_every_value(yaml: &str) -> Result<(), Error> {
    let values: Vec<String> = names(yaml).iter().map(|n| format!("{n}: A")).collect();
    let row = one_row(&format!("{{{}}}", values.join(", ")));
    fill_in(yaml, &row).map(|_| ())
}

#[test]
fn the_template_schema_accepts_exactly_the_templates_the_reader_accepts() {
    let mut cases: Vec<Case> = Vec::new();
    for place in [&ATTRIBUTE_VALUE, &TEXT_CONTENTS, &MTEXT_CONTENTS] {
        cases.extend([
            (place, "PLAIN TEXT", json!("PLAIN TEXT"), true),
            (place, "{row: TITLE}", json!({"row": "TITLE"}), true),
            (place, "{row: dwg-no_2}", json!({"row": "dwg-no_2"}), true),
            (place, "{row: '7'}", json!({"row": "7"}), true),
            (place, "{row: 7}", json!({"row": 7}), true),
            (place, "{row: true}", json!({"row": true}), true),
            (place, "{row: 'DWG NO'}", json!({"row": "DWG NO"}), false),
            (place, "{row: ''}", json!({"row": ""}), false),
            (place, "{row: [A]}", json!({"row": ["A"]}), false),
            (
                place,
                "{row: A, default: B}",
                json!({"row": "A", "default": "B"}),
                false,
            ),
            (place, "{}", json!({}), false),
            (place, "{value: A}", json!({"value": "A"}), false),
            (place, "[A]", json!(["A"]), false),
        ]);
    }
    // Only an attribute's value and text's contents come from the row.
    cases.push((&LAYER_NAME, "{row: LAYER}", json!({"row": "LAYER"}), false));
    assert_reader_and_schema_agree(&template_schema(), cases, read_template_with_every_value);
}

#[test]
fn the_sheet_description_schema_takes_no_value_from_a_row() {
    let validator = jsonschema::validator_for(&super::schema::sheet_description_schema()).unwrap();
    let json: Value = serde_json::from_str(
        &TEXT_CONTENTS
            .json
            .replace('V', &json!({"row": "TITLE"}).to_string()),
    )
    .unwrap();
    assert!(!validator.is_valid(&json));
    assert!(parse_sheet_description(&TEXT_CONTENTS.yaml.replace('V', "{row: TITLE}")).is_err());
}
