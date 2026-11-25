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
                annotative: false,
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
            underline: false,
            overline: false,
            layer: "0".into(),
            field: None,
            properties: Default::default(),
            annotative: false,
            annotation_scales: Vec::new(),
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
            runs: Vec::new(),
            position: Point3::new(0.0, 50.0, 0.0),
            height: 3.0,
            width: 80.0,
            style: "notes".into(),
            width_factor: Some(0.9),
            rotation: 45.0,
            justify: Attachment::BottomRight,
            underline: false,
            overline: false,
            layer: "0".into(),
            field: None,
            properties: Default::default(),
            annotative: false,
            annotation_scales: Vec::new(),
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
fn text_and_mtext_parse_and_round_trip_underline_and_overline() {
    let Object::Text(t) = one(
        "text: {contents: 'NOTES:', position: [10, 20, 0], height: 3, underline: true, overline: true}",
    ) else {
        panic!()
    };
    assert!(t.underline);
    assert!(t.overline);
    let Object::MText(m) = one(
        "mtext: {contents: 'LINE 1\\nLINE 2', position: [10, 30, 0], height: 3, underline: true}",
    ) else {
        panic!()
    };
    assert!(m.underline);
    assert!(!m.overline);

    // Full DWG build + read-back round-trip (#60).
    let dir = std::env::temp_dir().join(format!("agentdrafter-uline-dwg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let yaml = dir.join("sheet.yaml");
    let dwg = dir.join("sheet.dwg");
    std::fs::write(
        &yaml,
        "units: mm\nobjects:\n\
         \x20 - text: {contents: 'NOTES:', position: [10, 20, 0], height: 3, underline: true}\n\
         \x20 - text: {contents: 'HEADER', position: [10, 30, 0], height: 3, overline: true}\n\
         \x20 - text: {contents: 'BOTH', position: [10, 40, 0], height: 3, underline: true, overline: true}\n\
         \x20 - mtext: {contents: 'MTEXT', position: [10, 50, 0], height: 3, underline: true, overline: true}\n",
    )
    .unwrap();
    let settings = crate::Settings::default();
    crate::build_with(
        &crate::BuildRequest {
            description: yaml,
            output: Some(dwg.clone()),
            overwrite: true,
        },
        &settings,
    )
    .unwrap();
    let read = crate::read_original(&crate::ReadRequest {
        source: dwg.clone(),
        mirror: None,
        no_mirror: false,
    })
    .unwrap();
    assert!(read.decoded);
    assert!(read.not_read.is_empty(), "{:?}", read.not_read);
    let mirror = std::fs::read_to_string(read.mirror.as_deref().unwrap()).unwrap();
    assert!(mirror.contains("underline: true"), "{mirror}");
    assert!(mirror.contains("overline: true"), "{mirror}");
    let read_dwg = crate::dwg::read(&dwg).unwrap();
    assert_eq!(read_dwg.objects.len(), 4);
    match &read_dwg.objects[0] {
        Object::Text(t) => assert_eq!(
            (t.contents.as_str(), t.underline, t.overline),
            ("NOTES:", true, false)
        ),
        other => panic!("expected text: {other:?}"),
    }
    match &read_dwg.objects[1] {
        Object::Text(t) => assert_eq!(
            (t.contents.as_str(), t.underline, t.overline),
            ("HEADER", false, true)
        ),
        other => panic!("expected text: {other:?}"),
    }
    match &read_dwg.objects[2] {
        Object::Text(t) => assert_eq!(
            (t.contents.as_str(), t.underline, t.overline),
            ("BOTH", true, true)
        ),
        other => panic!("expected text: {other:?}"),
    }
    match &read_dwg.objects[3] {
        Object::MText(m) => assert_eq!(
            (m.contents.as_str(), m.underline, m.overline),
            ("MTEXT", true, true)
        ),
        other => panic!("expected mtext: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mtext_inline_runs_parse_build_draw_and_round_trip() {
    // Rejects specifying both `contents` and `runs`.
    let both_err =
        object_error("mtext: {contents: A, runs: [{text: A}], position: [0, 0, 0], height: 2}");
    assert_eq!(both_err.code, ErrorCode::BadValue);

    // Full DWG build + read-back + display-list test with inline formatting (#61).
    let dir = std::env::temp_dir().join(format!("agentdrafter-mtext-runs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let yaml = dir.join("sheet.yaml");
    let dwg = dir.join("sheet.dwg");
    std::fs::write(
        &yaml,
        "units: mm\nobjects:\n\
         \x20 - mtext:\n\
         \x20     position: [10, 50, 0]\n\
         \x20     height: 3\n\
         \x20     width: 80\n\
         \x20     runs:\n\
         \x20       - {text: 'NOTES:', font: Arial, bold: true, underline: true, color: red}\n\
         \x20       - {text: \"\\n1. Plain text \"}\n\
         \x20       - {text: 'BIG', height: 5, width_factor: 0.9, oblique: 12, tracking: 1.1, overline: true}\n",
    )
    .unwrap();
    let settings = crate::Settings::default();
    crate::build_with(
        &crate::BuildRequest {
            description: yaml,
            output: Some(dwg.clone()),
            overwrite: true,
        },
        &settings,
    )
    .unwrap();
    let read = crate::read_original(&crate::ReadRequest {
        source: dwg.clone(),
        mirror: None,
        no_mirror: false,
    })
    .unwrap();
    assert!(read.decoded);
    assert!(read.not_read.is_empty(), "{:?}", read.not_read);
    let mirror = std::fs::read_to_string(read.mirror.as_deref().unwrap()).unwrap();
    assert!(mirror.contains("runs:"), "{mirror}");
    assert!(mirror.contains("bold: true"), "{mirror}");

    let read_dwg = crate::dwg::read(&dwg).unwrap();
    assert_eq!(read_dwg.objects.len(), 1);
    let Object::MText(m) = &read_dwg.objects[0] else {
        panic!("expected mtext");
    };
    assert_eq!(m.contents, "NOTES:\n1. Plain text BIG");
    assert_eq!(m.runs.len(), 3);
    assert_eq!(m.runs[0].font.as_deref(), Some("Arial"));
    assert!(m.runs[0].bold && m.runs[0].underline);
    assert_eq!(m.runs[0].color, Some(crate::model::ObjectColor::Index(1)));
    assert_eq!(m.runs[2].height, Some(5.0));
    assert_eq!(m.runs[2].width_factor, Some(0.9));
    assert_eq!(m.runs[2].oblique, Some(12.0));
    assert_eq!(m.runs[2].tracking, Some(1.1));
    assert!(m.runs[2].overline);

    // Drawing pipeline emits red (Color(1)) items for the first run alongside
    // the layer's default colour items for the other runs.
    let dl_req: crate::DisplayListRequest =
        serde_json::from_value(serde_json::json!({ "source": dwg })).unwrap();
    let dl = crate::display_list_with(&dl_req, &settings).unwrap();
    assert!(
        dl.display_list
            .items
            .iter()
            .any(|it| it.color == crate::Color(1)),
        "expected red inline-coloured run in display list"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
        &["a text takes contents, field, position, height, style, width_factor, rotation, justify, underline, overline, annotative, annotation_scales, layer"],
    );
    let e = object_error("mtext: {contents: A, position: [0, 0, 0], height: 1, columns: 2}");
    assert_names(
        &e,
        ErrorCode::UnknownKey,
        "objects[0].mtext.columns",
        &["an mtext takes", "width"],
    );
}
