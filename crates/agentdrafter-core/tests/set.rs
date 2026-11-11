//! A drawing set builds one DWG per row, each its template filled in from
//! the row: the title block's attributes read back from each DWG with the
//! row's values, in the set's unit, in the set's output folder. A row that
//! lacks a value fails its own sheet only, naming the sheet and the value,
//! and two builds of the same set give the same bytes.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, read_dwg, DrawingContents, ErrorCode, NoticeCode, Object, Reply,
    SetBuildRequest, SetReport, Settings, Unit,
};

#[path = "support/demo_set.rs"]
mod demo_set;

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-set")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A copy of the demo plant's drawing set in a folder of its own.
fn demo_set(test: &str) -> PathBuf {
    let set = scratch(test).join("demo-set");
    demo_set::copy_demo_set(&set);
    set
}

fn build(set: &Path, overwrite: bool) -> Reply<SetReport> {
    build_set_with(
        &SetBuildRequest {
            set: set.to_owned(),
            overwrite,
        },
        &Settings::default(),
    )
}

/// Builds the set and returns its report, failing the test unless every
/// sheet was built.
fn build_all(set: &Path) -> SetReport {
    let reply = build(set, false);
    assert!(reply.ok, "{:#?}", reply.error);
    reply.result.unwrap()
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The tags and values of the attributes of the insert named `name`.
fn attributes(read: &DrawingContents, name: &str) -> Vec<(String, String)> {
    read.objects
        .iter()
        .find_map(|o| match o {
            Object::Insert(insert) if insert.name.as_deref() == Some(name) => Some(
                insert
                    .attributes
                    .iter()
                    .map(|a| (a.tag.clone(), a.value.clone()))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_else(|| panic!("an insert named {name} in {:#?}", read.objects))
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

#[test]
fn a_set_with_one_template_builds_one_sheet_per_row_with_the_rows_values_in_its_title_block() {
    let set = demo_set("demo");
    let report = build_all(&set);

    assert_eq!((report.built, report.failed), (3, 0));
    assert_eq!(report.units, Unit::In);
    assert_eq!(
        files_in(&set.join("dwg")),
        ["DEMO-001.dwg", "DEMO-002.dwg", "DEMO-003.dwg"]
    );
    let expected = [
        ("DEMO-001", "AREA 100 GENERAL ARRANGEMENT", "1 OF 3", "0"),
        ("DEMO-002", "AREA 200 GENERAL ARRANGEMENT", "2 OF 3", "0"),
        ("DEMO-003", "AREA 300 GENERAL ARRANGEMENT", "3 OF 3", "A"),
    ];
    for ((sheet, title, of, rev), outcome) in expected.iter().zip(&report.sheets) {
        assert_eq!(outcome.sheet, *sheet);
        assert_eq!(outcome.template, "templates/ansi-b.yaml");
        let result = outcome.result.as_ref().unwrap();
        assert_eq!(result.blocks, ["TITLE_BLOCK"]);
        let dwg = set.join("dwg").join(format!("{sheet}.dwg"));
        assert_eq!(Path::new(&result.dwg), dwg);

        let read = read_dwg(&dwg).unwrap();
        assert_eq!(read.dwg_version, "AC1032");
        assert_eq!(read.units, Some(Unit::In));
        assert_eq!(read.objects_not_read, 0, "{read:#?}");
        assert_eq!(
            attributes(&read, "title-block"),
            pairs(&[
                ("TITLE", title),
                ("DWG_NO", sheet),
                ("SHEET", of),
                ("REV", rev),
                ("DATE", "2026-11-11"),
                ("DRAWN", "AD"),
            ])
        );
        let heading = read.objects.iter().find_map(|o| match o {
            Object::Text(text) => Some(text.contents.as_str()),
            _ => None,
        });
        assert_eq!(heading, Some(*title));
    }
    // The build wrote its DWGs and nothing else.
    assert_eq!(
        files_in(&set),
        ["blocks.yaml", "dwg", "rows.yaml", "set.yaml", "templates"]
    );
}

/// Writes a small set in `dir`: a template with a title block, and the
/// rows `rows`, with the settings `settings`.
fn small_set(dir: &Path, settings: &str, rows: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("set.yaml"), settings).unwrap();
    fs::write(dir.join("rows.yaml"), rows).unwrap();
    fs::write(
        dir.join("blocks.yaml"),
        "units: cm\nblocks:\n  TB:\n    objects:\n      - line: {from: [-10, 0, 0], to: [0, 0, 0]}\n      - attdef: {tag: TITLE, position: [-9, 1, 0], height: 0.5}\n",
    )
    .unwrap();
    fs::write(
        dir.join("sheet.yaml"),
        "units: cm\nblock_library: blocks.yaml\nobjects:\n  - polyline: {points: [[0, 0, 0], [42, 0, 0], [42, 29.7, 0], [0, 29.7, 0]], closed: true}\n  - insert: {block: TB, position: [41, 1, 0], name: tb, attributes: {TITLE: {row: TITLE}}}\n",
    )
    .unwrap();
}

#[test]
fn the_sets_unit_and_output_folder_are_honoured() {
    let root = scratch("unit-and-output");
    let set = root.join("set");
    small_set(
        &set,
        "units: cm\noutput: ../issued/dwg\nrows: rows.yaml\n",
        "rows:\n  - {sheet: S-1, template: sheet.yaml, values: {TITLE: ONE}}\n  - {sheet: S-2, template: sheet.yaml, values: {TITLE: TWO}}\n",
    );

    let report = build_all(&set);

    assert_eq!(report.units, Unit::Cm);
    let output = root.join("issued").join("dwg");
    assert_eq!(Path::new(&report.output), set.join("../issued/dwg"));
    assert_eq!(files_in(&output), ["S-1.dwg", "S-2.dwg"]);
    assert_eq!(
        files_in(&set),
        ["blocks.yaml", "rows.yaml", "set.yaml", "sheet.yaml"]
    );
    for (sheet, title) in [("S-1", "ONE"), ("S-2", "TWO")] {
        let read = read_dwg(&output.join(format!("{sheet}.dwg"))).unwrap();
        assert_eq!(read.units, Some(Unit::Cm));
        assert_eq!(attributes(&read, "tb"), pairs(&[("TITLE", title)]));
    }

    // A template in another unit than the set's fails its sheet, at its units.
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: ../issued/mm\nrows: rows.yaml\n",
    )
    .unwrap();
    let reply = build(&set, false);
    let report = reply.result.unwrap();
    assert_eq!(report.built, 0);
    let error = report.sheets[0].error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::BadValue);
    assert_eq!(error.field.as_deref(), Some("units"));
    assert_eq!(
        error.file,
        Some(set.join("sheet.yaml").display().to_string())
    );
    assert!(error.message.contains("drawing set in mm"), "{error:?}");
    assert!(files_in(&root.join("issued").join("mm")).is_empty());
}

#[test]
fn a_value_the_template_does_not_use_is_reported_and_its_sheet_is_built() {
    let set = demo_set("unused-value");
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    let rows = rows.replace("      REV: A\n", "      REV: A\n      CHECKED: ZM\n");
    fs::write(set.join("rows.yaml"), rows).unwrap();

    let report = build_all(&set);

    assert_eq!((report.built, report.failed), (3, 0));
    assert!(report.sheets[..2].iter().all(|s| s.notices.is_empty()));
    let [notice] = report.sheets[2].notices.as_slice() else {
        panic!("one notice, found {:#?}", report.sheets[2].notices);
    };
    assert_eq!(notice.code, NoticeCode::UnusedValue);
    assert_eq!(notice.sheet, "DEMO-003");
    assert_eq!(notice.template, "templates/ansi-b.yaml");
    assert_eq!(notice.field, "rows[2].values.CHECKED");
    assert_eq!(notice.file, set.join("rows.yaml").display().to_string());
    assert!(notice.message.contains("`CHECKED`"), "{notice:?}");
    assert!(set.join("dwg").join("DEMO-003.dwg").is_file());
}

#[test]
fn a_sheet_whose_name_is_as_long_as_allowed_is_built() {
    // 251 characters, so its DWG's file name is 255, the most systems allow.
    let set = scratch("longest").join("set");
    let name = "L".repeat(251);
    small_set(
        &set,
        "units: cm\noutput: dwg\nrows: rows.yaml\n",
        &format!("rows:\n  - {{sheet: {name}, template: sheet.yaml, values: {{TITLE: LONG}}}}\n"),
    );

    build_all(&set);

    let file = format!("{name}.dwg");
    assert_eq!(file.len(), 255);
    assert_eq!(files_in(&set.join("dwg")), std::slice::from_ref(&file));
    let read = read_dwg(&set.join("dwg").join(file)).unwrap();
    assert_eq!(attributes(&read, "tb"), pairs(&[("TITLE", "LONG")]));
}

#[test]
fn a_row_lacking_a_value_fails_its_own_sheet_naming_the_sheet_and_the_value() {
    let set = demo_set("missing-value");
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    let rows = rows.replace("      TITLE: AREA 200 GENERAL ARRANGEMENT\n", "");
    fs::write(set.join("rows.yaml"), rows).unwrap();

    let reply = build(&set, false);

    assert!(!reply.ok);
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::SheetsFailed);
    assert!(
        error.message.contains("1 of the set's 3 sheets failed"),
        "{error:?}"
    );
    let report = reply.result.unwrap();
    assert_eq!((report.built, report.failed), (2, 1));
    let failed = &report.sheets[1];
    assert_eq!(failed.sheet, "DEMO-002");
    assert!(!failed.ok && failed.result.is_none());
    let error = failed.error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::MissingValue);
    assert_eq!(error.field.as_deref(), Some("rows[1].values.TITLE"));
    assert_eq!(
        error.file,
        Some(set.join("rows.yaml").display().to_string())
    );
    assert!(error.message.contains("sheet `DEMO-002`"), "{error:?}");
    assert!(error.message.contains("no value `TITLE`"), "{error:?}");
    // The failed sheet wrote nothing; the others were built.
    assert_eq!(files_in(&set.join("dwg")), ["DEMO-001.dwg", "DEMO-003.dwg"]);
}

#[test]
fn two_builds_of_the_same_set_give_byte_identical_dwgs() {
    let first = demo_set("twice-first");
    let second = demo_set("twice-second");
    build_all(&first);
    build_all(&second);
    let before: Vec<Vec<u8>> = files_in(&first.join("dwg"))
        .iter()
        .map(|name| fs::read(first.join("dwg").join(name)).unwrap())
        .collect();
    assert_eq!(before.len(), 3);
    for (name, bytes) in files_in(&second.join("dwg")).iter().zip(&before) {
        assert!(
            fs::read(second.join("dwg").join(name)).unwrap() == *bytes,
            "{name} differs between two builds in two folders"
        );
    }

    // Built again over its own DWGs, which carry the build mark.
    let reply = build(&first, true);
    assert!(reply.ok, "{:#?}", reply.error);
    for (name, bytes) in files_in(&first.join("dwg")).iter().zip(&before) {
        assert!(
            fs::read(first.join("dwg").join(name)).unwrap() == *bytes,
            "{name} differs when built again"
        );
    }
}

#[test]
fn a_rebuild_replaces_only_the_dwgs_agentdrafter_built() {
    let set = demo_set("rebuild");
    build_all(&set);
    let original = set.join("dwg").join("DEMO-002.dwg");
    fs::write(&original, b"someone else's drawing").unwrap();

    let reply = build(&set, true);

    let report = reply.result.unwrap();
    assert_eq!((report.built, report.failed), (2, 1));
    let error = report.sheets[1].error.as_ref().unwrap();
    assert_eq!(error.code, ErrorCode::NotABuild);
    assert_eq!(fs::read(&original).unwrap(), b"someone else's drawing");
}

#[test]
fn settings_or_rows_that_cannot_be_read_stop_the_set_before_anything_is_written() {
    let set = demo_set("unreadable");
    fs::write(
        set.join("rows.yaml"),
        "rows:\n  - {sheet: DEMO-001, template: templates/ansi-b.yaml}\n  - {sheet: demo-001, template: templates/ansi-b.yaml}\n",
    )
    .unwrap();
    let reply = build(&set, false);
    assert!(!reply.ok && reply.result.is_none());
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::DuplicateName);
    assert_eq!(error.field.as_deref(), Some("rows[1].sheet"));
    assert_eq!(
        error.file,
        Some(set.join("rows.yaml").display().to_string())
    );
    assert!(!set.join("dwg").exists());

    fs::remove_file(set.join("set.yaml")).unwrap();
    let error = build(&set, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::ReadFailed);
    assert_eq!(error.file, Some(set.join("set.yaml").display().to_string()));
}
