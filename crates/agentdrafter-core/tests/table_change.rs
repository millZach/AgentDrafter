//! A change set edits a table's grid: `set_cell` sets a cell's keys, and
//! `add_row`, `delete_row`, `add_column` and `delete_column` reshape the
//! table, keeping its merges, title and header in line; a template's table
//! on every sheet built from it, a row's own on its sheet; a cell its
//! template takes from the row sets the row's value; every refusal names
//! what to do; undo puts the records back byte for byte; and the DWG reads
//! back with the new shape. Every table here is made up.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, read_dwg, undo, ChangeReport, ChangeRequest, ErrorCode, Object,
    Reply, SetBuildRequest, Settings, Table, UndoRequest,
};
use serde_json::{json, Value};

#[path = "support/snapshot.rs"]
mod snapshot;

const TEMPLATE: &str = "units: mm
paper: [200, 100]
objects:
  - table:
      id: sched
      position: [10, 90, 0]
      columns: [30, 40, 40]
      rows: [8, 6, 6, 6]
      title: true
      header: true
      cells:
        # The title spans every column; FEED spans two rows.
        - [{text: SCHEDULE, merge: [1, 3]}, '', '']
        - [TAG, SERVICE, NOTE]
        - [{row: PUMP}, {text: FEED, merge: [2, 1]}, '']
        - [V-1, '', SPARE]
";

const ROWS: &str = "rows:
  - sheet: S-1
    template: templates/t.yaml
    values: {PUMP: P-101}
    objects:
      - table:
          id: own
          position: [120, 90, 0]
          columns: [20, 30]
          rows: [6, 7]
          cells:
            - [A, B]
            - ['1', '2']
  - sheet: S-2
    template: templates/t.yaml
    values: {PUMP: P-202}
";

/// A built made-up set of two sheets from one template, whose table has a
/// title, a header, two merges and a cell from the row; sheet `S-1` has a
/// table of its own.
fn table_set(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-table-change")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("set");
    fs::create_dir_all(set.join("templates")).unwrap();
    fs::write(
        set.join("set.yaml"),
        "units: mm\noutput: dwg\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(set.join("rows.yaml"), ROWS).unwrap();
    fs::write(set.join("templates/t.yaml"), TEMPLATE).unwrap();
    let built = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
        },
        &Settings::default(),
    );
    assert!(built.ok, "{:#?}", built.error);
    assert_eq!(built.result.unwrap().built, 2);
    set
}

fn apply(set: &Path, operations: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set: json!({ "operations": operations }),
            dry_run: false,
        },
        &Settings::default(),
    )
}

fn lands(set: &Path, operations: Value) -> ChangeReport {
    let reply = apply(set, operations);
    assert!(reply.ok, "{:#?}", reply.error);
    let report = reply.result.unwrap();
    assert_eq!(report.failed, 0, "{:#?}", report.sheets);
    report
}

/// The error of a change set that fails on every sheet it touches, or is
/// refused whole.
fn fails(set: &Path, operations: Value) -> agentdrafter_core::Error {
    let before = snapshot::snapshot(set);
    let reply = apply(set, operations);
    assert!(!reply.ok);
    assert_eq!(snapshot::snapshot(set), before, "nothing is written");
    let report = reply.result.expect("a sheet failed");
    assert_eq!(report.landed, 0);
    report.sheets[0].error.clone().expect("the sheet's error")
}

/// The table `id` as sheet `sheet`'s DWG reads it back.
fn table(set: &Path, sheet: &str, id: &str) -> Table {
    let read = read_dwg(&set.join("dwg").join(format!("{sheet}.dwg"))).unwrap();
    read.ids_and_names
        .iter()
        .zip(&read.objects)
        .find_map(|(ids, object)| match object {
            Object::Table(t) if ids.id.as_deref() == Some(&format!("{sheet}/{id}")) => {
                Some(t.clone())
            }
            _ => None,
        })
        .expect("the table reads back")
}

fn texts(table: &Table) -> Vec<Vec<&str>> {
    table
        .cells
        .iter()
        .map(|row| row.iter().map(|c| c.text.as_str()).collect())
        .collect()
}

fn template(set: &Path) -> String {
    fs::read_to_string(set.join("templates/t.yaml")).unwrap()
}

fn undo_all(set: &Path, report: &ChangeReport) {
    let undone = undo(&UndoRequest {
        set: set.to_owned(),
        change_set: report.journal_entry.unwrap(),
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
}

#[test]
fn set_cell_on_a_template_table_reaches_every_sheet_and_undoes_byte_for_byte() {
    let set = table_set("set-template");
    let start = snapshot::snapshot(&set);
    let report = lands(
        &set,
        json!([{"op": "TABLEDIT", "target": {"id": "S-1/sched"}, "cell": [3, 0], "set": {"text": "V-2", "justify": "middle_center"}}]),
    );
    let op = &report.operations[0];
    assert_eq!(op.sheets, ["S-1", "S-2"]);
    assert!(
        op.message
            .starts_with("sets justify, text of cell [3, 0] of `S-1/sched`"),
        "{}",
        op.message
    );
    // Only the table's lines change; the comment outside it stays.
    let text = template(&set);
    assert!(
        text.contains("        - [{text: V-2, justify: middle_center}, '', SPARE]\n"),
        "{text}"
    );
    assert_eq!(
        text.lines().count(),
        TEMPLATE.lines().count() - 1,
        "the comment inside the table goes"
    );
    for sheet in ["S-1", "S-2"] {
        let t = table(&set, sheet, "sched");
        assert_eq!(t.cells[3][0].text, "V-2");
        assert!(t.cells[3][0].justify.is_some());
    }
    undo_all(&set, &report);
    assert_eq!(snapshot::snapshot(&set), start);
}

#[test]
fn a_cell_its_template_takes_from_the_row_sets_that_sheets_row_value_alone() {
    let set = table_set("set-row-value");
    let template_before = template(&set);
    let s1 = fs::read(set.join("dwg/S-1.dwg")).unwrap();
    let report = lands(
        &set,
        json!([{"op": "set_cell", "target": {"id": "S-2/sched"}, "cell": [2, 0], "set": {"text": "P-999"}}]),
    );
    let op = &report.operations[0];
    assert_eq!(op.sheets, ["S-2"]);
    let row_value = op.row_value.as_ref().expect("the row value it set");
    assert_eq!(
        (
            row_value.value.as_str(),
            row_value.was.as_str(),
            row_value.now.as_str()
        ),
        ("PUMP", "P-202", "P-999")
    );
    assert_eq!(row_value.fills[0].id, "S-2/sched");
    assert_eq!(row_value.fills[0].at, "cells[2][0]");
    assert_eq!(template(&set), template_before);
    assert!(fs::read_to_string(set.join("rows.yaml"))
        .unwrap()
        .contains("values: {PUMP: P-999}"));
    assert_eq!(fs::read(set.join("dwg/S-1.dwg")).unwrap(), s1);
    assert_eq!(table(&set, "S-2", "sched").cells[2][0].text, "P-999");
}

#[test]
fn a_merge_is_set_and_taken_away_and_a_cell_inside_one_is_refused() {
    let set = table_set("merge");
    // A cell inside FEED's merge is refused, naming the merge's cell.
    let e = fails(
        &set,
        json!([{"op": "set_cell", "target": {"id": "S-1/sched"}, "cell": [3, 1], "set": {"text": "X"}}]),
    );
    assert_eq!(e.code, ErrorCode::BadValue);
    assert_eq!(e.field.as_deref(), Some("operations[0].cell"));
    assert!(
        e.message.contains("cell [2, 1], which merges [2, 1]"),
        "{}",
        e.message
    );
    // A merge over a cell that is not empty is refused, naming it.
    let e = fails(
        &set,
        json!([{"op": "set_cell", "target": {"id": "S-1/own"}, "cell": [0, 0], "set": {"merge": [1, 2]}}]),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].set.merge"));
    assert!(e.message.contains("not empty: [0, 1]"), "{}", e.message);
    // Emptied first, it lands.
    lands(
        &set,
        json!([
            {"op": "set_cell", "target": {"id": "S-1/own"}, "cell": [0, 1], "set": {"text": null}},
            {"op": "set_cell", "target": {"id": "S-1/own"}, "cell": [0, 0], "set": {"merge": [1, 2]}},
        ]),
    );
    assert!(fs::read_to_string(set.join("rows.yaml"))
        .unwrap()
        .contains("            - [{text: A, merge: [1, 2]}, '']\n"));
    assert_eq!(table(&set, "S-1", "own").cells[0][0].merge, Some([1, 2]));
    lands(
        &set,
        json!([{"op": "set_cell", "target": {"id": "S-1/own"}, "cell": [0, 0], "set": {"merge": null}}]),
    );
    assert!(fs::read_to_string(set.join("rows.yaml"))
        .unwrap()
        .contains("            - [{text: A}, '']\n"));
    assert_eq!(table(&set, "S-1", "own").cells[0][0].merge, None);
}

#[test]
fn add_row_at_the_start_the_middle_and_the_end() {
    let set = table_set("add-row");
    let start = snapshot::snapshot(&set);
    let report = lands(
        &set,
        json!([
            // Inside FEED's merge, which grows.
            {"op": "add_row", "target": {"id": "S-1/sched"}, "at": 3, "height": 5, "cells": ["V-0", "", "NEW"]},
            // At the end, as high as the last, empty.
            {"op": "add_row", "target": {"id": "S-1/sched"}},
            // At the start of the data rows, after the header.
            {"op": "add_row", "target": {"id": "S-1/sched"}, "at": 2, "cells": ["FIRST", "", ""]},
            // At the start of a table with no title, as high as the first.
            {"op": "add_row", "target": {"id": "S-1/own"}, "at": 0},
        ]),
    );
    assert!(
        report.operations[0]
            .message
            .contains("the merge of cell [2, 1] grows to [3, 1]"),
        "{}",
        report.operations[0].message
    );
    for sheet in ["S-1", "S-2"] {
        let t = table(&set, sheet, "sched");
        assert_eq!(t.rows, [8.0, 6.0, 6.0, 6.0, 5.0, 6.0, 6.0]);
        let pump = if sheet == "S-1" { "P-101" } else { "P-202" };
        assert_eq!(
            texts(&t),
            [
                vec!["SCHEDULE", "", ""],
                vec!["TAG", "SERVICE", "NOTE"],
                vec!["FIRST", "", ""],
                vec![pump, "FEED", ""],
                vec!["V-0", "", "NEW"],
                vec!["V-1", "", "SPARE"],
                vec!["", "", ""],
            ]
        );
        assert_eq!(t.cells[3][1].merge, Some([3, 1]));
        assert!(t.title && t.header);
    }
    let own = table(&set, "S-1", "own");
    assert_eq!(own.rows, [6.0, 6.0, 7.0]);
    assert_eq!(texts(&own)[0], ["", ""]);
    undo_all(&set, &report);
    assert_eq!(snapshot::snapshot(&set), start);
}

#[test]
fn a_row_is_not_added_above_the_title_or_between_it_and_the_header() {
    let set = table_set("add-row-head");
    for at in [0, 1] {
        let e = fails(
            &set,
            json!([{"op": "add_row", "target": {"id": "S-1/sched"}, "at": at}]),
        );
        assert_eq!(e.code, ErrorCode::BadValue);
        assert_eq!(e.field.as_deref(), Some("operations[0].at"));
        assert!(
            e.message.contains("title row and its header row"),
            "{}",
            e.message
        );
    }
    let e = fails(
        &set,
        json!([{"op": "add_row", "target": {"id": "S-1/sched"}, "at": 9}]),
    );
    assert!(e.message.contains("no place 9"), "{}", e.message);
}

#[test]
fn add_column_at_the_start_the_middle_and_the_end() {
    let set = table_set("add-column");
    let start = snapshot::snapshot(&set);
    let report = lands(
        &set,
        json!([
            // Inside the title's merge, which grows.
            {"op": "add_column", "target": {"id": "S-1/sched"}, "at": 1, "width": "25 mm"},
            // At the end: the title's merge ends before it.
            {"op": "add_column", "target": {"id": "S-1/sched"}, "cells": ["", "QTY", "1", "2"]},
            // At the start, with cells.
            {"op": "add_column", "target": {"id": "S-1/sched"}, "at": 0, "width": 10, "cells": ["#", "#", "1", "2"]},
        ]),
    );
    let t = table(&set, "S-2", "sched");
    assert_eq!(t.columns, [10.0, 30.0, 25.0, 40.0, 40.0, 40.0]);
    assert_eq!(t.cells[0][1].merge, Some([1, 4]));
    assert_eq!(texts(&t)[1], ["#", "TAG", "", "SERVICE", "NOTE", "QTY"]);
    assert_eq!(texts(&t)[0], ["#", "SCHEDULE", "", "", "", ""]);
    assert_eq!(t.cells[2][3].merge, Some([2, 1]));
    undo_all(&set, &report);
    assert_eq!(snapshot::snapshot(&set), start);
}

#[test]
fn delete_row_through_a_merge_shrinks_it_and_on_its_cell_is_refused() {
    let set = table_set("delete-row");
    // The row FEED's merge starts in is refused, naming the cell.
    let e = fails(
        &set,
        json!([{"op": "delete_row", "target": {"id": "S-1/sched"}, "row": 2}]),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].row"));
    assert!(
        e.message.contains("cell [2, 1], which merges [2, 1]"),
        "{}",
        e.message
    );
    assert!(
        e.message.contains("set: {\"merge\": null}"),
        "{}",
        e.message
    );

    let start = snapshot::snapshot(&set);
    let report = lands(
        &set,
        json!([
            // Through FEED's merge, which then spans its own cell alone.
            {"op": "delete_row", "target": {"id": "S-1/sched"}, "row": 3},
            // The title row: the table has no title then.
            {"op": "delete_row", "target": {"id": "S-1/sched"}, "row": 0},
        ]),
    );
    assert!(
        report.operations[0].message.contains("so it is taken away"),
        "{}",
        report.operations[0].message
    );
    let text = template(&set);
    assert!(!text.contains("title:"), "{text}");
    assert!(text.contains("header: true"), "{text}");
    assert!(text.contains("rows: [6, 6]"), "{text}");
    let t = table(&set, "S-1", "sched");
    assert!(!t.title && t.header);
    assert_eq!(
        texts(&t),
        [vec!["TAG", "SERVICE", "NOTE"], vec!["P-101", "FEED", ""]]
    );
    assert_eq!(t.cells[1][1].merge, None);
    undo_all(&set, &report);
    assert_eq!(snapshot::snapshot(&set), start);
}

#[test]
fn delete_column_through_a_merge_shrinks_it_and_on_its_cell_is_refused() {
    let set = table_set("delete-column");
    let e = fails(
        &set,
        json!([{"op": "delete_column", "target": {"id": "S-1/sched"}, "column": 0}]),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].column"));
    assert!(
        e.message.contains("cell [0, 0], which merges [1, 3]"),
        "{}",
        e.message
    );

    let start = snapshot::snapshot(&set);
    let report = lands(
        &set,
        json!([{"op": "delete_column", "target": {"id": "S-1/sched"}, "column": 2}]),
    );
    assert!(report.operations[0].message.contains("shrinks to [1, 2]"));
    let t = table(&set, "S-2", "sched");
    assert_eq!(t.columns, [30.0, 40.0]);
    assert_eq!(t.cells[0][0].merge, Some([1, 2]));
    assert_eq!(texts(&t)[3], ["V-1", ""]);
    undo_all(&set, &report);
    assert_eq!(snapshot::snapshot(&set), start);
}

#[test]
fn the_last_row_and_the_last_column_are_not_deleted() {
    let set = table_set("delete-last");
    let e = fails(
        &set,
        json!([
            {"op": "delete_row", "target": {"id": "S-1/own"}, "row": 1},
            {"op": "delete_row", "target": {"id": "S-1/own"}, "row": 0},
        ]),
    );
    assert_eq!(e.field.as_deref(), Some("operations[1].row"));
    assert!(e.message.contains("one row"), "{}", e.message);
    let e = fails(
        &set,
        json!([
            {"op": "delete_column", "target": {"id": "S-1/own"}, "column": 0},
            {"op": "delete_column", "target": {"id": "S-1/own"}, "column": 0},
        ]),
    );
    assert_eq!(e.field.as_deref(), Some("operations[1].column"));
}

#[test]
fn a_table_the_operation_leaves_wrong_is_placed_on_the_operations_field() {
    let set = table_set("errors");
    // A merge the reader refuses.
    let e = fails(
        &set,
        json!([{"op": "set_cell", "target": {"id": "S-1/own"}, "cell": [1, 1], "set": {"merge": [0, 1]}}]),
    );
    assert_eq!(e.code, ErrorCode::BadNumber);
    assert_eq!(e.field.as_deref(), Some("operations[0].set.merge[0]"));
    // A height that is not more than 0.
    let e = fails(
        &set,
        json!([{"op": "add_row", "target": {"id": "S-1/own"}, "height": 0}]),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].height"));
    // A cell of the new row inside a merge that grows must be empty.
    let e = fails(
        &set,
        json!([{"op": "add_row", "target": {"id": "S-1/sched"}, "at": 3, "cells": ["", "X", ""]}]),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].cells[1]"));
    // A table operation on something else.
    let e = fails(
        &set,
        json!([{"op": "set_cell", "target": {"id": "S-1/own"}, "cell": [5, 0], "set": {"text": "X"}}]),
    );
    assert_eq!(e.field.as_deref(), Some("operations[0].cell"));
    // A key a cell does not take refuses the change set whole.
    let reply = apply(
        &set,
        json!([{"op": "set_cell", "target": {"id": "S-1/own"}, "cell": [0, 0], "set": {"colour": "red"}}]),
    );
    let e = reply.error.unwrap();
    assert_eq!(e.code, ErrorCode::UnknownKey);
    assert_eq!(e.field.as_deref(), Some("operations[0].set.colour"));
}
