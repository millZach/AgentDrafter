//! Importing a drawing's blocks into a block library: `import-blocks` and
//! the change set's `import_blocks` operation. Every drawing here is made
//! up, built from a made-up library in the test (ADR-0008).

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build, build_set_with, change_with, import_blocks, parse_block_library, read_dwg, undo, Block,
    BlockLibrary, BuildRequest, ChangeReport, ChangeRequest, ErrorCode, ImportBlocksReport,
    ImportBlocksRequest, ImportNoticeCode, Object, Reply, SetBuildRequest, Settings, UndoRequest,
};
use serde::Serialize;
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;

/// A made-up block library holding what a plant's title block and symbols
/// hold: a nested block, a polyline with an arc and widths, hatches, an
/// object's own colour, linetype and lineweight, attribute definitions with
/// every mode, a tag defined twice, and a cell.
const LIBRARY: &str = r"# Made up for AgentDrafter's tests: no plant's block.
units: mm
linetypes:
  DASHED: {description: Dashes, pattern: [5, -2.5]}
layers:
  symbols: {color: yellow, lineweight: 0.35}
  hidden: {color: 8, linetype: DASHED}
  tags: {color: green}
hatch_patterns:
  GRID:
    lines:
      - {delta: [0, 5]}
      - {angle: 90, delta: [0, 5], dashes: [2, -1]}
text_styles:
  tags: {font: romans.shx, width_factor: 0.8}
blocks:
  TERMINAL:
    objects:
      - circle: {center: [0, 0, 0], radius: 4, layer: symbols}
      - attdef: {tag: NUMBER, prompt: Number, default: '0', position: [0, 0, 0], height: 3, style: tags, justify: middle_center, layer: tags}
  STRIP:
    base_point: [5, 0, 0]
    objects:
      - polyline:
          points:
            - [0, 0, 0]
            - {point: [40, 0, 0], bulge: 0.5}
            - {point: [40, 12, 0], start_width: 0.5, end_width: 1}
            - [0, 12, 0]
          closed: true
          layer: symbols
      - hatch: {pattern: GRID, scale: 2, angle: 30, boundary: [{polyline: {points: [[1, 1, 0], [10, 1, 0], [10, 5, 0]]}}], layer: symbols}
      - hatch: {pattern: SOLID, boundary: [{edges: [{line: {from: [20, 1, 0], to: [24, 1, 0]}}, {arc: {center: [22, 1, 0], radius: 2, start_angle: 0, end_angle: 180}}]}], color: red}
      - line: {from: [0, -2, 0], to: [40, -2, 0], layer: hidden, color: 1, linetype: DASHED, lineweight: 0.5, linetype_scale: 0.5}
      - arc: {center: [20, 6, 0], radius: 3, start_angle: 30, end_angle: 200, color: ByBlock}
      - ellipse: {center: [30, 6, 0], major_axis: [4, 0, 0], ratio: 0.5, start_angle: 10, end_angle: 300}
      - spline: {fit_points: [[0, 20, 0], [10, 25, 0], [20, 20, 0]]}
      - text: {contents: STRIP, position: [2, 14, 0], height: 2.5, style: tags, rotation: 15, color: ByBlock}
      - mtext:
          contents: |
            LINE ONE
            LINE TWO
          position: [2, 30, 0]
          height: 2
          width: 30
      - insert: {block: TERMINAL, position: [8, 6, 0], attributes: {NUMBER: '1'}}
      - insert: {block: TERMINAL, position: [32, 6, 0], scale: 0.5, rotation: 90, attributes: {NUMBER: '2'}}
      - attdef: {tag: TAG, prompt: Tag, default: '?', position: [20, 16, 0], height: 2.5, style: tags, justify: middle_center, cell: [[0, 14, 0], [40, 18, 0]]}
      - attdef: {tag: NOTE, position: [0, -6, 0], height: 2, default: '-'}
      - attdef: {tag: NOTE, position: [0, -9, 0], height: 2, default: '-', invisible: true}
      - attdef: {tag: MAKER, position: [0, -12, 0], height: 2, default: ACME, constant: true}
      - attdef: {tag: CHECKED, position: [0, -15, 0], height: 2, verify: true, preset: true, lock_position: true}
";

/// A made-up sheet that inserts the strip, and so its terminals.
const SHEET: &str = "units: mm
block_library: blocks.yaml
objects:
  - insert: {block: STRIP, position: [50, 50, 0], attributes: {TAG: S-1, NOTE: [A, B], CHECKED: 'yes'}}
";

/// An empty folder for one test.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-import")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Builds the made-up `library` and `sheet` in `dir` into `sheet.dwg`.
fn built(dir: &Path, library: &str, sheet: &str) -> PathBuf {
    fs::write(dir.join("blocks.yaml"), library).unwrap();
    fs::write(dir.join("sheet.yaml"), sheet).unwrap();
    build(&BuildRequest {
        description: dir.join("sheet.yaml"),
        output: None,
        overwrite: true,
    })
    .unwrap();
    dir.join("sheet.dwg")
}

fn request(drawing: &Path, library: &Path) -> ImportBlocksRequest {
    ImportBlocksRequest {
        drawing: drawing.to_owned(),
        library: library.to_owned(),
        blocks: Vec::new(),
        replace: false,
        units: None,
        dry_run: false,
    }
}

fn library_at(path: &Path) -> BlockLibrary {
    parse_block_library(&fs::read_to_string(path).unwrap()).unwrap()
}

fn names(report: &ImportBlocksReport) -> (Vec<&str>, Vec<&str>) {
    fn sorted(blocks: &[agentdrafter_core::ImportedBlock]) -> Vec<&str> {
        let mut names: Vec<&str> = blocks.iter().map(|b| b.name.as_str()).collect();
        names.sort_unstable();
        names
    }
    (
        sorted(&report.imported.blocks_added),
        sorted(&report.imported.blocks_replaced),
    )
}

/// Whether `a` and `b` are the same as plain data, numbers to a billionth:
/// a DWG keeps angles in radians, so they come back a little off in
/// degrees, as the reader's own tests allow.
fn same<T: Serialize>(a: &T, b: &T) -> bool {
    fn close(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
                (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0)
            }
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(x, y)| close(x, y))
            }
            (Value::Object(x), Value::Object(y)) => {
                x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| close(v, w)))
            }
            _ => a == b,
        }
    }
    close(
        &serde_json::to_value(a).unwrap(),
        &serde_json::to_value(b).unwrap(),
    )
}

/// The block named `name`, without its cells, which a DWG does not hold.
fn block(library: &BlockLibrary, name: &str) -> Block {
    let block = library.blocks.iter().find(|b| b.name == name).unwrap();
    Block {
        cells: Vec::new(),
        ..block.clone()
    }
}

#[test]
fn a_built_sheet_s_blocks_come_back_through_an_import_as_the_library_defines_them() {
    let dir = scratch("round-trip");
    let dwg = built(&dir, LIBRARY, SHEET);
    let imported = dir.join("imported").join("blocks.yaml");
    fs::create_dir_all(imported.parent().unwrap()).unwrap();

    let report = import_blocks(&request(&dwg, &imported)).unwrap();

    assert!(report.created);
    assert_eq!(report.units, agentdrafter_core::Unit::Mm);
    assert_eq!(names(&report), (vec!["STRIP", "TERMINAL"], vec![]));
    let strip = report
        .imported
        .blocks_added
        .iter()
        .find(|b| b.name == "STRIP")
        .unwrap();
    assert_eq!(strip.nests, ["TERMINAL"]);
    assert_eq!(strip.objects, 16);
    assert!(
        report.imported.not_carried.is_empty(),
        "{:?}",
        report.imported.not_carried
    );
    assert!(
        report.imported.notices.is_empty(),
        "{:?}",
        report.imported.notices
    );
    let tables = &report.imported.tables_added;
    assert_eq!(tables.linetypes, ["DASHED"]);
    let mut layers = tables.layers.clone();
    layers.sort();
    assert_eq!(layers, ["hidden", "symbols", "tags"]);
    assert_eq!(tables.hatch_patterns, ["GRID"]);
    assert_eq!(tables.text_styles, ["tags"]);

    // The library written reads back, and holds the blocks and tables as
    // the original defines them, but for the cell a DWG does not hold.
    let original = parse_block_library(LIBRARY).unwrap();
    let back = library_at(&imported);
    for name in ["TERMINAL", "STRIP"] {
        let (ours, theirs) = (block(&back, name), block(&original, name));
        assert!(same(&ours, &theirs), "{name}:\n{ours:#?}\n{theirs:#?}");
    }
    let sorted = |mut list: Vec<agentdrafter_core::Layer>| {
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    };
    assert_eq!(sorted(back.layers.clone()), sorted(original.layers.clone()));
    assert_eq!(back.linetypes, original.linetypes);
    assert_eq!(back.text_styles, original.text_styles);

    // A sheet built on the imported library draws the same DWG.
    fs::write(dir.join("imported").join("sheet.yaml"), SHEET).unwrap();
    build(&BuildRequest {
        description: dir.join("imported").join("sheet.yaml"),
        output: None,
        overwrite: false,
    })
    .unwrap();
    let first = read_dwg(&dwg).unwrap();
    let again = read_dwg(&dir.join("imported").join("sheet.dwg")).unwrap();
    assert!(same(&first.objects, &again.objects));
    assert_eq!(first.blocks.len(), again.blocks.len());
}

#[test]
fn a_library_that_is_there_keeps_every_byte_and_its_own_definitions() {
    let dir = scratch("existing");
    let dwg = built(&dir, LIBRARY, SHEET);
    let library = dir.join("ours.yaml");
    let ours = "# Our own library, made up: its comments stay.\r\nunits: mm\r\nlayers:\r\n  symbols: {color: red}   # ours is red\r\nblocks:\r\n  OURS:\r\n    objects:\r\n      - circle: {center: [0, 0, 0], radius: 1}\r\n";
    fs::write(&library, ours).unwrap();

    let mut asked = request(&dwg, &library);
    asked.blocks = vec!["terminal".into()];
    let report = import_blocks(&asked).unwrap();

    assert!(!report.created);
    assert_eq!(names(&report), (vec!["TERMINAL"], vec![]));
    let text = fs::read_to_string(&library).unwrap();
    assert!(
        text.starts_with(&ours[..ours.find("blocks:").unwrap()]),
        "{text}"
    );
    assert!(
        text.contains(
            "  OURS:\r\n    objects:\r\n      - circle: {center: [0, 0, 0], radius: 1}\r\n"
        ),
        "{text}"
    );
    assert!(
        !text.contains("\n\n") && !text.replace("\r\n", "").contains('\n'),
        "the file's own line ends: {text:?}"
    );
    let kept = &report.imported.notices;
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].code, ImportNoticeCode::DefinitionKept);
    assert!(
        kept[0].message.contains("layer `symbols`"),
        "{}",
        kept[0].message
    );
    assert_eq!(report.imported.tables_added.layers, ["tags"]);
    let back = library_at(&library);
    assert_eq!(
        back.layers
            .iter()
            .find(|l| l.name == "symbols")
            .unwrap()
            .color,
        1
    );
    assert!(same(
        &block(&back, "TERMINAL"),
        &block(&parse_block_library(LIBRARY).unwrap(), "TERMINAL")
    ));
}

#[test]
fn a_block_the_library_holds_is_refused_unless_replaced() {
    let dir = scratch("duplicate");
    let dwg = built(&dir, LIBRARY, SHEET);
    let library = dir.join("imported.yaml");
    import_blocks(&request(&dwg, &library)).unwrap();
    let before = fs::read(&library).unwrap();

    let error = import_blocks(&request(&dwg, &library)).unwrap_err();
    assert_eq!(error.code, ErrorCode::DuplicateName, "{error:?}");
    assert!(
        error.message.contains("`STRIP`") && error.message.contains("--replace"),
        "{error:?}"
    );
    assert_eq!(fs::read(&library).unwrap(), before, "nothing was written");

    // Asked for alone, its nested block, held the same, is used as it is.
    let mut strip = request(&dwg, &library);
    strip.blocks = vec!["STRIP".into()];
    assert_eq!(
        import_blocks(&strip).unwrap_err().code,
        ErrorCode::DuplicateName
    );
    strip.replace = true;
    let report = import_blocks(&strip).unwrap();
    assert_eq!(names(&report), (vec![], vec!["STRIP"]));
    assert_eq!(library_at(&library).blocks.len(), 2);

    // A nested block the library defines otherwise is refused too.
    let changed = fs::read_to_string(&library)
        .unwrap()
        .replace("radius: 4", "radius: 5");
    fs::write(&library, &changed).unwrap();
    strip.replace = false;
    let error = import_blocks(&ImportBlocksRequest {
        library: library.clone(),
        ..strip.clone()
    })
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::DuplicateName);
    assert!(
        error.message.contains("`TERMINAL`, which `STRIP` nests"),
        "{error:?}"
    );
    assert_eq!(fs::read_to_string(&library).unwrap(), changed);
}

#[test]
fn an_import_refuses_what_it_cannot_do_and_writes_nothing() {
    let dir = scratch("refusals");
    let dwg = built(&dir, LIBRARY, SHEET);
    let library = dir.join("imported.yaml");

    // A library in another unit.
    fs::write(
        &library,
        "units: in\nblocks:\n  A:\n    objects: [{circle: {center: [0, 0, 0], radius: 1}}]\n",
    )
    .unwrap();
    let before = fs::read(&library).unwrap();
    let error = import_blocks(&request(&dwg, &library)).unwrap_err();
    assert_eq!(error.code, ErrorCode::BadValue, "{error:?}");
    assert!(
        error.message.contains("in in") && error.message.contains("in mm"),
        "{error:?}"
    );
    assert_eq!(fs::read(&library).unwrap(), before);

    // A block the drawing does not have, named with its blocks.
    let fresh = dir.join("fresh.yaml");
    let mut asked = request(&dwg, &fresh);
    asked.blocks = vec!["NOPE".into()];
    let error = import_blocks(&asked).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownName, "{error:?}");
    assert!(
        error.message.contains("`NOPE`") && error.message.contains("STRIP"),
        "{error:?}"
    );
    assert!(!fresh.exists());

    // A file that is neither a DWG nor a DXF.
    let error = import_blocks(&request(&dir.join("sheet.yaml"), &fresh)).unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage, "{error:?}");

    // A dry run says what it would write and writes nothing.
    let mut dry = request(&dwg, &fresh);
    dry.dry_run = true;
    let report = import_blocks(&dry).unwrap();
    assert!(report.created && report.dry_run);
    assert_eq!(names(&report).0, ["STRIP", "TERMINAL"]);
    assert!(!fresh.exists());
}

#[test]
fn a_drawing_set_s_library_is_refused_and_named_to_its_operation() {
    let dir = scratch("set-library");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:?}", built.error);
    let before = fs::read(set.join("blocks.yaml")).unwrap();
    let error = import_blocks(&ImportBlocksRequest {
        replace: true,
        ..request(
            &set.join("dwg").join("DEMO-001.dwg"),
            &set.join("blocks.yaml"),
        )
    })
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Usage, "{error:?}");
    assert!(error.message.contains("import_blocks"), "{error:?}");
    assert_eq!(fs::read(set.join("blocks.yaml")).unwrap(), before);
}

fn change(set: &Path, change_set: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set,
            dry_run: false,
        },
        &Settings::default(),
    )
}

#[test]
fn import_blocks_adds_a_block_to_a_set_s_library_which_undo_takes_back_byte_for_byte() {
    let dir = scratch("operation");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    assert!(
        build_set_with(
            &SetBuildRequest {
                set: set.clone(),
                overwrite: false,
            },
            &Settings::default(),
        )
        .ok
    );
    let library = set.join("blocks.yaml");
    let library_before = fs::read(&library).unwrap();
    let dwg_before = fs::read(set.join("dwg").join("DEMO-001.dwg")).unwrap();

    // A made-up badge, drawn in the set's unit, outside the set.
    let parts = dir.join("parts");
    fs::create_dir_all(&parts).unwrap();
    let badge = built(
        &parts,
        "units: in\nblocks:\n  BADGE:\n    objects:\n      - circle: {center: [0, 0, 0], radius: 0.25}\n      - attdef: {tag: CODE, position: [0, 0, 0], height: 0.1, justify: middle_center}\n",
        "units: in\nblock_library: blocks.yaml\nobjects:\n  - insert: {block: BADGE, position: [1, 1, 0], attributes: {CODE: X}}\n",
    );

    let reply = change(
        &set,
        json!({"operations": [{"op": "ADCENTER", "from": badge.display().to_string()}]}),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let report = reply.result.unwrap();
    let imported = report.operations[0].imported.as_ref().unwrap();
    assert_eq!(imported.blocks_added.len(), 1);
    assert_eq!(imported.blocks_added[0].name, "BADGE");
    assert_eq!(report.operations[0].op, "import_blocks");
    assert_eq!(
        report.landed, 3,
        "it reaches every sheet whose template names the library"
    );
    let entry = report.journal_entry.unwrap();
    assert!(library_at(&library)
        .blocks
        .iter()
        .any(|b| b.name == "BADGE"));

    // The block it added is inserted as any other.
    let reply = change(
        &set,
        json!({"operations": [{"op": "insert_block", "sheet": "DEMO-001", "block": "BADGE", "position": [2, 2, 0], "attributes": {"CODE": "B-1"}}]}),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let drawn = read_dwg(&set.join("dwg").join("DEMO-001.dwg")).unwrap();
    assert!(drawn.objects.iter().any(
        |o| matches!(o, Object::Insert(i) if i.block == "BADGE" && i.attributes[0].value == "B-1")
    ));

    // Importing it again is refused, and changes nothing.
    let after = fs::read(&library).unwrap();
    let reply = change(
        &set,
        json!({"operations": [{"op": "import_blocks", "from": badge.display().to_string()}]}),
    );
    assert!(!reply.ok);
    assert_eq!(fs::read(&library).unwrap(), after);

    // Undone, the library and the sheet are as they were, byte for byte.
    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: entry,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(fs::read(&library).unwrap(), library_before);
    assert_eq!(
        fs::read(set.join("dwg").join("DEMO-001.dwg")).unwrap(),
        dwg_before
    );
}

#[test]
fn import_blocks_with_replace_brings_the_inserts_of_a_block_it_replaces_into_line() {
    let dir = scratch("operation-replace");
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    assert!(
        build_set_with(
            &SetBuildRequest {
                set: set.clone(),
                overwrite: false,
            },
            &Settings::default(),
        )
        .ok
    );
    // The set's own title block, read back from one of its sheets.
    fs::create_dir_all(dir.join("parts")).unwrap();
    let title = dir.join("parts").join("DEMO-001.dwg");
    fs::copy(set.join("dwg").join("DEMO-001.dwg"), &title).unwrap();

    let reply = change(
        &set,
        json!({"operations": [{"op": "import_blocks", "from": "../parts/DEMO-001.dwg", "blocks": ["title_block"]}]}),
    );
    assert!(!reply.ok);
    let error = reply.result.unwrap().operations[0].error.clone().unwrap();
    assert_eq!(error.code, ErrorCode::DuplicateName, "{error:?}");
    assert!(error.message.contains("replace"), "{error:?}");

    let report = change(
        &set,
        json!({"operations": [{"op": "import_blocks", "from": "../parts/DEMO-001.dwg", "replace": true}]}),
    );
    assert!(report.ok, "{:#?}", report.error);
    let outcome = &report.result.unwrap().operations[0];
    let imported = outcome.imported.as_ref().unwrap();
    assert_eq!(imported.blocks_replaced.len(), 1);
    assert_eq!(imported.blocks_replaced[0].name, "TITLE_BLOCK");
    assert_eq!(outcome.inserts.len(), 3, "{:?}", outcome.inserts);
    assert!(imported
        .notices
        .iter()
        .any(|n| n.code == ImportNoticeCode::CellsDropped));
}
