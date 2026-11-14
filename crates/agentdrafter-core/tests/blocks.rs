//! A block defined once in a block library is inserted on two sheets built
//! from it. Each DWG defines the block as the library does, and reading it
//! back gives every insert's attribute values, position, scale, rotation
//! and name. An insert of a missing block, or of an attribute the block
//! does not have, is an error that lists the valid choices.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_with, display_list_with, parse_block_library, read_dwg, read_sheet_description,
    Attribute, Block, BuildReport, BuildRequest, DisplayListRequest, DrawingContents, ErrorCode,
    Insert, NoticeCode, Object, Settings, Sheet, TextStyle,
};

/// How far an angle read back may be from the one written, in degrees. A
/// DWG holds angles in radians, so degrees do not come back bit for bit.
const ANGLE_TOLERANCE: f64 = 1e-9;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-blocks")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A copy of the block library example in a folder of its own.
fn example_set(test: &str) -> PathBuf {
    let dir = scratch(test);
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/blocks");
    for file in ["blocks.yaml", "sheet-1.yaml", "sheet-2.yaml"] {
        fs::copy(examples.join(file), dir.join(file)).unwrap();
    }
    dir
}

fn build(description: &Path) -> BuildReport {
    build_with(
        &BuildRequest {
            description: description.to_owned(),
            output: None,
            overwrite: false,
        },
        &Settings::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// Builds the sheet description at `description` and returns the sheet,
/// the build's report and what the DWG reads back as.
fn build_and_read(description: &Path) -> (Sheet, BuildReport, DrawingContents) {
    let report = build(description);
    let sheet = read_sheet_description(description).unwrap();
    let read = read_dwg(Path::new(&report.dwg)).unwrap();
    assert_eq!(read.dwg_version, "AC1032");
    assert_eq!(read.objects_not_read, 0, "{read:#?}");
    assert_eq!(read.blocks_not_read, 0, "{read:#?}");
    assert_eq!(read.text_styles_not_read, 0, "{read:#?}");
    (sheet, report, read)
}

fn block<'a>(read: &'a DrawingContents, name: &str) -> &'a Block {
    read.blocks
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("block {name} in {:#?}", read.blocks))
}

fn inserts(objects: &[Object]) -> Vec<&Insert> {
    objects
        .iter()
        .filter_map(|o| match o {
            Object::Insert(insert) => Some(insert),
            _ => None,
        })
        .collect()
}

/// Whether an object read back is the one written: everything bit for bit
/// but rotations, which match to [`ANGLE_TOLERANCE`].
fn same_object(wrote: &Object, came_back: &Object) -> bool {
    let close = |a: f64, b: f64| (a - b).abs() <= ANGLE_TOLERANCE;
    match (wrote, came_back) {
        (Object::Insert(w), Object::Insert(r)) => {
            let mut r2 = r.clone();
            r2.rotation = w.rotation;
            close(w.rotation, r.rotation) && *w == r2
        }
        (Object::AttributeDefinition(w), Object::AttributeDefinition(r)) => {
            let mut r2 = r.clone();
            r2.rotation = w.rotation;
            close(w.rotation, r.rotation) && *w == r2
        }
        (Object::Text(w), Object::Text(r)) => {
            let mut r2 = r.clone();
            r2.rotation = w.rotation;
            close(w.rotation, r.rotation) && *w == r2
        }
        _ => wrote == came_back,
    }
}

#[track_caller]
fn assert_same_list(wrote: &[Object], came_back: &[Object]) {
    assert_eq!(came_back.len(), wrote.len(), "{came_back:#?}");
    for (i, (w, r)) in wrote.iter().zip(came_back).enumerate() {
        assert!(
            same_object(w, r),
            "object {i}: wrote {w:#?}, read back {r:#?}"
        );
    }
}

/// Asserts that the DWG read back holds the sheet's objects, in order.
#[track_caller]
fn assert_same_objects(sheet: &Sheet, read: &DrawingContents) {
    assert_same_list(&sheet.objects, &read.objects);
}

/// Asserts that the DWG read back defines `block` as it was written.
#[track_caller]
fn assert_same_block(block: &Block, read: &DrawingContents) {
    let came_back = self::block(read, &block.name);
    assert_eq!(came_back.base_point, block.base_point);
    assert_same_list(&block.objects, &came_back.objects);
}

#[test]
fn a_block_defined_once_is_inserted_on_two_sheets_from_the_same_library() {
    let dir = example_set("two-sheets");
    let library =
        parse_block_library(&fs::read_to_string(dir.join("blocks.yaml")).unwrap()).unwrap();
    let marker = library.blocks.iter().find(|b| b.name == "MARKER").unwrap();

    let (_, first_report, first) = build_and_read(&dir.join("sheet-1.yaml"));
    let (_, second_report, second) = build_and_read(&dir.join("sheet-2.yaml"));

    // Each DWG defines the block it inserts as the library does.
    assert_same_block(marker, &first);
    assert_same_block(marker, &second);
    assert_eq!(first_report.blocks, ["MARKER", "ARROW"]);
    assert_eq!(second_report.blocks, ["MARKER"]);
    assert_eq!(second.blocks.len(), 1, "only the blocks a sheet inserts");
    assert_eq!(
        first_report.block_library,
        Some(dir.join("blocks.yaml").display().to_string())
    );
    assert_eq!(inserts(&first.objects).len(), 3);
    assert_eq!(inserts(&second.objects).len(), 2);
    assert_eq!(first_report.objects.get("insert"), Some(&3));

    // The library's layers and text styles the blocks use are in each DWG.
    for read in [&first, &second] {
        for layer in &library.layers {
            assert!(
                read.layers.contains(layer),
                "{layer:?} in {:#?}",
                read.layers
            );
        }
        assert_eq!(
            read.text_styles,
            [TextStyle::standard(), library.text_styles[0].clone()]
        );
    }
}

#[test]
fn attribute_values_position_scale_rotation_and_name_read_back() {
    let dir = example_set("read-back");
    let (sheet, _, first) = build_and_read(&dir.join("sheet-1.yaml"));
    assert_same_objects(&sheet, &first);
    let (sheet, _, second) = build_and_read(&dir.join("sheet-2.yaml"));
    assert_same_objects(&sheet, &second);

    let turned = inserts(&first.objects)[1];
    assert_eq!(turned.block, "MARKER");
    assert_eq!(turned.position.x, 100.0);
    assert_eq!(turned.position.y, 60.0);
    assert_eq!(turned.scale, 1.5);
    assert!((turned.rotation - 30.0).abs() <= ANGLE_TOLERANCE);
    assert_eq!(turned.name.as_deref(), Some("marker-a2"));
    let values: Vec<(&str, &str)> = turned
        .attributes
        .iter()
        .map(|a| (a.tag.as_str(), a.value.as_str()))
        .collect();
    assert_eq!(
        values,
        [("TAG", "A-2"), ("NUMBER", "102"), ("NOTE", "SEE NOTE 1")]
    );
    // An attribute the insert leaves out takes its definition's default.
    let defaulted = inserts(&second.objects)[1];
    assert_eq!(defaulted.attributes[1].value, "0");
    assert_eq!(defaulted.attributes[2].value, "");
    assert_eq!(defaulted.name, None);
}

#[test]
fn a_block_in_another_unit_scale_or_turn_reads_back_from_its_own_library() {
    let dir = scratch("inches");
    fs::write(
        dir.join("library.yaml"),
        "units: in
text_styles:
  Standard: {font: simplex.shx}
blocks:
  'TAG BUBBLE':
    base_point: [1, 1, 0]
    objects:
      - circle: {center: [1, 1, 0], radius: 0.25}
      - attdef: {tag: ID, position: [1, 1, 0], height: 0.125, justify: middle, rotation: 15}
      - attdef: {tag: LOAD, position: [1.5, 1, 0.5], height: 3 mm, justify: bottom_right, default: '50%'}
",
    )
    .unwrap();
    let description = dir.join("sheet.yaml");
    fs::write(
        &description,
        "units: in
block_library: library.yaml
text_styles:
  Standard: {font: simplex.shx}
objects:
  - insert: {block: TAG BUBBLE, position: [-3.5, 1e3, 2], scale: 0.3, rotation: 123.4, attributes: {ID: 'Ünï 100%%'}}
  - insert: {block: TAG BUBBLE, position: [0, 0, 0], scale: 40, rotation: 359.99}
",
    )
    .unwrap();
    let (sheet, _, read) = build_and_read(&description);
    assert_eq!(read.blocks.len(), 1);
    assert_same_block(&sheet.blocks[0], &read);
    assert_same_objects(&sheet, &read);
}

#[test]
fn an_insert_of_a_missing_block_or_attribute_is_an_error_listing_the_choices() {
    let dir = example_set("errors");
    let description = dir.join("bad.yaml");
    for (insert, field, listed) in [
        (
            "{block: VALVE, position: [0, 0, 0]}",
            "objects[0].insert.block",
            "the block library's blocks: MARKER, ARROW",
        ),
        (
            "{block: MARKER, position: [0, 0, 0], attributes: {SIZE: '2'}}",
            "objects[0].insert.attributes.SIZE",
            "block `MARKER` has the attributes TAG, NUMBER, NOTE",
        ),
    ] {
        fs::write(
            &description,
            format!("units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert: {insert}\n"),
        )
        .unwrap();
        let error = build_with(
            &BuildRequest {
                description: description.clone(),
                output: None,
                overwrite: false,
            },
            &Settings::default(),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownName, "{error:?}");
        assert_eq!(error.field.as_deref(), Some(field), "{error:?}");
        assert!(error.message.contains(listed), "{error:?}");
        assert_eq!(error.file, Some(description.display().to_string()));
        assert!(!dir.join("bad.dwg").exists(), "no DWG is written");
    }
}

#[test]
fn a_blocks_own_properties_and_its_inserts_read_back() {
    let dir = scratch("own-properties");
    fs::write(
        dir.join("blocks.yaml"),
        "units: mm
linetypes:
  DOTS: {pattern: [0, -1]}
blocks:
  B:
    objects:
               - line: {from: [0, 0, 0], to: [10, 0, 0], color: ByBlock, linetype: DOTS, lineweight: ByBlock}
               - attdef: {tag: TAG, position: [0, 5, 0], height: 2, color: ByBlock, lineweight: 0.5}
",
    )
    .unwrap();
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm
block_library: blocks.yaml
objects:
         - insert: {block: B, position: [0, 0, 0], attributes: {TAG: X}, color: 6, lineweight: 0.7}
         - insert: {block: B, position: [0, 20, 0], attributes: {TAG: Y}}
",
    )
    .unwrap();
    let (sheet, _, read) = build_and_read(&dir.join("sheet.yaml"));
    assert_same_objects(&sheet, &read);
    assert_same_block(&sheet.blocks[0], &read);
    assert!(
        read.linetypes.iter().any(|t| t.name == "DOTS"),
        "{:?}",
        read.linetypes
    );
}

/// A block that inserts another, defined before it in the library, comes
/// to the sheet with the block it inserts and not with a block nothing
/// inserts; the DWG defines both, the outer block's insert with its
/// attribute, and reads back as it was written.
#[test]
fn a_block_that_inserts_another_reads_back_with_both_defined() {
    let dir = scratch("nested");
    fs::write(
        dir.join("blocks.yaml"),
        "units: mm
blocks:
  STRIP:
    objects:
      - polyline: {points: [[0, 0, 0], [20, 0, 0], [20, 10, 0], [0, 10, 0]], closed: true}
      - insert: {block: TERMINAL, position: [5, 5, 0], attributes: {NUMBER: '1'}}
      - insert: {block: TERMINAL, position: [15, 5, 0], rotation: 90, scale: 0.5, attributes: {NUMBER: '2'}, color: ByBlock}
  TERMINAL:
    objects:
      - circle: {center: [0, 0, 0], radius: 2}
      - attdef: {tag: NUMBER, position: [0, -1, 0], height: 2, justify: middle_center}
  UNUSED:
    objects:
      - line: {from: [0, 0, 0], to: [1, 0, 0]}
",
    )
    .unwrap();
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm
block_library: blocks.yaml
objects:
  - insert: {block: STRIP, position: [50, 50, 0]}
  - insert: {block: STRIP, position: [50, 80, 0], scale: 2}
",
    )
    .unwrap();
    let (sheet, _, read) = build_and_read(&dir.join("sheet.yaml"));
    let names: Vec<&str> = sheet.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["STRIP", "TERMINAL"]);
    assert_same_objects(&sheet, &read);
    for block in &sheet.blocks {
        assert_same_block(block, &read);
    }
    assert!(read.blocks.iter().all(|b| b.name != "UNUSED"));
    let strip = block(&read, "STRIP");
    let numbers: Vec<&str> = inserts(&strip.objects)
        .iter()
        .map(|i| i.attributes[0].value.as_str())
        .collect();
    assert_eq!(numbers, ["1", "2"]);
}

/// Every attribute mode reads back from the DWG: a constant attribute in
/// the block's definition only, as AutoCAD writes it, and an insert's
/// attributes with their definition's Invisible, Verify, Preset and Lock
/// position.
#[test]
fn attribute_modes_read_back() {
    let dir = scratch("modes");
    fs::write(
        dir.join("blocks.yaml"),
        "units: mm
blocks:
  MODES:
    objects:
      - attdef: {tag: PLAIN, position: [0, 0, 0], height: 2, default: P}
      - attdef: {tag: HIDDEN, position: [0, 3, 0], height: 2, invisible: true}
      - attdef: {tag: FIXED, position: [0, 6, 0], height: 2, default: K-1, constant: true}
      - attdef: {tag: CHECKED, position: [0, 9, 0], height: 2, verify: true}
      - attdef: {tag: QUIET, position: [0, 12, 0], height: 2, preset: true}
      - attdef: {tag: PINNED, position: [0, 15, 0], height: 2, lock_position: true}
      - attdef: {tag: SECRET, position: [0, 18, 0], height: 2, default: S, constant: true, invisible: true}
",
    )
    .unwrap();
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm
block_library: blocks.yaml
objects:
  - insert: {block: MODES, position: [0, 0, 0], attributes: {PLAIN: A, HIDDEN: B, CHECKED: C, QUIET: D, PINNED: E}}
  - insert: {block: MODES, position: [50, 0, 0], rotation: 30, scale: 2}
",
    )
    .unwrap();
    let (sheet, _, read) = build_and_read(&dir.join("sheet.yaml"));
    assert_same_objects(&sheet, &read);
    assert_same_block(&sheet.blocks[0], &read);
    let modes = block(&read, "MODES");
    let constant: Vec<&str> = modes
        .attribute_definitions()
        .filter(|d| d.constant)
        .map(|d| d.tag.as_str())
        .collect();
    assert_eq!(constant, ["FIXED", "SECRET"]);
    for insert in inserts(&read.objects) {
        assert_eq!(insert.attributes.len(), 5, "{insert:?}");
    }
}

/// A made-up revision block of the kind a plant keeps: a tag defined more
/// than once, one of those times constant, and tags with punctuation and
/// one in lowercase, as AutoCAD allows them.
const REVISIONS: &str = "units: mm
blocks:
  REVISIONS:
    objects:
      - attdef: {tag: NOTE, position: [0, 0, 0], height: 2, default: '-'}
      - attdef: {tag: 'NOTE#1', position: [0, 3, 0], height: 2}
      - attdef: {tag: NOTE, position: [0, 6, 0], height: 2, default: '-'}
      - attdef: {tag: A/B, position: [0, 9, 0], height: 2}
      - attdef: {tag: NOTE, position: [0, 12, 0], height: 2, default: K, constant: true}
      - attdef: {tag: MM/DD/YY, position: [0, 15, 0], height: 2}
      - attdef: {tag: X.Y, position: [0, 18, 0], height: 2}
      - attdef: {tag: REV-1, position: [0, 21, 0], height: 2}
      - attdef: {tag: '#', position: [0, 24, 0], height: 2}
      - attdef: {tag: '1', position: [0, 27, 0], height: 2}
      - attdef: {tag: rev, position: [0, 30, 0], height: 2}
      - attdef: {tag: NOTE, position: [0, 33, 0], height: 2, default: '-'}
";

/// A tag a block defines more than once takes a list of values, one for
/// each definition but the constant one, in the block's order; tags with
/// punctuation are kept as written, and one written in lowercase is taken
/// in capitals, which the build's reply says. The DWG holds one attribute
/// for each definition, in order, reads back as written, and draws each
/// value where its own definition puts it.
#[test]
fn repeated_and_punctuated_tags_read_back_from_a_dwg() {
    let dir = scratch("repeated");
    fs::write(dir.join("blocks.yaml"), REVISIONS).unwrap();
    fs::write(
        dir.join("sheet.yaml"),
        "units: mm
block_library: blocks.yaml
objects:
  - insert: {block: REVISIONS, position: [0, 0, 0], attributes: {NOTE: [first, null, third], 'NOTE#1': n1, A/B: ab, MM/DD/YY: 11/13/26, X.Y: xy, REV-1: r1, '#': hash, '1': one, Rev: B}}
  - insert: {block: REVISIONS, position: [50, 0, 0], attributes: {NOTE: [only]}}
",
    )
    .unwrap();
    let (sheet, report, read) = build_and_read(&dir.join("sheet.yaml"));
    let tags: Vec<&str> = sheet.blocks[0]
        .attribute_definitions()
        .map(|d| d.tag.as_str())
        .collect();
    assert_eq!(
        tags,
        [
            "NOTE", "NOTE#1", "NOTE", "A/B", "NOTE", "MM/DD/YY", "X.Y", "REV-1", "#", "1", "REV",
            "NOTE"
        ]
    );
    let given = |tag: &str, value: &str| Attribute {
        tag: tag.into(),
        value: value.into(),
    };
    let Object::Insert(first) = &sheet.objects[0] else {
        panic!("an insert");
    };
    assert_eq!(
        first.attributes,
        [
            given("NOTE", "first"),
            given("NOTE#1", "n1"),
            given("NOTE", "-"),
            given("A/B", "ab"),
            given("MM/DD/YY", "11/13/26"),
            given("X.Y", "xy"),
            given("REV-1", "r1"),
            given("#", "hash"),
            given("1", "one"),
            given("REV", "B"),
            given("NOTE", "third"),
        ]
    );
    let Object::Insert(second) = &sheet.objects[1] else {
        panic!("an insert");
    };
    let notes: Vec<&str> = second
        .attributes
        .iter()
        .filter(|a| a.tag == "NOTE")
        .map(|a| a.value.as_str())
        .collect();
    assert_eq!(notes, ["only", "-", "-"]);
    // The library's `rev` and the sheet's `Rev`, each where it is written.
    let notices: Vec<(NoticeCode, &str, &str)> = report
        .notices
        .iter()
        .map(|n| (n.code, n.field.as_str(), n.message.as_str()))
        .collect();
    assert_eq!(notices.len(), 2, "{notices:?}");
    assert_eq!(notices[0].0, NoticeCode::TagInCapitals);
    assert_eq!(notices[0].1, "blocks.REVISIONS.objects[10].attdef.tag");
    assert!(report.notices[0].file.ends_with("blocks.yaml"));
    assert!(
        notices[0].2.contains("`rev` is taken as `REV`"),
        "{notices:?}"
    );
    assert_eq!(notices[1].1, "objects[0].insert.attributes.Rev");
    assert!(report.notices[1].file.ends_with("sheet.yaml"));
    // Read back from the DWG, each value is its own definition's.
    assert_same_objects(&sheet, &read);
    assert_same_block(&sheet.blocks[0], &read);
    // And each draws where its definition puts it: occurrence 3 of NOTE is
    // the last definition, 33 above the insert.
    let drawn = display_list_with(
        &DisplayListRequest::new(dir.join("sheet.yaml")),
        &Settings::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let at = |word: &str, object: usize| -> f64 {
        drawn
            .display_list
            .text
            .iter()
            .filter(|run| run.object == object)
            .find(|run| run.characters.iter().map(|(c, _)| c).collect::<String>() == word)
            .unwrap_or_else(|| panic!("`{word}` drawn"))
            .origin[1]
    };
    assert!((at("first", 0) - 0.0).abs() < 1e-9);
    assert!((at("third", 0) - 33.0).abs() < 1e-9);
    assert!((at("K", 0) - 12.0).abs() < 1e-9);
    assert!((at("only", 1) - 0.0).abs() < 1e-9);
}

/// A repeated tag takes a list no longer than the times the block defines
/// it, and not one value; a tag defined once takes one value, not a list.
/// A tag cannot hold a space, and one given twice, in two cases, is an
/// error.
#[test]
fn a_repeated_tag_takes_a_list_and_a_single_tag_one_value() {
    let dir = scratch("repeated-errors");
    fs::write(dir.join("blocks.yaml"), REVISIONS).unwrap();
    let sheet = dir.join("sheet.yaml");
    for (attributes, code, field, words) in [
        (
            "{NOTE: [a, b, c, d]}",
            ErrorCode::BadValue,
            "objects[0].insert.attributes.NOTE",
            "defines the attribute `NOTE` 3 times, so it takes at most 3 values",
        ),
        (
            "{NOTE: a}",
            ErrorCode::WrongType,
            "objects[0].insert.attributes.NOTE",
            "takes a list of values",
        ),
        (
            "{A/B: [a]}",
            ErrorCode::WrongType,
            "objects[0].insert.attributes.A/B",
            "defines the attribute `A/B` once",
        ),
        (
            "{X.Y: [a]}",
            ErrorCode::WrongType,
            "objects[0].insert.attributes.X.Y",
            "defines the attribute `X.Y` once",
        ),
        (
            "{NOTE: [a, \"b\\nc\"]}",
            ErrorCode::BadValue,
            "objects[0].insert.attributes.NOTE[1]",
            "one line",
        ),
        (
            "{rev: a, REV: b}",
            ErrorCode::DuplicateKey,
            "objects[0].insert.attributes.REV",
            "given twice",
        ),
        (
            "{'NOTE 2': a}",
            ErrorCode::UnknownName,
            "objects[0].insert.attributes.NOTE 2",
            "NOTE (3 times, given as a list), NOTE#1, A/B",
        ),
    ] {
        fs::write(
            &sheet,
            format!(
                "units: mm\nblock_library: blocks.yaml\nobjects:\n  - insert: {{block: REVISIONS, position: [0, 0, 0], attributes: {attributes}}}\n"
            ),
        )
        .unwrap();
        let e = read_sheet_description(&sheet).unwrap_err();
        assert_eq!(e.code, code, "{attributes}: {e:?}");
        assert_eq!(e.field.as_deref(), Some(field), "{attributes}: {e:?}");
        assert!(e.message.contains(words), "{attributes}: {}", e.message);
    }
    // A tag with a space is refused where the block defines it.
    let e = parse_block_library(&REVISIONS.replace("tag: X.Y", "tag: 'X Y'")).unwrap_err();
    assert_eq!(e.code, ErrorCode::BadValue);
    assert_eq!(
        e.field.as_deref(),
        Some("blocks.REVISIONS.objects[6].attdef.tag")
    );
    assert!(e.message.contains("cannot hold a space"), "{}", e.message);
}
