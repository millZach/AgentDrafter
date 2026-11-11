//! A block defined once in a block library is inserted on two sheets built
//! from it. Each DWG defines the block as the library does, and reading it
//! back gives every insert's attribute values, position, scale, rotation
//! and name. An insert of a missing block, or of an attribute the block
//! does not have, is an error that lists the valid choices.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_with, parse_block_library, read_dwg, read_sheet_description, Block, BuildReport,
    BuildRequest, DrawingContents, ErrorCode, Insert, Object, Settings, Sheet, TextStyle,
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
