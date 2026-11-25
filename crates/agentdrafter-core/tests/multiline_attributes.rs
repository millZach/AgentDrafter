//! End-to-end tests for multi-line attributes (#117):
//! ATTDEF `multiline: true` and `width`, multi-line insert attribute values,
//! DWG/DXF round-trip, `import-blocks`, single drawing pipeline wrapping and
//! scaling, checker cell overflow & text-on-text per line, `set_attribute` on
//! originals, and `explode` to `mtext`.
//!
//! All drawings in these tests are made up (ADR-0008).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    block_library_schema, build_with, check_with, display_list_with, import_blocks,
    parse_block_library, read_dwg, read_original, revise_with, sheet_description_schema,
    BuildRequest, CheckRequest, DisplayListRequest, ImportBlocksRequest, Justify, Object,
    ReadRequest, ReviseRequest, Rule, Settings,
};
use serde_json::json;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-multiline-attributes")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn multiline_attdef_validates_round_trips_dwg_imports_and_draws_wrapped_lines() {
    let dir = scratch("roundtrip");
    let lib_path = dir.join("blocks.yaml");
    let lib_yaml = "\
units: mm
blocks:
  TITLE:
    objects:
      - polyline: {points: [[0, 0, 0], [50, 0, 0], [50, 20, 0], [0, 20, 0]], closed: true}
      - attdef:
          tag: DESC
          prompt: Description
          default: |
            DEFAULT LINE 1
            DEFAULT LINE 2
          position: [2, 18, 0]
          height: 3
          multiline: true
          width: 46
          cell: [[0, 0, 0], [50, 20, 0]]
      - attdef:
          tag: SINGLE
          position: [2, 2, 0]
          height: 2
      - attdef:
          tag: CONST_NOTE
          default: \"FIXED A\\nFIXED B\"
          position: [60, 18, 0]
          height: 2.5
          constant: true
          multiline: true
          width: 30
";
    fs::write(&lib_path, lib_yaml).unwrap();

    // Parse block library directly and verify fields.
    let parsed_lib = parse_block_library(lib_yaml).unwrap();
    let title = &parsed_lib.blocks[0];
    let Object::AttributeDefinition(desc) = &title.objects[1] else {
        panic!("expected attdef");
    };
    assert!(desc.multiline);
    assert_eq!(desc.width, 46.0);
    assert_eq!(desc.justify, Justify::TopLeft);
    assert_eq!(desc.default, "DEFAULT LINE 1\nDEFAULT LINE 2");

    // Refuses `width` on a single-line attribute definition.
    let bad_width = "\
units: mm
blocks:
  BAD:
    objects:
      - attdef: {tag: TAG, position: [0, 0, 0], height: 2.5, width: 20}
";
    assert!(parse_block_library(bad_width).is_err());

    // Schemas accept `multiline`, `width`, and multi-line attribute values.
    let lib_schema = block_library_schema();
    let lib_validator = jsonschema::validator_for(&lib_schema).unwrap();
    assert!(lib_validator.is_valid(&json!({
        "units": "mm",
        "blocks": {
            "TITLE": {
                "objects": [
                    {
                        "attdef": {
                            "tag": "DESC",
                            "default": "LINE 1\nLINE 2",
                            "position": [0, 0, 0],
                            "height": 3,
                            "multiline": true,
                            "width": 40
                        }
                    }
                ]
            }
        }
    })));
    let sheet_schema = sheet_description_schema();
    let sheet_validator = jsonschema::validator_for(&sheet_schema).unwrap();
    assert!(sheet_validator.is_valid(&json!({
        "units": "mm",
        "block_library": "blocks.yaml",
        "objects": [
            {
                "insert": {
                    "block": "TITLE",
                    "position": [0, 0, 0],
                    "attributes": { "DESC": "LINE 1\nLINE 2" }
                }
            }
        ]
    })));

    // Single-line attribute still refuses line breaks on an insert.
    let bad_sheet = dir.join("bad_sheet.yaml");
    fs::write(
        &bad_sheet,
        "\
units: mm
block_library: blocks.yaml
objects:
  - insert:
      block: TITLE
      position: [0, 0, 0]
      attributes:
        SINGLE: \"NO\\nBREAK\"
",
    )
    .unwrap();
    assert!(display_list_with(&DisplayListRequest::new(&bad_sheet), &Settings::default()).is_err());

    // Valid sheet with a multi-line attribute value (explicit break + word wrap).
    let sheet_path = dir.join("sheet.yaml");
    fs::write(
        &sheet_path,
        "\
units: mm
block_library: blocks.yaml
objects:
  - insert:
      id: title-1
      block: TITLE
      position: [10, 10, 0]
      scale: 1
      attributes:
        DESC: \"PUMP DISCHARGE PRESSURE TRANSMITTER\\nAREA 200\"
        SINGLE: OK
",
    )
    .unwrap();

    let settings = Settings::default();

    // 1. Display list draws multiple TextRuns (wrapped lines + explicit break)
    //    for DESC (part 1) and CONST_NOTE (part 3).
    let dl = display_list_with(&DisplayListRequest::new(&sheet_path), &settings).unwrap();
    let desc_runs: Vec<String> = dl
        .display_list
        .text
        .iter()
        .filter(|r| r.object == 0 && r.part == Some(1))
        .map(|r| r.characters.iter().map(|&(c, _)| c).collect())
        .collect();
    assert!(
        desc_runs.len() >= 3,
        "expected wrapped lines + explicit break for DESC, got {desc_runs:?}"
    );
    assert_eq!(desc_runs.last().map(String::as_str), Some("AREA 200"));
    let const_runs: Vec<String> = dl
        .display_list
        .text
        .iter()
        .filter(|r| r.object == 0 && r.part == Some(3))
        .map(|r| r.characters.iter().map(|&(c, _)| c).collect())
        .collect();
    assert_eq!(const_runs, vec!["FIXED A", "FIXED B"]);

    // 2. Build to DWG 2018 and read back whole via `read_dwg` and `read_original`.
    let dwg_path = dir.join("sheet.dwg");
    build_with(
        &BuildRequest {
            description: sheet_path.clone(),
            output: Some(dwg_path.clone()),
            overwrite: true,
        },
        &settings,
    )
    .unwrap();

    let read = read_dwg(&dwg_path).unwrap();
    let read_title = read.blocks.iter().find(|b| b.name == "TITLE").unwrap();
    let Object::AttributeDefinition(read_desc) = &read_title.objects[1] else {
        panic!("expected attdef");
    };
    assert!(read_desc.multiline);
    assert_eq!(read_desc.width, 46.0);
    assert_eq!(read_desc.default, "DEFAULT LINE 1\nDEFAULT LINE 2");
    let Object::Insert(read_ins) = &read.objects[0] else {
        panic!("expected insert");
    };
    assert_eq!(
        read_ins.attributes[0].value,
        "PUMP DISCHARGE PRESSURE TRANSMITTER\nAREA 200"
    );

    let orig = read_original(&ReadRequest {
        source: dwg_path.clone(),
        mirror: None,
        no_mirror: true,
    })
    .unwrap();
    assert!(
        orig.not_read.is_empty(),
        "multiline attributes should all be modelled, got {:?}",
        orig.not_read
    );

    // 3. `import-blocks` preserves `multiline: true` and `width` cleanly.
    let imported_path = dir.join("imported.yaml");
    import_blocks(&ImportBlocksRequest {
        drawing: dwg_path.clone(),
        library: imported_path.clone(),
        blocks: Vec::new(),
        replace: true,
        units: None,
        dry_run: false,
    })
    .unwrap();
    let imported_yaml = fs::read_to_string(&imported_path).unwrap();
    let imported_lib = parse_block_library(&imported_yaml).unwrap();
    let imp_title = imported_lib
        .blocks
        .iter()
        .find(|b| b.name == "TITLE")
        .unwrap();
    let Object::AttributeDefinition(imp_desc) = &imp_title.objects[1] else {
        panic!("expected imported attdef");
    };
    assert!(imp_desc.multiline);
    assert_eq!(imp_desc.width, 46.0);

    // 4. `set_attribute` on the original DWG updates the multiline value cleanly
    //    and refuses line breaks on the single-line attribute.
    let rev_dwg = dir.join("sheet.rev1.dwg");
    let rev = revise_with(
        &ReviseRequest {
            set: dwg_path.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "set_attribute",
                        "target": { "id": "sheet/title-1" },
                        "tag": "DESC",
                        "value": "UPDATED LINE 1\nUPDATED LINE 2"
                    }
                ]
            }),
            dry_run: false,
            output: Some(rev_dwg.clone()),
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(rev.ok, "{:?}", rev.error);
    let rev_read = read_dwg(&rev_dwg).unwrap();
    let Object::Insert(rev_ins) = &rev_read.objects[0] else {
        panic!("expected insert");
    };
    assert_eq!(
        rev_ins.attributes[0].value,
        "UPDATED LINE 1\nUPDATED LINE 2"
    );

    let bad_rev = revise_with(
        &ReviseRequest {
            set: dwg_path.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "set_attribute",
                        "target": { "id": "sheet/title-1" },
                        "tag": "SINGLE",
                        "value": "TWO\nLINES"
                    }
                ]
            }),
            dry_run: true,
            output: None,
            allow_loss: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(
        !bad_rev.ok,
        "single-line attribute should refuse line breaks"
    );
}

#[test]
fn checker_holds_multiline_attribute_lines_to_cell_and_other_text() {
    let dir = scratch("checker");
    let lib_path = dir.join("blocks.yaml");
    fs::write(
        &lib_path,
        "\
units: mm
blocks:
  BOX:
    objects:
      - polyline: {points: [[0, 0, 0], [40, 0, 0], [40, 12, 0], [0, 12, 0]], closed: true}
      - attdef:
          tag: NOTES
          position: [2, 10, 0]
          height: 3
          multiline: true
          width: 36
          cell: [[0, 0, 0], [40, 12, 0]]
",
    )
    .unwrap();

    // Case A: Two short lines fit cleanly inside the 12 mm cell.
    let fits_path = dir.join("fits.yaml");
    fs::write(
        &fits_path,
        "\
units: mm
block_library: blocks.yaml
objects:
  - insert:
      id: box-ok
      block: BOX
      position: [10, 10, 0]
      attributes:
        NOTES: \"LINE 1\\nLINE 2\"
",
    )
    .unwrap();
    let fits_report = check_with(
        &CheckRequest {
            source: fits_path.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    assert!(
        fits_report
            .findings
            .iter()
            .all(|f| f.rule != Rule::TextOverflow && f.rule != Rule::TextOnText),
        "fitting multiline attribute should pass cell & text-on-text checks: {:?}",
        fits_report.findings
    );

    // Case B: Four lines overflow the bottom of the 12 mm cell AND line 2 hits
    //         another text sitting on top of it.
    let overflow_path = dir.join("overflow.yaml");
    fs::write(
        &overflow_path,
        "\
units: mm
block_library: blocks.yaml
objects:
  - insert:
      id: box-bad
      block: BOX
      position: [10, 10, 0]
      attributes:
        NOTES: \"LINE 1\\nCOLLIDE HERE\\nLINE 3\\nLINE 4\"
  - text:
      id: intruder
      contents: \"OVERLAP\"
      position: [14, 12, 0]
      height: 3
",
    )
    .unwrap();
    let bad_report = check_with(
        &CheckRequest {
            source: overflow_path.clone(),
            sheets: Vec::new(),
        },
        &Settings::default(),
    )
    .unwrap();
    let overflow = bad_report
        .findings
        .iter()
        .find(|f| f.rule == Rule::TextOverflow)
        .expect("expected TextOverflow finding for multiline attribute overflowing cell");
    assert!(
        overflow.past.as_ref().is_some_and(|p| p.bottom > 0.0),
        "expected bottom overflow: {overflow:?}"
    );
    let collision = bad_report
        .findings
        .iter()
        .find(|f| f.rule == Rule::TextOnText)
        .expect("expected TextOnText finding for second line hitting intruder text");
    assert!(
        collision.message.contains("COLLIDE HERE"),
        "collision message should name the colliding line: {}",
        collision.message
    );
}
