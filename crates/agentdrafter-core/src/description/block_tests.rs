//! Block libraries, their blocks and attribute definitions, and inserts of
//! them on a sheet.

use std::fs;
use std::path::PathBuf;

use super::{parse_block_library, parse_sheet_description, read_sheet_description, sheet};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Attribute, AttributeDefinition, Block, BlockLibrary, Circle, Insert, Layer, Linetype, Object,
    Point3, Sheet,
};
use crate::text::{Justify, TextStyle, STANDARD};
use crate::units::Unit;

/// A made-up library: a marker with a tag and a hidden note, and an arrow
/// with no attributes.
const LIBRARY: &str = "units: mm
linetypes:
  DASHED: {pattern: [2, -1]}
layers:
  symbols: {color: yellow}
  hidden: {color: cyan, linetype: DASHED}
  unused: {color: red}
text_styles:
  tags: {font: romans.shx, width_factor: 0.8}
  spare: {font: arial.ttf}
blocks:
  MARKER:
    base_point: [0, -5, 0]
    objects:
      - circle: {center: [0, 0, 0], radius: 5, layer: symbols}
      - attdef: {tag: TAG, prompt: Tag, default: '?', position: [0, 0, 0], height: 2.5, style: tags, justify: middle_center, layer: symbols}
      - attdef: {tag: NOTE, position: [0, -8, 0], height: 1.5, invisible: true, layer: hidden}
  ARROW:
    objects:
      - line: {from: [0, 0, 0], to: [10, 0, 0]}
";

fn library() -> BlockLibrary {
    parse_block_library(LIBRARY).unwrap_or_else(|e| panic!("{e}"))
}

/// Reads a sheet that draws on `library` without touching the disk.
fn read_with(text: &str, library: &str) -> Result<Sheet, Error> {
    sheet(
        super::strict_yaml::read(text)?,
        &mut |_| parse_block_library(library),
        None,
    )
}

fn with_library(text: &str) -> Sheet {
    read_with(text, LIBRARY).unwrap_or_else(|e| panic!("{e}"))
}

fn error_with_library(text: &str) -> Error {
    match read_with(text, LIBRARY) {
        Ok(sheet) => panic!("expected an error, read {sheet:?}"),
        Err(error) => error,
    }
}

fn library_error(text: &str) -> Error {
    match parse_block_library(text) {
        Ok(library) => panic!("expected an error, read {library:?}"),
        Err(error) => error,
    }
}

/// A sheet in mm on the library above holding `object`.
fn one_object(object: &str) -> String {
    format!("units: mm\nblock_library: blocks.yaml\nobjects:\n  - {object}\n")
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

/// An empty folder for one test under the system's temporary folder.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("agentdrafter-blocks-{}", std::process::id()))
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_library_reads_its_blocks_with_their_attribute_definitions() {
    let library = library();
    assert_eq!(library.units, Unit::Mm);
    assert_eq!(library.layers.len(), 3);
    let marker = &library.blocks[0];
    assert_eq!(marker.name, "MARKER");
    assert_eq!(marker.base_point, Point3::new(0.0, -5.0, 0.0));
    assert_eq!(
        marker.objects[0],
        Object::Circle(Circle {
            center: Point3::new(0.0, 0.0, 0.0),
            radius: 5.0,
            layer: "symbols".into(),
            properties: Default::default(),
        })
    );
    assert_eq!(
        marker.objects[1],
        Object::AttributeDefinition(AttributeDefinition {
            tag: "TAG".into(),
            prompt: "Tag".into(),
            default: "?".into(),
            position: Point3::new(0.0, 0.0, 0.0),
            height: 2.5,
            style: "tags".into(),
            width_factor: 0.8,
            rotation: 0.0,
            justify: Justify::MiddleCenter,
            invisible: false,
            layer: "symbols".into(),
            properties: Default::default(),
        })
    );
    assert_eq!(
        marker.objects[2],
        Object::AttributeDefinition(AttributeDefinition {
            tag: "NOTE".into(),
            prompt: String::new(),
            default: String::new(),
            position: Point3::new(0.0, -8.0, 0.0),
            height: 1.5,
            style: STANDARD.into(),
            width_factor: 1.0,
            rotation: 0.0,
            justify: Justify::Left,
            invisible: true,
            layer: "hidden".into(),
            properties: Default::default(),
        })
    );
    let arrow = &library.blocks[1];
    assert_eq!(arrow.base_point, Point3::new(0.0, 0.0, 0.0));
    assert_eq!(arrow.attribute_definitions().count(), 0);
}

#[test]
fn an_insert_reads_every_key_and_fills_in_attributes_it_leaves_out() {
    let sheet = with_library(&one_object(
        "insert: {block: MARKER, position: [100, 50, 0], scale: 2, rotation: -90, attributes: {NOTE: 'SEE NOTE 2'}, name: m1, layer: '0'}",
    ));
    assert_eq!(
        sheet.objects,
        [Object::Insert(Insert {
            block: "MARKER".into(),
            position: Point3::new(100.0, 50.0, 0.0),
            scale: 2.0,
            rotation: 270.0,
            attributes: vec![
                Attribute {
                    tag: "TAG".into(),
                    value: "?".into(),
                },
                Attribute {
                    tag: "NOTE".into(),
                    value: "SEE NOTE 2".into(),
                },
            ],
            name: Some("m1".into()),
            layer: "0".into(),
            properties: Default::default(),
        })]
    );
    let plain = with_library(&one_object("insert: {block: ARROW, position: [0, 0, 0]}"));
    let Object::Insert(arrow) = &plain.objects[0] else {
        panic!("an insert")
    };
    assert_eq!(
        (arrow.scale, arrow.rotation, arrow.name.as_deref()),
        (1.0, 0.0, None)
    );
    assert!(arrow.attributes.is_empty());
}

#[test]
fn a_sheet_takes_the_blocks_it_inserts_and_the_tables_they_use() {
    let sheet = with_library(
        "units: mm
block_library: blocks.yaml
layers:
  symbols: {color: yellow}
  notes: {color: green}
objects:
  - insert: {block: MARKER, position: [0, 0, 0], attributes: {TAG: A-1}}
  - insert: {block: MARKER, position: [20, 0, 0], attributes: {TAG: A-2}}
",
    );
    assert_eq!(sheet.block_library.as_deref(), Some("blocks.yaml"));
    let names: Vec<&str> = sheet.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["MARKER"]);
    assert_eq!(sheet.blocks[0], library().blocks[0]);
    let layers: Vec<&str> = sheet.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(layers, ["symbols", "notes", "hidden"]);
    assert_eq!(
        sheet.linetypes,
        [Linetype {
            name: "DASHED".into(),
            description: String::new(),
            pattern: vec![2.0, -1.0],
        }]
    );
    let styles: Vec<&str> = sheet.text_styles.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(styles, ["tags"]);

    let none = with_library("units: mm\nblock_library: blocks.yaml\nobjects: []\n");
    assert!(none.blocks.is_empty() && none.layers.is_empty() && none.text_styles.is_empty());
}

#[test]
fn an_insert_of_a_missing_block_lists_the_librarys_blocks() {
    let e = error_with_library(&one_object("insert: {block: VALVE, position: [0, 0, 0]}"));
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].insert.block",
        &["unknown block `VALVE`", "MARKER, ARROW"],
    );
    // Names are written exactly as the library writes them.
    let e = error_with_library(&one_object("insert: {block: marker, position: [0, 0, 0]}"));
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].insert.block",
        &["MARKER, ARROW"],
    );
    let e = parse_sheet_description(
        "units: mm\nobjects:\n  - insert: {block: MARKER, position: [0, 0, 0]}\n",
    )
    .unwrap_err();
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].insert.block",
        &["names no block library", "block_library:"],
    );
}

#[test]
fn an_attribute_the_block_does_not_have_lists_its_attributes() {
    let e = error_with_library(&one_object(
        "insert: {block: MARKER, position: [0, 0, 0], attributes: {TAG: A-1, SIZE: '2'}}",
    ));
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].insert.attributes.SIZE",
        &["unknown attribute `SIZE`", "TAG, NOTE"],
    );
    let e = error_with_library(&one_object(
        "insert: {block: MARKER, position: [0, 0, 0], attributes: {tag: A-1}}",
    ));
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].insert.attributes.tag",
        &["TAG, NOTE"],
    );
    let e = error_with_library(&one_object(
        "insert: {block: ARROW, position: [0, 0, 0], attributes: {TAG: A-1}}",
    ));
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].insert.attributes.TAG",
        &["block `ARROW` has no attributes"],
    );
}

#[test]
fn an_inserts_values_are_checked() {
    for (object, code, field, words) in [
        (
            "insert: {block: MARKER, position: [0, 0, 0], attributes: {TAG: \"A\\nB\"}}",
            ErrorCode::BadValue,
            "objects[0].insert.attributes.TAG",
            &["one line"][..],
        ),
        (
            "insert: {block: MARKER, position: [0, 0, 0], attributes: {TAG: '\\U+00B0'}}",
            ErrorCode::BadValue,
            "objects[0].insert.attributes.TAG",
            &["character code"],
        ),
        (
            "insert: {block: MARKER, position: [0, 0, 0], attributes: [TAG]}",
            ErrorCode::WrongType,
            "objects[0].insert.attributes",
            &["a mapping from attribute tags to values"],
        ),
        (
            "insert: {block: MARKER, position: [0, 0, 0], scale: 0}",
            ErrorCode::BadValue,
            "objects[0].insert.scale",
            &["more than 0"],
        ),
        (
            "insert: {block: MARKER, position: [0, 0, 0], scale: 2 mm}",
            ErrorCode::BadNumber,
            "objects[0].insert.scale",
            &["no unit"],
        ),
        (
            "insert: {block: MARKER, position: [0, 0, 0], name: ' jb'}",
            ErrorCode::BadValue,
            "objects[0].insert.name",
            &["start or end with a space"],
        ),
        (
            "insert: {block: MARKER, position: [0, 0, 0], name: ''}",
            ErrorCode::BadValue,
            "objects[0].insert.name",
            &["cannot be empty"],
        ),
        (
            "insert: {block: MARKER, position: [0, 0, 0], colour: red}",
            ErrorCode::UnknownKey,
            "objects[0].insert.colour",
            &["an insert takes block, position, scale, rotation, attributes, layer, color, linetype, lineweight, linetype_scale, id, name"],
        ),
        (
            "insert: {position: [0, 0, 0]}",
            ErrorCode::MissingKey,
            "objects[0].insert.block",
            &["block"],
        ),
        (
            "attdef: {tag: TAG, position: [0, 0, 0], height: 1}",
            ErrorCode::UnknownKey,
            "objects[0].attdef",
            &["belongs in a block in the block library", "insert"],
        ),
    ] {
        let e = error_with_library(&one_object(object));
        assert_names(&e, code, field, words);
    }
    // An empty value is a value.
    let sheet = with_library(&one_object(
        "insert: {block: MARKER, position: [0, 0, 0], attributes: {TAG: ''}}",
    ));
    let Object::Insert(insert) = &sheet.objects[0] else {
        panic!("an insert")
    };
    assert_eq!(insert.attributes[0].value, "");
}

#[test]
fn a_bare_null_leaves_a_text_field_out_and_quoted_is_the_word() {
    // Zach decided on 2026-11-11: a bare null on an optional text field
    // is the field left out, so an attribute takes its definition's
    // default, and a name or a layer is as if not written.
    for written in ["TAG:", "TAG: null", "TAG: ~", "TAG: NULL"] {
        let sheet = with_library(&format!(
            "units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert:\n      block: MARKER\n      position: [0, 0, 0]\n      name: null\n      layer: ~\n      attributes:\n        {written}\n"
        ));
        let Object::Insert(insert) = &sheet.objects[0] else {
            panic!("an insert")
        };
        assert_eq!(insert.attributes[0].value, "?", "{written}");
        assert_eq!((insert.name.as_deref(), insert.layer.as_str()), (None, "0"));
    }
    let sheet = with_library(&one_object(
        "insert: {block: MARKER, position: [0, 0, 0], name: 'null', attributes: {TAG: 'null', NOTE: ''}}",
    ));
    let Object::Insert(insert) = &sheet.objects[0] else {
        panic!("an insert")
    };
    assert_eq!(insert.name.as_deref(), Some("null"));
    assert_eq!(insert.attributes[0].value, "null");
    assert_eq!(insert.attributes[1].value, "");
    // A tag the block does not have is refused even with a bare null.
    let e = error_with_library(&one_object(
        "insert: {block: MARKER, position: [0, 0, 0], attributes: {LABEL: null}}",
    ));
    assert_eq!(e.code, ErrorCode::UnknownName, "{e:?}");
    // On a required field a bare null is the field left out: an error.
    let e = error_with_library(&one_object("insert: {block: null, position: [0, 0, 0]}"));
    assert_names(
        &e,
        ErrorCode::MissingKey,
        "objects[0].insert.block",
        &["`block` is a bare `null`", "quote it: `'null'`"],
    );
    let e = library_error(
        "units: mm\nblocks:\n  B: {objects: [{attdef: {tag:, position: [0, 0, 0], height: 1}}]}\n",
    );
    assert_names(
        &e,
        ErrorCode::MissingKey,
        "blocks.B.objects[0].attdef.tag",
        &["`tag` is left empty"],
    );
}

#[test]
fn a_name_is_given_once_on_a_sheet() {
    let e = error_with_library(
        "units: mm
block_library: blocks.yaml
objects:
  - insert: {block: ARROW, position: [0, 0, 0], name: jb}
  - insert: {block: ARROW, position: [5, 0, 0], name: JB}
",
    );
    assert_names(
        &e,
        ErrorCode::DuplicateName,
        "objects[1].insert.name",
        &["`JB`", "line 4", "differ only in case"],
    );
    // The line given is the first object's `name:` key's, not its own
    // first line (#35).
    let e = error_with_library(
        "units: mm
block_library: blocks.yaml
objects:
  - insert:
      block: ARROW
      position: [0, 0, 0]
      name: jb
  - insert: {block: ARROW, position: [5, 0, 0], name: jb}
",
    );
    assert_names(
        &e,
        ErrorCode::DuplicateName,
        "objects[1].insert.name",
        &["given at line 7 already, to the object at `objects[0]`"],
    );
}

#[test]
fn a_block_library_is_checked_as_a_sheet_is() {
    for (text, code, field, words) in [
        (
            "blocks: {}\n",
            ErrorCode::MissingUnit,
            "units",
            &["the block library declares no unit"][..],
        ),
        (
            "units: mm\n",
            ErrorCode::MissingKey,
            "blocks",
            &["missing key `blocks`"],
        ),
        (
            "units: mm\nobjects: []\nblocks: {}\n",
            ErrorCode::UnknownKey,
            "objects",
            &["a block library takes units, linetypes, layers, hatch_patterns, text_styles, blocks"],
        ),
        (
            "units: mm\nblocks:\n  'A/B': {objects: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}]}\n",
            ErrorCode::BadValue,
            "blocks.A/B",
            &["a block name cannot hold `/`"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}]}\n  a: {objects: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}]}\n",
            ErrorCode::DuplicateKey,
            "blocks.a",
            &["AutoCAD ignores case"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: []}\n",
            ErrorCode::WrongType,
            "blocks.A.objects",
            &["at least one object"],
        ),
        (
            "units: mm\nblocks:\n  A: {base: [0, 0, 0], objects: [{line: {from: [0, 0, 0], to: [1, 0, 0]}}]}\n",
            ErrorCode::UnknownKey,
            "blocks.A.base",
            &["a block takes base_point, objects"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{insert: {block: B, position: [0, 0, 0]}}]}\n",
            ErrorCode::UnknownName,
            "blocks.A.objects[0].insert.block",
            &["unknown block `B`", "the block library's blocks: A"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{circle: {center: [0, 0, 0], radius: 1, layer: symbols}}]}\n",
            ErrorCode::UnknownName,
            "blocks.A.objects[0].circle.layer",
            &["unknown layer `symbols`"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{attdef: {tag: Tag, position: [0, 0, 0], height: 1}}]}\n",
            ErrorCode::BadValue,
            "blocks.A.objects[0].attdef.tag",
            &["in capitals", "`TAG`"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{attdef: {tag: TWO WORDS, position: [0, 0, 0], height: 1}}]}\n",
            ErrorCode::BadValue,
            "blocks.A.objects[0].attdef.tag",
            &["cannot hold a space"],
        ),
        (
            "units: mm\nblocks:\n  A:\n    objects:\n      - attdef: {tag: TAG, position: [0, 0, 0], height: 1}\n      - attdef: {tag: TAG, position: [0, 2, 0], height: 1}\n",
            ErrorCode::DuplicateKey,
            "blocks.A.objects[1].attdef.tag",
            &["defined twice", "line 5"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{attdef: {tag: TAG, position: [0, 0, 0], height: 1, prompt: \"A\\nB\"}}]}\n",
            ErrorCode::BadValue,
            "blocks.A.objects[0].attdef.prompt",
            &["a prompt is one line"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{attdef: {tag: TAG, position: [0, 0, 0], height: 1, invisible: yes}}]}\n",
            ErrorCode::WrongType,
            "blocks.A.objects[0].attdef.invisible",
            &["`true` or `false`"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{attdef: {tag: TAG, position: [0, 0, 0]}}]}\n",
            ErrorCode::MissingKey,
            "blocks.A.objects[0].attdef.height",
            &["height"],
        ),
        (
            "units: mm\nblocks:\n  A: {objects: [{attdef: {tag: TAG, position: [0, 0, 0], height: 1, justify: aligned}}]}\n",
            ErrorCode::BadValue,
            "blocks.A.objects[0].attdef.justify",
            &["middle_center"],
        ),
    ] {
        let e = library_error(text);
        assert_names(&e, code, field, words);
    }
    let e = library_error("");
    assert_eq!(e.code, ErrorCode::MissingKey);
}

#[test]
fn a_sheet_names_its_block_library_by_a_relative_path() {
    for written in [
        "/blocks.yaml",
        "C:/blocks.yaml",
        "lib\\blocks.yaml",
        "lib//blocks.yaml",
        "lib/",
        "''",
    ] {
        let e = error_with_library(&format!(
            "units: mm\nblock_library: {written}\nobjects: []\n"
        ));
        assert_names(&e, ErrorCode::BadValue, "block_library", &["block library"]);
    }
    for written in ["blocks.yaml", "../set/blocks.yaml", "./library/blocks"] {
        with_library(&format!(
            "units: mm\nblock_library: {written}\nobjects: []\n"
        ));
    }
    let e = parse_sheet_description("units: mm\nblock_library: blocks.yaml\nobjects: []\n")
        .unwrap_err();
    assert_names(
        &e,
        ErrorCode::Usage,
        "block_library",
        &["read it from its file"],
    );
}

#[test]
fn a_sheet_draws_on_a_library_in_its_own_unit() {
    let e = error_with_library("units: in\nblock_library: blocks.yaml\nobjects: []\n");
    assert_names(
        &e,
        ErrorCode::BadValue,
        "block_library",
        &["in mm", "the sheet in in"],
    );
}

#[test]
fn a_table_entry_the_sheet_and_its_blocks_both_use_is_defined_the_same() {
    let e = error_with_library(
        "units: mm
block_library: blocks.yaml
layers:
  symbols: {color: red}
objects:
  - insert: {block: MARKER, position: [0, 0, 0]}
",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "layers.symbols",
        &["layer `symbols`", "differently in the block library"],
    );
    let e = error_with_library(
        "units: mm
block_library: blocks.yaml
text_styles:
  TAGS: {font: romans.shx, width_factor: 0.8}
objects:
  - insert: {block: MARKER, position: [0, 0, 0]}
",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "text_styles.TAGS",
        &["text style `TAGS`", "`tags` in another case"],
    );
    let e = error_with_library(
        "units: mm
block_library: blocks.yaml
linetypes:
  DASHED: {pattern: [3, -1]}
objects:
  - insert: {block: MARKER, position: [0, 0, 0]}
",
    );
    assert_names(
        &e,
        ErrorCode::BadValue,
        "linetypes.DASHED",
        &["linetype `DASHED`"],
    );
    // A layer only the library's unused blocks or nothing at all use is not compared.
    let sheet = with_library(
        "units: mm
block_library: blocks.yaml
layers:
  unused: {color: blue}
  symbols: {color: yellow}
objects:
  - insert: {block: ARROW, position: [0, 0, 0]}
",
    );
    assert_eq!(
        sheet.layers,
        [
            Layer {
                color: 5,
                ..Layer::new("unused")
            },
            Layer {
                color: 2,
                ..Layer::new("symbols")
            },
        ]
    );
    assert_eq!(sheet.text_styles, Vec::<TextStyle>::new());
    assert_eq!(
        sheet.blocks,
        vec![library().blocks[1].clone()] as Vec<Block>
    );
}

#[test]
fn a_sheet_description_finds_its_block_library_beside_it() {
    let dir = scratch("beside");
    fs::create_dir_all(dir.join("library")).unwrap();
    fs::write(dir.join("library/blocks.yaml"), LIBRARY).unwrap();
    let description = dir.join("sheet.yaml");
    fs::write(
        &description,
        "units: mm\nblock_library: library/blocks.yaml\nobjects:\n  - insert: {block: ARROW, position: [0, 0, 0]}\n",
    )
    .unwrap();
    let sheet = read_sheet_description(&description).unwrap();
    assert_eq!(sheet.blocks.len(), 1);

    // A library that is not there is an error at the key that names it.
    fs::write(
        &description,
        "units: mm\nblock_library: missing.yaml\nobjects: []\n",
    )
    .unwrap();
    let e = read_sheet_description(&description).unwrap_err();
    assert_names(
        &e,
        ErrorCode::ReadFailed,
        "block_library",
        &["missing.yaml"],
    );
    assert_eq!(e.file, Some(description.display().to_string()));

    // An error in the library names the library's file.
    fs::write(
        dir.join("library/blocks.yaml"),
        "units: mm\nblocks: {A: {objects: []}}\n",
    )
    .unwrap();
    fs::write(
        &description,
        "units: mm\nblock_library: library/blocks.yaml\nobjects: []\n",
    )
    .unwrap();
    let e = read_sheet_description(&description).unwrap_err();
    assert_names(
        &e,
        ErrorCode::WrongType,
        "blocks.A.objects",
        &["at least one object"],
    );
    assert_eq!(
        e.file,
        Some(dir.join("library/blocks.yaml").display().to_string())
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A made-up terminal strip: a block that inserts a terminal block twice,
/// filling in each terminal's tag in its own definition.
const NESTED: &str = "units: mm
layers:
  terminals: {color: green}
blocks:
  STRIP:
    objects:
      - line: {from: [0, 0, 0], to: [20, 0, 0]}
      - insert: {block: TERMINAL, position: [5, 0, 0], attributes: {TAG: '1'}}
      - insert: {block: TERMINAL, position: [15, 0, 0], attributes: {TAG: '2'}, layer: terminals}
  TERMINAL:
    objects:
      - circle: {center: [0, 0, 0], radius: 2}
      - attdef: {tag: TAG, position: [0, 3, 0], height: 2, justify: middle_center}
";

#[test]
fn a_block_inserts_another_block_and_fills_in_its_attributes() {
    let library = parse_block_library(NESTED).unwrap_or_else(|e| panic!("{e}"));
    // The blocks stay in the file's order, though TERMINAL is read first.
    let names: Vec<&str> = library.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["STRIP", "TERMINAL"]);
    let Object::Insert(first) = &library.blocks[0].objects[1] else {
        panic!("a nested insert")
    };
    assert_eq!(first.block, "TERMINAL");
    assert_eq!(first.attributes[0].value, "1");

    // A sheet that inserts the strip takes the terminal too.
    let sheet = read_with(
        "units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert: {block: STRIP, position: [0, 0, 0]}\n",
        NESTED,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let taken: Vec<&str> = sheet.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(taken, ["STRIP", "TERMINAL"]);
    assert!(sheet.layers.iter().any(|l| l.name == "terminals"));

    // The strip's insert cannot set a terminal's tag: the strip's definition
    // sets it, once.
    let e = read_with(
        "units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert: {block: STRIP, position: [0, 0, 0], attributes: {TAG: X}}\n",
        NESTED,
    )
    .unwrap_err();
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].insert.attributes.TAG",
        &[
            "belongs to block `TERMINAL`, which block `STRIP` inserts",
            "set once",
        ],
    );

    // Nested two deep, the tag is found where it is set: in PANEL's insert
    // of STRIP, STRIP's definition sets it.
    let deeper = NESTED.replace(
        "blocks:
",
        "blocks:
  PANEL:
    objects:
      - insert: {block: STRIP, position: [0, 0, 0]}
",
    );
    let e = read_with(
        "units: mm
block_library: blocks.yaml
objects:
  - insert: {block: PANEL, position: [0, 0, 0], attributes: {TAG: X}}
",
        &deeper,
    )
    .unwrap_err();
    assert_names(
        &e,
        ErrorCode::UnknownName,
        "objects[0].insert.attributes.TAG",
        &[
            "belongs to block `TERMINAL`, which block `STRIP` inserts, nested in block `PANEL`",
            "in block `STRIP`'s definition",
        ],
    );
}

#[test]
fn a_block_that_inserts_itself_is_refused() {
    let e = library_error(
        "units: mm\nblocks:\n  A:\n    objects:\n      - insert: {block: B, position: [0, 0, 0]}\n  B:\n    objects:\n      - insert: {block: A, position: [0, 0, 0]}\n",
    );
    assert_eq!(e.code, ErrorCode::BadValue);
    assert!(
        e.message.contains("inserts itself, through A to B to A"),
        "{}",
        e.message
    );
}
