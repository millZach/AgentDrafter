//! Text styles, text and multiline text in a sheet description.

use super::parse_sheet_description;
use crate::error::{Error, ErrorCode};
use crate::model::{Object, Point3, Sheet};
use crate::settings::Settings;
use crate::text::measure::{measure_text_with, MeasureTextRequest};
use crate::text::{Attachment, Justify, MText, Text, TextStyle};

fn sheet(text: &str) -> Sheet {
    parse_sheet_description(text).unwrap_or_else(|e| panic!("{e}"))
}

const STYLES: &str = "text_styles:
  notes: {font: romans.shx, width_factor: 0.8}
  title: {font: Arial.TTF}
";

/// The one object of a sheet in mm with the styles above.
fn one(object: &str) -> Object {
    let mut all = sheet(&format!("units: mm\n{STYLES}objects:\n  - {object}\n")).objects;
    assert_eq!(all.len(), 1);
    all.remove(0)
}

fn error(text: &str) -> Error {
    match parse_sheet_description(text) {
        Ok(sheet) => panic!("expected an error, read {sheet:?}"),
        Err(error) => error,
    }
}

/// The error for a sheet in mm with the styles above holding `object`.
fn object_error(object: &str) -> Error {
    error(&format!("units: mm\n{STYLES}objects:\n  - {object}\n"))
}

#[track_caller]
fn assert_names(e: &Error, code: ErrorCode, field: &str, words: &[&str]) {
    assert_eq!(e.code, code, "code of {e:?}");
    assert_eq!(e.field.as_deref(), Some(field), "field of {e:?}");
    assert!(e.line.is_some() && e.column.is_some(), "position of {e:?}");
    for word in words {
        assert!(e.message.contains(word), "`{word}` in {:?}", e.message);
    }
}

#[test]
fn text_styles_read_with_their_fonts_and_width_factors() {
    let sheet = sheet(&format!("units: mm\n{STYLES}objects: []\n"));
    assert_eq!(
        sheet.text_styles,
        [
            TextStyle {
                name: "notes".into(),
                font: "romans.shx".into(),
                width_factor: 0.8,
            },
            TextStyle::new("title", "Arial.TTF"),
        ]
    );
}

#[test]
fn text_reads_every_key() {
    assert_eq!(
        one("text: {contents: PUMP 101, position: [10, 20, 0], height: 2.5, style: title, width_factor: 1.2, rotation: -90, justify: middle_center, layer: '0'}"),
        Object::Text(Text {
            contents: "PUMP 101".into(),
            position: Point3::new(10.0, 20.0, 0.0),
            height: 2.5,
            style: "title".into(),
            width_factor: 1.2,
            rotation: 270.0,
            justify: Justify::MiddleCenter,
            layer: "0".into(), properties: Default::default(),
        })
    );
}

#[test]
fn text_takes_its_styles_width_factor_and_sensible_defaults() {
    let Object::Text(text) =
        one("text: {contents: A, position: [0, 0, 0], height: 1 in, style: notes}")
    else {
        panic!()
    };
    assert_eq!(text.width_factor, 0.8);
    assert_eq!(text.height, 25.4);
    assert_eq!(text.rotation, 0.0);
    assert_eq!(text.justify, Justify::Left);
    let Object::Text(text) = one("text: {contents: A, position: [0, 0, 0], height: 1}") else {
        panic!()
    };
    assert_eq!((text.style.as_str(), text.width_factor), ("Standard", 1.0));
}

#[test]
fn mtext_reads_every_key_and_drops_line_breaks_at_the_end() {
    let object = sheet(&format!(
        "units: mm\n{STYLES}objects:
  - mtext:
      contents: |
        FIRST LINE
        SECOND {{LINE}} 100%

      position: [0, 50, 0]
      height: 3
      width: 80
      style: notes
      width_factor: 0.9
      rotation: 45
      justify: bottom_right
"
    ))
    .objects
    .remove(0);
    assert_eq!(
        object,
        Object::MText(MText {
            contents: "FIRST LINE\nSECOND {LINE} 100%".into(),
            position: Point3::new(0.0, 50.0, 0.0),
            height: 3.0,
            width: 80.0,
            style: "notes".into(),
            width_factor: Some(0.9),
            rotation: 45.0,
            justify: Attachment::BottomRight,
            layer: "0".into(),
            properties: Default::default(),
        })
    );
    let Object::MText(text) =
        one("mtext: {contents: A, position: [0, 0, 0], height: 1, style: notes}")
    else {
        panic!()
    };
    // Multiline text takes its style's width factor in AutoCAD; it states none of its own.
    assert_eq!(text.width_factor, None);
    assert_eq!(text.width, 0.0);
    assert_eq!(text.justify, Attachment::TopLeft);
}

#[test]
fn standard_can_be_listed_to_set_its_font() {
    let sheet = sheet(
        "units: mm\ntext_styles:\n  Standard: {font: simplex.shx}\nobjects:\n  - text: {contents: A, position: [0, 0, 0], height: 1, style: Standard}\n",
    );
    assert_eq!(
        sheet.text_styles,
        [TextStyle::new("Standard", "simplex.shx")]
    );
}

#[test]
fn standard_in_another_case_is_an_error() {
    // AutoCAD would take it for `Standard`; the sheet would not.
    for name in ["standard", "STANDARD"] {
        let e = error(&format!(
            "units: mm\ntext_styles:\n  {name}: {{font: arial.ttf, width_factor: 0.5}}\nobjects:\n  - text: {{contents: A, position: [0, 0, 0], height: 1}}\n"
        ));
        assert_names(
            &e,
            ErrorCode::BadValue,
            &format!("text_styles.{name}"),
            &[
                &format!("`{name}`"),
                "list it as `Standard`",
                "ignores case",
            ],
        );
    }
}

#[test]
fn an_unknown_style_lists_the_sheets_styles() {
    let e = object_error("text: {contents: A, position: [0, 0, 0], height: 1, style: Notes}");
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].text.style",
        &["`Notes`", "Standard, notes, title"],
    );
}

#[test]
fn a_justification_that_does_not_exist_lists_the_choices() {
    let e = object_error("text: {contents: A, position: [0, 0, 0], height: 1, justify: centre}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].text.justify",
        &[
            "`centre`",
            "left, center, right, middle, top_left",
            "bottom_right",
        ],
    );
    // Multiline text has no baseline: `left` is a single line's.
    let e = object_error("mtext: {contents: A, position: [0, 0, 0], height: 1, justify: left}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].mtext.justify",
        &["top_left, top_center", "bottom_right"],
    );
    assert!(!e.message.contains(" left,"), "{}", e.message);
    let e = object_error("text: {contents: A, position: [0, 0, 0], height: 1, justify: [left]}");
    assert_names(
        &e,
        ErrorCode::WrongType,
        "objects[0].text.justify",
        &["middle_center"],
    );
}

#[test]
fn a_font_is_an_shx_or_truetype_file_name() {
    for font in [
        "romans",
        "arial.otf",
        ".shx",
        "fonts/romans.shx",
        "C:romans.shx",
    ] {
        let e = error(&format!(
            "units: mm\ntext_styles:\n  a: {{font: '{font}'}}\nobjects: []\n"
        ));
        assert_names(
            &e,
            ErrorCode::BadValue,
            "text_styles.a.font",
            &["`romans.shx`", "`arial.ttf`"],
        );
    }
    let e = error("units: mm\ntext_styles:\n  a: {width_factor: 1}\nobjects: []\n");
    assert_names(&e, ErrorCode::MissingKey, "text_styles.a.font", &["`font`"]);
}

#[test]
fn the_sheet_description_and_measuring_refuse_the_same_font_names_alike() {
    for font in [
        "a|b.shx",
        "x?.ttf",
        "<x>.shx",
        "x\".shx",
        "x*.ttf",
        "/some/where/x.ttf",
        "..\\x.shx",
        "C:x.ttf",
        "..",
        "romans",
        "arial.otf",
    ] {
        let read = error(&format!(
            "units: mm\ntext_styles:\n  a: {{font: '{font}'}}\nobjects: []\n"
        ));
        let measured = measure_text_with(
            &MeasureTextRequest {
                contents: "A".into(),
                font: font.into(),
                height: 1.0,
                width_factor: 1.0,
            },
            &Settings::default(),
        )
        .expect_err(&format!("`{font}` is not a font's file name"));
        assert_eq!(
            read.field.as_deref(),
            Some("text_styles.a.font"),
            "{read:?}"
        );
        assert_eq!(measured.field.as_deref(), Some("font"), "{measured:?}");
        assert_eq!(
            (measured.code, &measured.message),
            (read.code, &read.message),
            "`{font}`"
        );
    }
}

#[test]
fn style_names_follow_autocads_rules() {
    let e = error(
        "units: mm\ntext_styles:\n  Notes: {font: a.shx}\n  NOTES: {font: b.shx}\nobjects: []\n",
    );
    assert_names(
        &e,
        ErrorCode::DuplicateKey,
        "text_styles.NOTES",
        &["ignores case"],
    );
    let e = error("units: mm\ntext_styles:\n  'a|b': {font: a.shx}\nobjects: []\n");
    assert_names(&e, ErrorCode::BadValue, "text_styles.a|b", &["`|`"]);
    let e = error("units: mm\ntext_styles:\n  a: {font: a.shx, height: 2}\nobjects: []\n");
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "text_styles.a.height",
        &["font, width_factor"],
    );
}

#[test]
fn heights_and_width_factors_are_checked() {
    let e = object_error("text: {contents: A, position: [0, 0, 0], height: 0}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].text.height",
        &["more than 0"],
    );
    let e = object_error("text: {contents: A, position: [0, 0, 0], height: 1, width_factor: 0}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].text.width_factor",
        &["0.01 to 100"],
    );
    let e = object_error("text: {contents: A, position: [0, 0, 0], height: 1, width_factor: 1 mm}");
    assert_names(
        &e,
        ErrorCode::BadNumber,
        "objects[0].text.width_factor",
        &["no unit"],
    );
    let e = error("units: mm\ntext_styles:\n  a: {font: a.shx, width_factor: 101}\nobjects: []\n");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "text_styles.a.width_factor",
        &["0.01 to 100"],
    );
    let e = object_error("mtext: {contents: A, position: [0, 0, 0], height: 1, width: -1}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].mtext.width",
        &["0 or more"],
    );
}

#[test]
fn contents_show_as_written_or_are_refused() {
    let e = object_error("text: {contents: \"one\\ntwo\", position: [0, 0, 0], height: 1}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].text.contents",
        &["`mtext`"],
    );
    let e = object_error("text: {contents: '  ', position: [0, 0, 0], height: 1}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].text.contents",
        &["needs contents"],
    );
    let e = object_error("text: {contents: \"a\\tb\", position: [0, 0, 0], height: 1}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].text.contents",
        &["`\\t`"],
    );
    let e = object_error("text: {contents: 'deg \\U+00B0', position: [0, 0, 0], height: 1}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].text.contents",
        &["`\\U+00B0`", "character itself"],
    );
    let e = object_error("mtext: {contents: \"\\n\\n\", position: [0, 0, 0], height: 1}");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "objects[0].mtext.contents",
        &["needs contents"],
    );
    let e = object_error("text: {position: [0, 0, 0], height: 1}");
    assert_names(
        &e,
        ErrorCode::MissingKey,
        "objects[0].text.contents",
        &["`contents`"],
    );
    // Every scalar is text: a number or `yes` is what it says.
    let Object::Text(text) = one("text: {contents: 100, position: [0, 0, 0], height: 1}") else {
        panic!()
    };
    assert_eq!(text.contents, "100");
    let Object::MText(text) =
        one("mtext: {contents: 'C:\\temp 50%%', position: [0, 0, 0], height: 1}")
    else {
        panic!()
    };
    assert_eq!(text.contents, "C:\\temp 50%%");
}

#[test]
fn an_unknown_key_lists_the_keys_text_takes() {
    let e = object_error("text: {contents: A, position: [0, 0, 0], height: 1, oblique: 15}");
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].text.oblique",
        &["a text takes contents, position, height, style, width_factor, rotation, justify, layer"],
    );
    let e = object_error("mtext: {contents: A, position: [0, 0, 0], height: 1, columns: 2}");
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].mtext.columns",
        &["an mtext takes", "width"],
    );
}
