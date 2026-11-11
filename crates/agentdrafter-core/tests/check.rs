//! The checker: each default check fails on a made-up sheet built to break
//! it and passes on a clean one, a sheet that declares no paper is told so,
//! the checker reads only what is saved on disk, a change set's reply
//! carries the failed checks for the sheets it landed on, and the demo set
//! passes clean.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, check_with, parse_block_library, parse_sheet_description,
    ChangeRequest, CheckReport, CheckRequest, ErrorCode, Finding, Paper, Rule, SetBuildRequest,
    Settings,
};
use serde_json::json;

#[path = "support/demo_set.rs"]
mod demo_set;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-check")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A copy of the demo set, built when `build` is set.
fn demo_set(test: &str, build: bool) -> PathBuf {
    let set = scratch(test).join("demo-set");
    demo_set::copy_demo_set(&set);
    if build {
        let built = build_set_with(
            &SetBuildRequest {
                set: set.clone(),
                overwrite: false,
            },
            &Settings::default(),
        );
        assert!(built.ok, "{:#?}", built.error);
    }
    set
}

fn check(source: &Path, sheets: &[&str]) -> CheckReport {
    check_with(
        &CheckRequest {
            source: source.to_owned(),
            sheets: sheets.iter().map(|s| (*s).to_owned()).collect(),
        },
        &Settings::default(),
    )
    .unwrap_or_else(|e| panic!("{e:#?}"))
}

/// Writes a made-up sheet description beside a made-up block library and
/// checks it.
fn check_sheet(dir: &Path, description: &str) -> CheckReport {
    fs::write(dir.join("lib.yaml"), LIBRARY).unwrap();
    let path = dir.join("sheet.yaml");
    fs::write(&path, description).unwrap();
    check(&path, &[])
}

/// The findings of one rule.
fn of(report: &CheckReport, rule: Rule) -> Vec<&Finding> {
    report.findings.iter().filter(|f| f.rule == rule).collect()
}

#[track_caller]
fn passes(report: &CheckReport) {
    assert!(report.passed, "{:#?}", report.findings);
    assert_eq!(report.count, 0);
    assert!(
        report.message.starts_with("every check passes"),
        "{}",
        report.message
    );
}

#[track_caller]
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} is not {b}");
}

/// A made-up block library: a box 40 by 10 mm whose tag declares the box
/// as its cell, with a label beside it that declares none.
const LIBRARY: &str = "units: mm
blocks:
  BOX:
    objects:
      - polyline: {points: [[0, 0, 0], [40, 0, 0], [40, 10, 0], [0, 10, 0]], closed: true}
      - attdef: {tag: TAG, position: [20, 5, 0], height: 4, justify: middle_center, cell: [[40, 10, 0], [0, 0, 0]]}
      - text: {contents: A LABEL THAT DECLARES NO CELL, position: [0, 12, 0], height: 3}
      - attdef: {tag: HIDDEN, position: [0, 0, 0], height: 4, invisible: true, cell: [[0, 0, 0], [1, 1, 0]]}
";

/// The trim line of a made-up sheet 200 by 100 mm, from the origin.
const TRIM: &str =
    "  - polyline: {points: [[0, 0, 0], [200, 0, 0], [200, 100, 0], [0, 100, 0]], closed: true}\n";

#[test]
fn the_demo_set_passes_clean() {
    let set = demo_set("demo", false);
    let report = check(&set, &[]);
    passes(&report);
    assert_eq!(report.sheets, ["DEMO-001", "DEMO-002", "DEMO-003"]);
    assert_eq!(report.message, "every check passes on the 3 sheets checked");
    assert_eq!(report.units.symbol(), "in");
    close(report.tolerance, 0.01 / 25.4);
    // The template declares its paper by name, and the title block its cells.
    let template = fs::read_to_string(set.join("templates/ansi-b.yaml")).unwrap();
    assert!(template.contains("paper: ANSI B landscape"));
    let library = parse_block_library(&fs::read_to_string(set.join("blocks.yaml")).unwrap())
        .unwrap()
        .blocks
        .remove(0);
    assert_eq!(library.cells.len(), 12, "every text and attribute");
}

#[test]
fn the_exact_paper_size_fails_on_a_sheet_that_is_not_its_papers_size() {
    let dir = scratch("paper-size");
    // ISO A4 landscape is 297 by 210 mm; this sheet draws 290 by 210.
    let narrow = check_sheet(
        &dir,
        "units: mm
paper: ISO A4 landscape
objects:
  - polyline: {points: [[0, 0, 0], [290, 0, 0], [290, 210, 0], [0, 210, 0]], closed: true}
",
    );
    assert_eq!(narrow.count, 1, "{:#?}", narrow.findings);
    let finding = &narrow.findings[0];
    assert_eq!(finding.sheet, "sheet");
    assert_eq!(finding.rule, Rule::PaperSize);
    assert!(finding.object.is_none(), "a check of the whole sheet");
    let off_by = finding.off_by.unwrap();
    close(off_by[0], -7.0);
    close(off_by[1], 0.0);
    assert_eq!(
        finding.paper,
        Some(Paper {
            name: Some("ISO A4 landscape".into()),
            width: 297.0,
            height: 210.0
        })
    );
    assert_eq!(
        finding.message,
        "sheet `sheet` draws 290 by 210 mm, from [0, 0] to [290, 210], and its paper, ISO A4 \
         landscape, is 297 by 210 mm: it is 7 mm too narrow"
    );

    let exact = check_sheet(
        &dir,
        "units: mm
paper: ISO A4 landscape
objects:
  - polyline: {points: [[0, 0, 0], [297, 0, 0], [297, 210, 0], [0, 210, 0]], closed: true}
",
    );
    passes(&exact);

    // Within the tolerance, 0.01 mm on paper, a sheet is its paper's size.
    let near = check_sheet(
        &dir,
        "units: mm
paper: [297, 210]
objects:
  - polyline: {points: [[0, 0, 0], [297.005, 0, 0], [297.005, 210, 0], [0, 210, 0]], closed: true}
",
    );
    passes(&near);

    // A sheet that draws nothing is not its paper's size.
    let empty = check_sheet(&dir, "units: mm\npaper: [297, 210]\nobjects: []\n");
    assert_eq!(empty.count, 1);
    assert_eq!(empty.findings[0].rule, Rule::PaperSize);
    assert!(empty.findings[0].message.contains("draws nothing"));
}

#[test]
fn nothing_off_the_sheet_fails_on_an_object_past_the_paper() {
    let dir = scratch("off-sheet");
    // The sheet is its paper's size, 200 by 100 mm, but drawn 2 mm left of
    // the origin, where the paper's lower left corner is.
    let shifted = check_sheet(
        &dir,
        "units: mm
paper: [200, 100]
objects:
  - polyline: {points: [[-2, 0, 0], [198, 0, 0], [198, 100, 0], [-2, 100, 0]], closed: true, id: trim, name: edge}
  - circle: {center: [100, 50, 0], radius: 10}
",
    );
    assert_eq!(shifted.count, 1, "{:#?}", shifted.findings);
    let finding = &shifted.findings[0];
    assert_eq!(finding.rule, Rule::OffSheet);
    assert_eq!(finding.sheet, "sheet");
    let object = finding.object.as_ref().unwrap();
    assert_eq!(object.id, "sheet/trim");
    assert_eq!(object.name.as_deref(), Some("edge"));
    assert_eq!(object.kind, "polyline");
    assert_eq!(object.written_at.field, "objects[0]");
    assert_eq!(object.written_at.line, 4);
    let past = finding.past.unwrap();
    close(past.left, 2.0);
    close(past.right, 0.0);
    assert_eq!(
        finding.message,
        "the polyline `sheet/trim` (named `edge`) draws 2 mm past the left edge of the paper: it \
         draws from [-2, 0] to [198, 100], and the paper runs from [0, 0] to [200, 100] mm"
    );

    // Text off the sheet, and a circle past two edges, each found.
    let past_two = check_sheet(
        &dir,
        &format!(
            "units: mm
paper: [200, 100]
objects:
{TRIM}  - circle: {{center: [199, 99, 0], radius: 5, id: corner}}
  - text: {{contents: RUNS OFF, position: [190, 50, 0], height: 5, id: note}}
"
        ),
    );
    let off: Vec<&str> = of(&past_two, Rule::OffSheet)
        .iter()
        .map(|f| f.object.as_ref().unwrap().id.as_str())
        .collect();
    assert_eq!(off, ["sheet/corner", "sheet/note"]);
    // A curve is drawn as chords within a millionth of the sheet's larger
    // side, about 0.0002 mm here, so how far it reaches is found to within
    // about that.
    let corner = of(&past_two, Rule::OffSheet)[0].past.unwrap();
    assert!((corner.right - 4.0).abs() < 1e-3, "{corner:?}");
    assert!((corner.top - 4.0).abs() < 1e-3, "{corner:?}");
    assert_eq!(
        of(&past_two, Rule::PaperSize).len(),
        1,
        "it is too large too"
    );

    let clean = check_sheet(
        &dir,
        &format!(
            "units: mm
paper: [200, 100]
objects:
{TRIM}  - circle: {{center: [100, 50, 0], radius: 10}}
  - text: {{contents: STAYS ON, position: [150, 50, 0], height: 5}}
"
        ),
    );
    passes(&clean);
}

#[test]
fn no_text_overflowing_its_cell_fails_on_an_attribute_too_long_for_it() {
    let dir = scratch("overflow");
    let sheet = |value: &str, rotation: f64, scale: f64| {
        format!(
            "units: mm
paper: [200, 100]
block_library: lib.yaml
objects:
{TRIM}  - insert: {{block: BOX, position: [20, 20, 0], name: box, attributes: {{TAG: '{value}'}}}}
  - insert: {{block: BOX, position: [150, 30, 0], rotation: {rotation}, scale: {scale}, attributes: {{TAG: '{value}'}}}}
"
        )
    };
    let clean = check_sheet(&dir, &sheet("A-1", 90.0, 0.5));
    passes(&clean);

    let long = check_sheet(&dir, &sheet("A TAG FAR TOO LONG FOR ITS BOX", 90.0, 0.5));
    let overflow = of(&long, Rule::TextOverflow);
    assert_eq!(overflow.len(), 2, "{:#?}", long.findings);
    let first = overflow[0];
    let object = first.object.as_ref().unwrap();
    assert_eq!(object.id, "sheet/objects[1]");
    assert_eq!(object.name.as_deref(), Some("box"));
    assert_eq!(object.kind, "insert");
    let part = object.in_block.as_ref().unwrap();
    assert_eq!(part.block, "BOX");
    assert_eq!(part.field, "blocks.BOX.objects[1]");
    assert_eq!(part.kind, "attdef");
    assert_eq!(part.tag.as_deref(), Some("TAG"));
    assert!(part.file.ends_with("lib.yaml"), "{}", part.file);
    assert_eq!(
        object.text.as_deref(),
        Some("A TAG FAR TOO LONG FOR ITS BOX")
    );
    let cell = first.cell.unwrap();
    assert_eq!((cell.min, cell.max), ([0.0, 0.0], [40.0, 10.0]));
    let past = first.past.unwrap();
    assert!(past.left > 1.0 && past.right > 1.0, "{past:?}");
    close(past.left, past.right);
    close(past.top, 0.0);
    // The cell is placed by the insert: turned a quarter and halved, the
    // second insert's text overflows the same cell, in the block's own
    // coordinates, by the same amount.
    let turned = overflow[1];
    assert_eq!(turned.object.as_ref().unwrap().id, "sheet/objects[2]");
    let turned_past = turned.past.unwrap();
    close(turned_past.left, past.left);
    assert!(
        turned.message.contains("the insert's scale of 0.5"),
        "{}",
        turned.message
    );
    assert!(
        first.message.starts_with(
            "the attribute TAG of the insert `sheet/objects[1]` (named `box`) saying `A TAG FAR \
             TOO LONG FOR ITS BOX` draws "
        ),
        "{}",
        first.message
    );

    // The label declares no cell and is not checked for overflow, and the
    // hidden attribute draws nothing to check.
    assert!(overflow.iter().all(
        |f| f.object.as_ref().unwrap().in_block.as_ref().unwrap().field == "blocks.BOX.objects[1]"
    ));
}

#[test]
fn a_text_that_declares_its_cell_in_a_block_is_held_to_it() {
    let dir = scratch("text-cell");
    fs::write(
        dir.join("lib.yaml"),
        "units: mm
blocks:
  TB:
    objects:
      - text: {contents: DEMO PLANT AND MORE, position: [0, 0, 0], height: 4, cell: [[0, -1, 0], [20, 5, 0]]}
",
    )
    .unwrap();
    let path = dir.join("sheet.yaml");
    fs::write(
        &path,
        format!(
            "units: mm\npaper: [200, 100]\nblock_library: lib.yaml\nobjects:\n{TRIM}  - insert: {{block: TB, position: [10, 10, 0]}}\n"
        ),
    )
    .unwrap();
    let report = check(&path, &[]);
    let overflow = of(&report, Rule::TextOverflow);
    assert_eq!(overflow.len(), 1, "{:#?}", report.findings);
    let part = overflow[0]
        .object
        .as_ref()
        .unwrap()
        .in_block
        .as_ref()
        .unwrap();
    assert_eq!(part.kind, "text");
    assert_eq!(part.tag, None);
    assert!(overflow[0].past.unwrap().right > 10.0);
    assert_eq!(overflow[0].past.unwrap().left, 0.0);
}

#[test]
fn no_text_sitting_on_other_text_fails_on_two_texts_whose_boxes_overlap() {
    let dir = scratch("text-on-text");
    let sheet = |second: &str| {
        format!(
            "units: mm
paper: [200, 100]
block_library: lib.yaml
objects:
{TRIM}  - text: {{contents: FIRST NOTE, position: [10, 10, 0], height: 5, id: first}}
  - text: {{contents: SECOND NOTE, position: {second}, height: 5, id: second, name: second-note}}
"
        )
    };
    let clean = check_sheet(&dir, &sheet("[10, 20, 0]"));
    passes(&clean);

    let on = check_sheet(&dir, &sheet("[12, 12, 0]"));
    assert_eq!(on.count, 1, "{:#?}", on.findings);
    let finding = &on.findings[0];
    assert_eq!(finding.rule, Rule::TextOnText);
    let object = finding.object.as_ref().unwrap();
    assert_eq!(object.id, "sheet/second");
    assert_eq!(object.name.as_deref(), Some("second-note"));
    assert_eq!(object.text.as_deref(), Some("SECOND NOTE"));
    assert_eq!(finding.other.as_ref().unwrap().id, "sheet/first");
    // The boxes overlap 3 mm up and down, from 12 to 15, and further across.
    close(finding.overlap.unwrap(), 3.0);
    assert!(finding.drawn.is_some() && finding.other_drawn.is_some());
    assert!(
        finding.message.starts_with(
            "the text `sheet/second` (named `second-note`) saying `SECOND NOTE` sits on the text \
             `sheet/first` saying `FIRST NOTE`: their drawn boxes overlap by 3 mm"
        ),
        "{}",
        finding.message
    );

    // Turned 45 degrees, two lines stacked along their own up direction do
    // not sit on each other, though the boxes square to the sheet overlap.
    let up = 7.0 / 2f64.sqrt();
    let turned = check_sheet(
        &dir,
        &format!(
            "units: mm
paper: [200, 100]
objects:
{TRIM}  - text: {{contents: ABCDEFGHIJ, position: [50, 50, 0], height: 5, rotation: 45}}
  - text: {{contents: ABCDEFGHIJ, position: [{}, {}, 0], height: 5, rotation: 45}}
",
            50.0 - up,
            50.0 + up
        ),
    );
    passes(&turned);

    // Text an insert draws counts: a sheet's text on an insert's attribute.
    let on_insert = check_sheet(
        &dir,
        &format!(
            "units: mm
paper: [200, 100]
block_library: lib.yaml
objects:
{TRIM}  - insert: {{block: BOX, position: [20, 20, 0], name: box, attributes: {{TAG: A-1}}}}
  - text: {{contents: OVER THE TAG, position: [35, 23, 0], height: 4, id: over}}
"
        ),
    );
    let on: Vec<&Finding> = of(&on_insert, Rule::TextOnText);
    assert_eq!(on.len(), 1, "{:#?}", on_insert.findings);
    assert_eq!(on[0].object.as_ref().unwrap().id, "sheet/over");
    let other = on[0].other.as_ref().unwrap();
    assert_eq!(other.id, "sheet/objects[1]");
    assert_eq!(other.in_block.as_ref().unwrap().tag.as_deref(), Some("TAG"));
    assert_eq!(other.text.as_deref(), Some("A-1"));
}

#[test]
fn a_sheet_with_no_paper_declared_gets_one_finding_that_says_so() {
    let dir = scratch("no-paper");
    // Nothing else about the paper is checked: the line far away is not
    // reported as off a paper the sheet does not declare.
    let report = check_sheet(
        &dir,
        "units: mm
objects:
  - polyline: {points: [[0, 0, 0], [200, 0, 0], [200, 100, 0], [0, 100, 0]], closed: true}
  - line: {from: [999, 999, 0], to: [1001, 1001, 0]}
",
    );
    assert_eq!(report.count, 1, "{:#?}", report.findings);
    let finding = &report.findings[0];
    assert_eq!(finding.sheet, "sheet");
    assert_eq!(finding.rule, Rule::PaperSize);
    assert!(finding.object.is_none());
    assert!(!report.passed);
    assert_eq!(
        finding.message,
        "sheet `sheet` declares no paper, so neither its size nor what is off it is checked; it \
         draws 1001 by 1001 mm, from [0, 0] to [1001, 1001]. Declare its paper under `paper` in \
         its sheet description, such as `paper: [1001, 1001]`"
    );
    let drawn = finding.drawn.unwrap();
    assert_eq!((drawn.width, drawn.height), (1001.0, 1001.0));
}

#[test]
fn a_paper_is_read_by_its_size_or_by_name_in_the_declared_unit() {
    let read = |paper: &str, units: &str| {
        parse_sheet_description(&format!("units: {units}\npaper: {paper}\nobjects: []\n"))
            .map(|sheet| sheet.paper.unwrap())
    };
    let named = read("ISO A3 landscape", "in").unwrap();
    assert_eq!(named.name.as_deref(), Some("ISO A3 landscape"));
    close(named.width, 420.0 / 25.4);
    close(named.height, 297.0 / 25.4);
    let portrait = read("ANSI B portrait", "mm").unwrap();
    close(portrait.width, 279.4);
    close(portrait.height, 431.8);
    let sized = read("[17 in, 279.4 mm]", "in").unwrap();
    assert_eq!(sized.name, None);
    close(sized.width, 17.0);
    close(sized.height, 11.0);

    let error = read("ANSI B", "in").unwrap_err();
    assert_eq!(error.code, ErrorCode::BadValue);
    assert_eq!(error.field.as_deref(), Some("paper"));
    assert!(
        error
            .message
            .contains("`ANSI B landscape`, 17 by 11 in, or `ANSI B portrait`, 11 by 17 in"),
        "{}",
        error.message
    );
    let error = read("Tabloid", "in").unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownName);
    assert!(
        error.message.contains("ISO A4 portrait"),
        "{}",
        error.message
    );
    let error = read("[17, 0]", "in").unwrap_err();
    assert_eq!(error.code, ErrorCode::BadValue);
    assert_eq!(error.field.as_deref(), Some("paper[1]"));
    let error = read("[17, 11, 0]", "in").unwrap_err();
    assert_eq!(error.code, ErrorCode::WrongType);
}

#[test]
fn a_cell_is_read_in_a_block_and_refused_on_a_sheet() {
    let library = parse_block_library(LIBRARY).unwrap();
    let block = &library.blocks[0];
    assert_eq!(block.cells.len(), 2);
    assert_eq!(block.cells[0].object, 1);
    assert_eq!(block.cells[0].min, [0.0, 0.0]);
    assert_eq!(block.cells[0].max, [40.0, 10.0]);
    assert_eq!(block.cells[1].object, 3);

    let flat = parse_block_library(
        "units: mm\nblocks:\n  B: {objects: [{attdef: {tag: T, position: [0, 0, 0], height: 1, cell: [[0, 0, 0], [5, 0, 0]]}}]}\n",
    )
    .unwrap_err();
    assert_eq!(flat.code, ErrorCode::BadValue);
    assert_eq!(
        flat.field.as_deref(),
        Some("blocks.B.objects[0].attdef.cell")
    );

    let on_a_sheet = parse_sheet_description(
        "units: mm\nobjects:\n  - text: {contents: A, position: [0, 0, 0], height: 1, cell: [[0, 0, 0], [5, 5, 0]]}\n",
    )
    .unwrap_err();
    assert_eq!(on_a_sheet.code, ErrorCode::UnknownKey);
}

#[test]
fn the_checker_reads_only_what_is_saved_on_disk() {
    let set = demo_set("disk", true);
    passes(&check(&set, &[]));
    let dwg = set.join("dwg/DEMO-002.dwg");
    let built = fs::read(&dwg).unwrap();

    // A row's value changed on disk, behind every other part's back: the
    // finding follows the file, and the DWG, which still holds the old
    // value, is not what the checker reads.
    let rows_path = set.join("rows.yaml");
    let rows = fs::read_to_string(&rows_path).unwrap();
    let long = "AREA 200 GENERAL ARRANGEMENT OF THE NORTH BUILDING";
    fs::write(
        &rows_path,
        rows.replacen("AREA 200 GENERAL ARRANGEMENT", long, 1),
    )
    .unwrap();
    let report = check(&set, &[]);
    assert_eq!(report.count, 1, "{:#?}", report.findings);
    let finding = &report.findings[0];
    assert_eq!(finding.sheet, "DEMO-002");
    assert_eq!(finding.rule, Rule::TextOverflow);
    let object = finding.object.as_ref().unwrap();
    assert_eq!(object.name.as_deref(), Some("title-block"));
    assert_eq!(object.text.as_deref(), Some(long));
    assert_eq!(fs::read(&dwg).unwrap(), built, "the checker writes nothing");

    // The template moved on disk: the heading now runs off the sheet.
    let template_path = set.join("templates/ansi-b.yaml");
    let template = fs::read_to_string(&template_path).unwrap();
    fs::write(
        &template_path,
        template.replace("position: [1, 10, 0]", "position: [12, 10, 0]"),
    )
    .unwrap();
    let report = check(&set, &["demo-001"]);
    assert_eq!(report.sheets, ["DEMO-001"]);
    let off: Vec<&str> = of(&report, Rule::OffSheet)
        .iter()
        .map(|f| f.object.as_ref().unwrap().id.as_str())
        .collect();
    assert_eq!(off, ["DEMO-001/heading"]);

    // Put back, the set passes again.
    fs::write(&rows_path, rows).unwrap();
    fs::write(&template_path, template).unwrap();
    passes(&check(&set, &[]));

    // A sheet the set does not hold is an error that lists its sheets.
    let error = check_with(
        &CheckRequest {
            source: set.clone(),
            sheets: vec!["DEMO-009".into()],
        },
        &Settings::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownName);
    assert!(error.message.contains("DEMO-001, DEMO-002, DEMO-003"));
}

#[test]
fn a_change_sets_reply_carries_the_failed_checks_for_the_sheets_it_landed_on() {
    let set = demo_set("change", true);
    let change_set = json!({"operations": [
        {"op": "set_attribute", "target": {"name": "title-block", "sheet": "DEMO-001"}, "tag": "TITLE",
         "value": "AREA 100 GENERAL ARRANGEMENT OF THE NORTH BUILDING"},
        {"op": "add_text", "sheet": "DEMO-002", "contents": "OFF THE EDGE", "position": [16.5, 5, 0],
         "height": 0.2, "style": "notes", "layer": "notes", "id": "edge"},
        {"op": "add_circle", "sheet": "DEMO-003", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
    ]});
    let apply = |dry_run: bool| {
        change_with(
            &ChangeRequest {
                set: set.clone(),
                change_set: change_set.clone(),
                dry_run,
            },
            &Settings::default(),
        )
    };

    // A dry run saves nothing, so there is nothing for the checker to read.
    let dry = apply(true).result.unwrap();
    assert!(dry.checks.is_none());

    let reply = apply(false);
    assert_eq!(reply.error.as_ref().unwrap().code, ErrorCode::SheetsFailed);
    let report = reply.result.unwrap();
    assert_eq!((report.landed, report.failed), (2, 1));
    let checks = report.checks.expect("the reply carries the checks");
    assert!(checks.ok, "{:#?}", checks.error);
    let checked = checks.result.unwrap();
    // The sheets it landed on; DEMO-003 failed and was left as it was.
    assert_eq!(checked.sheets, ["DEMO-001", "DEMO-002"]);
    assert!(!checked.passed);
    let found: Vec<(&str, Rule, String)> = checked
        .findings
        .iter()
        .map(|f| {
            let object = f.object.as_ref().map_or(String::new(), |o| o.id.clone());
            (f.sheet.as_str(), f.rule, object)
        })
        .collect();
    assert_eq!(
        found,
        [
            ("DEMO-001", Rule::TextOverflow, "DEMO-001/objects[2]".into()),
            ("DEMO-002", Rule::PaperSize, String::new()),
            ("DEMO-002", Rule::OffSheet, "DEMO-002/edge".into()),
        ]
    );
    let overflow = &checked.findings[0];
    let part = overflow.object.as_ref().unwrap().in_block.as_ref().unwrap();
    assert_eq!(part.tag.as_deref(), Some("TITLE"));

    // It is what the checker reads from the saved files.
    assert_eq!(checked, check(&set, &["DEMO-001", "DEMO-002"]));

    // In JSON, each finding names the sheet, the rule and the object.
    let written = serde_json::to_value(&checked.findings[2]).unwrap();
    assert_eq!(written["sheet"], "DEMO-002");
    assert_eq!(written["rule"], "off_sheet");
    assert_eq!(written["object"]["id"], "DEMO-002/edge");
    assert_eq!(written["object"]["type"], "text");
    assert_eq!(written["object"]["field"], "rows[1].objects[0]");
    assert!(written["past"]["right"].as_f64().unwrap() > 0.0);
}

#[test]
fn text_lying_wholly_on_other_text_is_found_though_its_box_has_no_width() {
    let dir = scratch("no-width");
    // `I` draws one upright stroke, so the box round it has no width.
    let sheet = |second: &str, contents: &str| {
        format!(
            "units: mm
paper: [200, 100]
objects:
{TRIM}  - text: {{contents: {contents}, position: [30, 30, 0], height: 5, id: first}}
  - text: {{contents: I, position: {second}, height: 5, id: second}}
"
        )
    };
    let twins = check_sheet(&dir, &sheet("[30, 30, 0]", "I"));
    assert_eq!(twins.count, 1, "{:#?}", twins.findings);
    let finding = &twins.findings[0];
    assert_eq!(finding.rule, Rule::TextOnText);
    assert_eq!(finding.object.as_ref().unwrap().id, "sheet/second");
    assert_eq!(finding.other.as_ref().unwrap().id, "sheet/first");
    assert_eq!(finding.drawn.unwrap().width, 0.0, "a box with no width");
    close(finding.overlap.unwrap(), 5.0);

    // An `I` in the middle of a wider word sits on it too.
    let inside = check_sheet(&dir, &sheet("[40, 30, 0]", "HHHHHH"));
    assert_eq!(
        of(&inside, Rule::TextOnText).len(),
        1,
        "{:#?}",
        inside.findings
    );

    // Two `I`s side by side are apart.
    passes(&check_sheet(&dir, &sheet("[32, 30, 0]", "I")));
}

#[test]
fn multiline_text_is_held_to_each_line_it_draws() {
    let dir = scratch("mtext-lines");
    let sheet = |note: &str| {
        format!(
            "units: mm
paper: [200, 100]
objects:
{TRIM}  - mtext: {{contents: \"LONG FIRST LINE\\nX\", position: [10, 70, 0], height: 5, id: note}}
  - text: {{contents: CLEAR, position: {note}, height: 3, id: clear}}
"
        )
    };
    // Beside the short second line and below the long first one: clear of
    // both, though inside the box round the two together.
    passes(&check_sheet(&dir, &sheet("[30, 57, 0]")));

    // On the first line, it sits on it, and the finding says which line.
    let on = check_sheet(&dir, &sheet("[30, 66, 0]"));
    let found = of(&on, Rule::TextOnText);
    assert_eq!(found.len(), 1, "{:#?}", on.findings);
    assert_eq!(found[0].object.as_ref().unwrap().id, "sheet/clear");
    assert_eq!(found[0].other.as_ref().unwrap().id, "sheet/note");
    assert!(
        found[0]
            .message
            .contains("sits on the mtext `sheet/note` on its line `LONG FIRST LINE`"),
        "{}",
        found[0].message
    );
}

#[test]
fn a_hatch_whose_boundary_runs_off_the_paper_is_off_the_sheet() {
    let dir = scratch("hatch-boundary");
    // The pattern's upright lines are 10 mm apart, so only the one at
    // X = 0 falls inside the boundary from X = -1 to 1: what the hatch
    // draws stays on the paper, but its boundary reaches 1 mm past it, and
    // the sheet still draws 200 by 100 mm.
    let report = check_sheet(
        &dir,
        "units: mm
paper: [200, 100]
hatch_patterns:
  RULE: {lines: [{angle: 90, delta: [0, 10]}]}
objects:
  - polyline: {points: [[0, 0, 0], [199, 0, 0], [199, 100, 0], [0, 100, 0]], closed: true}
  - hatch: {pattern: RULE, boundary: [{polyline: {points: [[-1, 10, 0], [1, 10, 0], [1, 20, 0], [-1, 20, 0]]}}], id: fill}
",
    );
    assert_eq!(report.count, 1, "{:#?}", report.findings);
    let finding = &report.findings[0];
    assert_eq!(finding.rule, Rule::OffSheet);
    let object = finding.object.as_ref().unwrap();
    assert_eq!(object.id, "sheet/fill");
    assert_eq!(object.kind, "hatch");
    close(finding.past.unwrap().left, 1.0);
}
