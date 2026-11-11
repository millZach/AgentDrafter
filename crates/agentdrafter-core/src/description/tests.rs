use super::parse_sheet_description;
use crate::error::{Error, ErrorCode};
use crate::model::{Line, Object, Point3, Sheet};
use crate::units::Unit;

const ONE_LINE: &str = "\
units: mm
objects:
  - line:
      from: [0, 0, 0]
      to: [100, 50, 0]
";

fn error(text: &str) -> Error {
    match parse_sheet_description(text) {
        Ok(sheet) => panic!("expected an error, read {sheet:?}"),
        Err(error) => error,
    }
}

fn assert_error(text: &str, code: ErrorCode, field: Option<&str>, line: usize) {
    let e = error(text);
    assert_eq!(e.code, code, "code of {e:?}");
    assert_eq!(e.field.as_deref(), field, "field of {e:?}");
    assert_eq!(e.line, Some(line), "line of {e:?}");
    assert!(e.column.is_some(), "column of {e:?}");
}

#[test]
fn reads_one_line() {
    assert_eq!(
        parse_sheet_description(ONE_LINE).unwrap(),
        Sheet {
            units: Unit::Mm,
            linetypes: Vec::new(),
            layers: Vec::new(),
            objects: vec![Object::Line(Line {
                from: Point3::new(0.0, 0.0, 0.0),
                to: Point3::new(100.0, 50.0, 0.0),
                layer: "0".to_owned(),
            })],
        }
    );
}

#[test]
fn block_style_points_and_quoted_text_read_the_same() {
    let text = "\
\"units\": 'mm'
objects:
  - line:
      from:
        - 0
        - 0
        - 0
      to: ['100', \"50\", 0]
";
    assert_eq!(
        parse_sheet_description(text).unwrap(),
        parse_sheet_description(ONE_LINE).unwrap()
    );
}

#[test]
fn a_number_with_its_own_unit_is_converted_to_the_declared_unit() {
    let sheet = parse_sheet_description(
        "units: mm\nobjects:\n  - line: {from: [1 in, 0.5 m, 0], to: [2cm, -1 ft, 3 mm]}\n",
    )
    .unwrap();
    assert_eq!(
        sheet.objects,
        vec![Object::Line(Line {
            from: Point3::new(25.4, 500.0, 0.0),
            to: Point3::new(20.0, -304.8, 3.0),
            layer: "0".to_owned(),
        })]
    );
}

#[test]
fn a_huge_length_converts_when_its_result_is_finite_and_is_refused_when_not() {
    let sheet = parse_sheet_description(
        "units: m\nobjects:\n  - line: {from: [1e307 in, 0, 0], to: [1, 1, 0]}\n",
    )
    .unwrap();
    let [Object::Line(line)] = sheet.objects.as_slice() else {
        panic!("expected one line, read {:?}", sheet.objects);
    };
    assert!(line.from.x.is_finite(), "{:?}", line.from);

    assert_error(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 1e308 m, 0]}\n",
        ErrorCode::BadNumber,
        Some("objects[0].line.to[1]"),
        3,
    );
    let e = error("units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 1e308 m, 0]}\n");
    assert_eq!(e.message, "`1e308 m` is too large once converted to mm");
}

#[test]
fn every_declared_unit_is_read() {
    for unit in Unit::ALL {
        let text = format!("units: {unit}\nobjects: []\n");
        assert_eq!(parse_sheet_description(&text).unwrap().units, unit);
    }
}

#[test]
fn a_comment_and_an_explicit_document_marker_are_fine() {
    let text = format!("# one line\n---\n{ONE_LINE}...\n");
    assert_eq!(
        parse_sheet_description(&text).unwrap(),
        parse_sheet_description(ONE_LINE).unwrap()
    );
}

#[test]
fn an_unknown_key_is_named() {
    assert_error(
        "units: mm\nobjects: []\ncolour: red\n",
        ErrorCode::UnknownKey,
        Some("colour"),
        3,
    );
    assert_error(
        "units: mm\nobjects:\n  - line:\n      form: [0, 0, 0]\n      to: [1, 1, 0]\n",
        ErrorCode::UnknownKey,
        Some("objects[0].line.form"),
        4,
    );
    let e = error("units: mm\nobjects:\n  - line:\n      form: [0, 0, 0]\n");
    assert_eq!(
        e.message,
        "unknown key `form`; a line takes from, to, layer"
    );
}

#[test]
fn an_unknown_object_type_is_named() {
    let e = error("units: mm\nobjects:\n  - hexagon: {centre: [0, 0, 0]}\n");
    assert_eq!(e.code, ErrorCode::UnknownKey);
    assert_eq!(e.field.as_deref(), Some("objects[0].hexagon"));
    assert_eq!(
        e.message,
        "unknown object type `hexagon`; known types: line, polyline, arc, circle, ellipse, spline"
    );
}

#[test]
fn a_duplicate_key_is_named() {
    assert_error(
        "units: mm\nunits: in\nobjects: []\n",
        ErrorCode::DuplicateKey,
        Some("units"),
        2,
    );
    let text = "units: mm\nobjects:\n  - line:\n      from: [0, 0, 0]\n      to: [1, 1, 0]\n      from: [2, 2, 0]\n";
    assert_error(
        text,
        ErrorCode::DuplicateKey,
        Some("objects[0].line.from"),
        6,
    );
    assert_eq!(
        error(text).message,
        "duplicate key `from`: it is first given at line 4"
    );
}

#[test]
fn a_duplicate_key_in_flow_style_is_named() {
    assert_error(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1, 1, 0], to: [2, 2, 0]}\n",
        ErrorCode::DuplicateKey,
        Some("objects[0].line.to"),
        3,
    );
}

#[test]
fn a_missing_unit_is_named() {
    let e = error("objects:\n  - line: {from: [0, 0, 0], to: [1, 1, 0]}\n");
    assert_eq!(e.code, ErrorCode::MissingUnit);
    assert_eq!(e.field.as_deref(), Some("units"));
    assert_eq!(
        e.message,
        "the sheet declares no unit; add `units:` with one of mm, cm, m, in, ft"
    );
    assert_error(
        "units:\nobjects: []\n",
        ErrorCode::MissingUnit,
        Some("units"),
        1,
    );
}

#[test]
fn an_unknown_unit_is_named() {
    assert_error(
        "units: furlong\nobjects: []\n",
        ErrorCode::UnknownUnit,
        Some("units"),
        1,
    );
    assert_error(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0], to: [1 furlong, 1, 0]}\n",
        ErrorCode::UnknownUnit,
        Some("objects[0].line.to[0]"),
        3,
    );
}

#[test]
fn a_value_that_is_not_a_number_is_named() {
    for bad in [
        "abc", "0x10", "1_000", ".5", "+1", ".inf", "null", "~", "true", "1 mm 2",
    ] {
        let text =
            format!("units: mm\nobjects:\n  - line: {{from: [0, {bad}, 0], to: [1, 1, 0]}}\n");
        let e = error(&text);
        assert_eq!(e.code, ErrorCode::BadNumber, "{bad}: {e:?}");
        assert_eq!(e.field.as_deref(), Some("objects[0].line.from[1]"), "{bad}");
    }
    assert_error(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 1e999], to: [1, 1, 0]}\n",
        ErrorCode::BadNumber,
        Some("objects[0].line.from[2]"),
        3,
    );
}

#[test]
fn a_point_has_three_coordinates() {
    assert_error(
        "units: mm\nobjects:\n  - line: {from: [0, 0], to: [1, 1, 0]}\n",
        ErrorCode::WrongType,
        Some("objects[0].line.from"),
        3,
    );
    assert_error(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0, 0], to: [1, 1, 0]}\n",
        ErrorCode::WrongType,
        Some("objects[0].line.from"),
        3,
    );
    assert_error(
        "units: mm\nobjects:\n  - line: {from: 0, to: [1, 1, 0]}\n",
        ErrorCode::WrongType,
        Some("objects[0].line.from"),
        3,
    );
}

#[test]
fn a_missing_key_is_named() {
    assert_error(
        "units: mm\nobjects:\n  - line: {from: [0, 0, 0]}\n",
        ErrorCode::MissingKey,
        Some("objects[0].line.to"),
        3,
    );
    assert_error("units: mm\n", ErrorCode::MissingKey, Some("objects"), 1);
}

#[test]
fn values_of_the_wrong_shape_are_named() {
    assert_error(
        "units: mm\nobjects: {line: 1}\n",
        ErrorCode::WrongType,
        Some("objects"),
        2,
    );
    assert_error(
        "units: mm\nobjects:\n  - line\n",
        ErrorCode::WrongType,
        Some("objects[0]"),
        3,
    );
    assert_error(
        "units: [mm]\nobjects: []\n",
        ErrorCode::WrongType,
        Some("units"),
        1,
    );
    let e = error("- units: mm\n");
    assert_eq!(e.code, ErrorCode::WrongType);
    assert_eq!(e.field, None);
}

#[test]
fn an_empty_description_is_refused() {
    for text in ["", "\n", "# nothing here\n"] {
        assert_eq!(error(text).code, ErrorCode::MissingKey, "{text:?}");
    }
}

#[test]
fn anchors_and_aliases_are_refused() {
    assert_error(
        "units: mm\nobjects:\n  - line: {from: &origin [0, 0, 0], to: [1, 1, 0]}\n",
        ErrorCode::NotInSubset,
        Some("objects[0].line.from"),
        3,
    );
    assert_error(
        "units: &u mm\nobjects: []\n",
        ErrorCode::NotInSubset,
        Some("units"),
        1,
    );
    let e = error("units: mm\nobjects:\n  - line: {from: &o [0, 0, 0], to: *o}\n");
    assert_eq!(e.code, ErrorCode::NotInSubset);
    assert!(e.message.contains("anchors"), "{e:?}");
}

#[test]
fn an_alias_is_refused_even_without_reaching_its_anchor() {
    // The anchor is refused first; an alias to an unknown anchor is a syntax error.
    let e = error("units: mm\nobjects: *nothing\n");
    assert!(
        matches!(e.code, ErrorCode::NotInSubset | ErrorCode::YamlSyntax),
        "{e:?}"
    );
}

#[test]
fn tags_are_refused() {
    for text in [
        "units: !!str mm\nobjects: []\n",
        "units: mm\nobjects: !!seq []\n",
        "units: mm\nobjects: !custom []\n",
        "!!map\nunits: mm\nobjects: []\n",
    ] {
        let e = error(text);
        assert_eq!(e.code, ErrorCode::NotInSubset, "{text:?}: {e:?}");
        assert!(e.message.contains("tags"), "{e:?}");
    }
}

#[test]
fn directives_are_refused() {
    assert_error(
        "%YAML 1.2\n---\nunits: mm\nobjects: []\n",
        ErrorCode::NotInSubset,
        None,
        1,
    );
}

#[test]
fn a_leading_byte_order_mark_is_ignored_and_every_rule_still_applies() {
    let with_mark = format!("\u{FEFF}{ONE_LINE}");
    assert_eq!(
        parse_sheet_description(&with_mark).unwrap(),
        parse_sheet_description(ONE_LINE).unwrap()
    );
    assert_error(
        "\u{FEFF}%YAML 1.2\n---\nunits: mm\nobjects: []\n",
        ErrorCode::NotInSubset,
        None,
        1,
    );
    let e = error("\u{FEFF}units: furlong\nobjects: []\n");
    assert_eq!(e.code, ErrorCode::UnknownUnit);
    assert_eq!((e.line, e.column), (Some(1), Some(8)), "{e:?}");
}

#[test]
fn a_second_document_is_refused() {
    assert_error(
        "units: mm\nobjects: []\n---\nunits: in\nobjects: []\n",
        ErrorCode::NotInSubset,
        None,
        3,
    );
}

#[test]
fn a_key_that_is_not_text_is_refused() {
    assert_error(
        "units: mm\nobjects: []\n? [a, b]\n: c\n",
        ErrorCode::NotInSubset,
        None,
        3,
    );
}

#[test]
fn broken_yaml_is_a_syntax_error_with_a_position() {
    let e = error("units: mm\nobjects: [\n");
    assert_eq!(e.code, ErrorCode::YamlSyntax);
    assert!(e.line.is_some() && e.column.is_some(), "{e:?}");
}
