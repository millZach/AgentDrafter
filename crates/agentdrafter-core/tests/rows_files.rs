//! A drawing set whose rows are in more than one rows file (#103): the
//! two-kinds example lists two, and a folder of them gives the same set.
//! The set builds each file's rows in turn, in the order given; a folder's
//! files go in the byte order of their names. A sheet is named once across
//! the files. Each error and each ID taken from a row's place names the
//! row's own file, a change set writes each edit to the file its row is in,
//! and an undo puts every file it touched back byte for byte.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, plot_set_with, query_with, undo, Answer, ChangeRequest, ErrorCode,
    PlotSetRequest, Query, QueryRequest, Reply, Selection, SetBuildRequest, SetReport, Settings,
    SourceKind, UndoRequest,
};
use serde_json::json;

#[path = "support/snapshot.rs"]
mod snapshot;

use snapshot::snapshot;

/// A copy of `examples/two-kinds`, its YAML files only, in a folder of its
/// own under Cargo's scratch folder in `target/`.
fn two_kinds(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-rows-files")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("two-kinds");
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/two-kinds");
    copy_yaml(&from, &set);
    set
}

fn copy_yaml(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_yaml(&path, &target);
        } else if path.extension().is_some_and(|e| e == "yaml") {
            fs::create_dir_all(to).unwrap();
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// Writes the set's settings with `rows` as given, in YAML.
fn rows_are(set: &Path, rows: &str) {
    fs::write(
        set.join("set.yaml"),
        format!("units: in\noutput: dwg\nrows: {rows}\n"),
    )
    .unwrap();
}

fn build(set: &Path) -> Reply<SetReport> {
    build_set_with(
        &SetBuildRequest {
            set: set.to_owned(),
            overwrite: true,
        },
        &Settings::default(),
    )
}

fn built(set: &Path) -> SetReport {
    let reply = build(set);
    assert!(reply.ok, "{:#?}", reply.error);
    reply.result.unwrap()
}

fn sheets(report: &SetReport) -> Vec<&str> {
    report.sheets.iter().map(|s| s.sheet.as_str()).collect()
}

/// The error a set's build stops with, before any sheet is built.
#[track_caller]
fn refused(set: &Path) -> agentdrafter_core::Error {
    let reply = build(set);
    assert!(reply.result.is_none(), "{:#?}", reply.result);
    reply.error.unwrap()
}

/// Where a path the core gives back leads, relative to the set's folder,
/// with `/` between folders.
fn in_set(set: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(set)
        .unwrap_or_else(|_| panic!("{path} is in {}", set.display()))
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn a_list_of_rows_files_builds_each_files_rows_in_turn() {
    let set = two_kinds("list");
    let report = built(&set);
    assert_eq!(sheets(&report), ["AREA-101", "AREA-102", "SCHED-201"]);
    let templates: Vec<&str> = report.sheets.iter().map(|s| s.template.as_str()).collect();
    assert_eq!(
        templates,
        [
            "templates/area.yaml",
            "templates/area.yaml",
            "templates/schedule.yaml"
        ]
    );
    for sheet in ["AREA-101", "AREA-102", "SCHED-201"] {
        assert!(set.join("dwg").join(format!("{sheet}.dwg")).is_file());
    }

    // The list's order is the build's.
    rows_are(&set, "[rows/schedules.yaml, rows/areas.yaml]");
    assert_eq!(sheets(&built(&set)), ["SCHED-201", "AREA-101", "AREA-102"]);
}

#[test]
fn a_folder_of_rows_files_builds_the_same_set_as_the_list_of_them() {
    let set = two_kinds("folder");
    built(&set);
    let listed: Vec<Vec<u8>> = ["AREA-101", "AREA-102", "SCHED-201"]
        .iter()
        .map(|s| fs::read(set.join("dwg").join(format!("{s}.dwg"))).unwrap())
        .collect();

    rows_are(&set, "rows");
    let report = built(&set);
    assert_eq!(sheets(&report), ["AREA-101", "AREA-102", "SCHED-201"]);
    for (sheet, bytes) in ["AREA-101", "AREA-102", "SCHED-201"].iter().zip(&listed) {
        let again = fs::read(set.join("dwg").join(format!("{sheet}.dwg"))).unwrap();
        assert!(again == *bytes, "{sheet} is built the same from a folder");
    }

    // A folder and a list mixed.
    fs::create_dir_all(set.join("more")).unwrap();
    fs::write(
        set.join("more/extra.yaml"),
        "rows:\n  - {sheet: EXTRA-1, template: templates/schedule.yaml, values: {TITLE: EXTRA, DWG_NO: EXTRA-1, REV: '0', COVERS: NONE}}\n",
    )
    .unwrap();
    rows_are(&set, "[more, rows/schedules.yaml]");
    assert_eq!(sheets(&built(&set)), ["EXTRA-1", "SCHED-201"]);
}

#[test]
fn a_folder_gives_its_yaml_files_in_the_byte_order_of_their_names_and_no_other() {
    let set = two_kinds("order");
    let rows = set.join("by-name");
    fs::create_dir_all(rows.join("inner")).unwrap();
    let one = |sheet: &str| {
        format!("rows:\n  - {{sheet: {sheet}, template: templates/schedule.yaml, values: {{TITLE: T, DWG_NO: {sheet}, REV: '0', COVERS: C}}}}\n")
    };
    // `Z` is before `a` and `-` before `_`, in bytes.
    fs::write(rows.join("a_1.yaml"), one("THIRD")).unwrap();
    fs::write(rows.join("a-2.yaml"), one("SECOND")).unwrap();
    fs::write(rows.join("Z.yaml"), one("FIRST")).unwrap();
    // Not rows files of the folder: another extension, and a folder's own.
    fs::write(rows.join("notes.txt"), "not rows").unwrap();
    fs::write(rows.join("a.yml"), "not rows").unwrap();
    fs::write(rows.join("inner/deeper.yaml"), one("DEEPER")).unwrap();
    rows_are(&set, "by-name");
    assert_eq!(sheets(&built(&set)), ["FIRST", "SECOND", "THIRD"]);
}

#[test]
fn rows_that_name_no_file_a_file_twice_or_a_folder_outside_the_set_are_refused() {
    let set = two_kinds("refused");
    let settings = set.join("set.yaml").display().to_string();

    rows_are(&set, "[]");
    let e = refused(&set);
    assert_eq!(e.code, ErrorCode::BadValue, "{e:?}");
    assert_eq!(e.field.as_deref(), Some("rows"));
    assert_eq!(e.file.as_deref(), Some(settings.as_str()));

    fs::create_dir_all(set.join("empty")).unwrap();
    fs::write(set.join("empty/readme.txt"), "no rows here").unwrap();
    rows_are(&set, "[rows/areas.yaml, empty]");
    let e = refused(&set);
    assert_eq!(e.code, ErrorCode::BadValue, "{e:?}");
    assert_eq!(e.field.as_deref(), Some("rows[1]"));
    assert!(e.message.contains("holds no rows file"), "{e:?}");
    assert_eq!(e.file.as_deref(), Some(settings.as_str()));

    for (rows, says) in [
        (
            "[rows/areas.yaml, rows/areas.yaml]",
            "by `rows[0]` and by `rows[1]`",
        ),
        (
            "[rows/areas.yaml, ./rows/../rows/areas.yaml]",
            "reached twice",
        ),
        (
            "[rows/areas.yaml, rows]",
            "by `rows[0]` and by `rows[1]`, the folder `rows`",
        ),
    ] {
        rows_are(&set, rows);
        let e = refused(&set);
        assert_eq!(e.code, ErrorCode::DuplicateName, "{rows}: {e:?}");
        assert_eq!(e.field.as_deref(), Some("rows[1]"), "{rows}: {e:?}");
        assert!(e.message.contains(says), "{rows}: {e:?}");
    }

    // Outside the set's folder: a list's path, and a folder.
    let outside = set.parent().unwrap().join("shared");
    fs::create_dir_all(&outside).unwrap();
    fs::copy(set.join("rows/areas.yaml"), outside.join("areas.yaml")).unwrap();
    for (rows, field) in [
        ("[../shared/areas.yaml]", "rows[0]"),
        ("[rows/schedules.yaml, rows/../../shared]", "rows[1]"),
        ("../shared", "rows"),
    ] {
        rows_are(&set, rows);
        let e = refused(&set);
        assert_eq!(e.code, ErrorCode::BadValue, "{rows}: {e:?}");
        assert_eq!(e.field.as_deref(), Some(field), "{rows}: {e:?}");
        assert!(
            e.message.contains("outside the drawing set's folder"),
            "{e:?}"
        );
    }
    // One path that names one file is read wherever it leads, as it always
    // has been.
    rows_are(&set, "../shared/areas.yaml");
    assert_eq!(sheets(&built(&set)), ["AREA-101", "AREA-102"]);
}

#[test]
fn a_sheet_named_in_two_rows_files_is_refused_naming_both_places() {
    let set = two_kinds("duplicate");
    let schedules = set.join("rows/schedules.yaml");
    let text = fs::read_to_string(&schedules).unwrap();
    fs::write(
        &schedules,
        text.replace("sheet: SCHED-201", "sheet: area-102"),
    )
    .unwrap();
    let e = refused(&set);
    assert_eq!(e.code, ErrorCode::DuplicateName, "{e:?}");
    assert_eq!(e.field.as_deref(), Some("rows[0].sheet"));
    assert_eq!(
        e.file.as_deref(),
        Some(schedules.display().to_string().as_str())
    );
    assert!(
        e.message.contains(
            "the sheet `area-102` is named in the rows file `rows/areas.yaml`, at line 14, \
             `rows[1]`, already"
        ),
        "{e:?}"
    );
}

#[test]
fn an_error_in_a_row_is_placed_in_the_rows_file_the_row_is_in() {
    let set = two_kinds("placed");
    let schedules = set.join("rows/schedules.yaml");
    let text = fs::read_to_string(&schedules).unwrap();
    fs::write(&schedules, text.replace("      REV: '0'\n", "")).unwrap();
    let reply = build(&set);
    assert_eq!(reply.error.unwrap().code, ErrorCode::SheetsFailed);
    let report = reply.result.unwrap();
    assert_eq!((report.built, report.failed), (2, 1));
    let failed = &report.sheets[2];
    assert_eq!(failed.sheet, "SCHED-201");
    let e = failed.error.as_ref().unwrap();
    assert_eq!(e.code, ErrorCode::MissingValue, "{e:?}");
    assert_eq!(e.field.as_deref(), Some("rows[0].values.REV"));
    assert_eq!(
        e.file.as_deref(),
        Some(schedules.display().to_string().as_str())
    );
}

#[test]
fn an_object_of_a_row_is_written_in_its_rows_file_and_takes_its_place_there() {
    let set = two_kinds("ids");
    built(&set);
    let report = query_with(
        &QueryRequest {
            source: set.clone(),
            sheet: None,
            query: Query::Find(Selection::default()),
        },
        &Settings::default(),
    )
    .unwrap();
    let Answer::Find { objects, .. } = report.answer else {
        panic!("a find's answer")
    };
    let label = objects
        .iter()
        .find(|o| o.id == "AREA-102/rows[1].objects[1]")
        .expect("the label without an id takes its place in its rows file");
    assert_eq!(in_set(&set, &label.written_at.file), "rows/areas.yaml");
    assert_eq!(label.written_at.field, "rows[1].objects[1]");
    let heading = objects
        .iter()
        .find(|o| o.id == "SCHED-201/heading")
        .unwrap();
    assert_eq!(
        in_set(&set, &heading.written_at.file),
        "templates/schedule.yaml"
    );
}

#[test]
fn each_sheets_plot_is_stamped_with_its_own_rows_file() {
    let set = two_kinds("stamp");
    let reply = plot_set_with(
        &PlotSetRequest {
            set: set.clone(),
            overwrite: false,
            plot_style: None,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    for (sheet, rows) in reply
        .result
        .unwrap()
        .sheets
        .iter()
        .map(|s| (s.sheet.as_str(), s.result.as_ref().unwrap()))
        .map(|(s, r)| {
            let rows: Vec<String> = r
                .stamp
                .sources
                .iter()
                .filter(|source| source.kind == SourceKind::Rows)
                .map(|source| source.path.clone())
                .collect();
            (s.to_owned(), rows)
        })
    {
        // A stamp names each source from the PDF's folder.
        let own = if sheet.starts_with("AREA") {
            "../rows/areas.yaml"
        } else {
            "../rows/schedules.yaml"
        };
        assert_eq!(rows, [own], "{sheet}");
    }
}

#[test]
fn a_change_set_on_sheets_of_two_rows_files_writes_each_and_undoes_byte_for_byte() {
    let set = two_kinds("change");
    built(&set);
    let before = snapshot(&set);
    let areas = set.join("rows/areas.yaml");
    let schedules = set.join("rows/schedules.yaml");
    let areas_before = fs::read_to_string(&areas).unwrap();
    let schedules_before = fs::read_to_string(&schedules).unwrap();

    let reply = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: json!({"operations": [
                {"op": "move", "target": {"id": "AREA-102/tank"}, "by": [1, 0, 0]},
                {"op": "move", "target": {"id": "AREA-102/rows[1].objects[1]"}, "by": [1, 0, 0]},
                {"op": "change", "target": {"id": "SCHED-201/heading"}, "set": {"contents": "AREA SHEETS"}},
            ]}),
            dry_run: false,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    let report = reply.result.unwrap();
    assert_eq!((report.landed, report.failed), (2, 0));
    let mut records: Vec<String> = report.records.iter().map(|r| in_set(&set, r)).collect();
    records.sort();
    assert_eq!(records, ["rows/areas.yaml", "rows/schedules.yaml"]);
    let written: Vec<String> = report
        .operations
        .iter()
        .map(|o| in_set(&set, o.record.as_deref().unwrap()))
        .collect();
    assert_eq!(
        written,
        ["rows/areas.yaml", "rows/areas.yaml", "rows/schedules.yaml"]
    );
    let areas_after = fs::read_to_string(&areas).unwrap();
    let schedules_after = fs::read_to_string(&schedules).unwrap();
    assert!(
        areas_after.contains("center: [10, 5.5, 0]"),
        "{areas_after}"
    );
    assert!(
        areas_after.contains("position: [9.2, 3, 0]"),
        "{areas_after}"
    );
    assert!(!areas_after.contains("AREA SHEETS"));
    assert!(
        schedules_after.contains("TITLE: AREA SHEETS"),
        "{schedules_after}"
    );
    assert_ne!(areas_after, areas_before);
    assert_ne!(schedules_after, schedules_before);

    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: report.journal_entry.unwrap(),
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert!(snapshot(&set) == before, "the undo puts every file back");
}
