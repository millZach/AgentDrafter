//! Integration tests for Rank 26 (#86): `replace_text` (`FIND`) and
//! `set_attributes` (`GATTE`) across drawing sets and original DWGs.
//!
//! All drawings, blocks, tags and values here are made up (ADR-0008).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use agentdrafter_core::{
    build_set_with, change_with, journal, query_with, read_dwg, revise_with, undo, ChangeRequest,
    ErrorCode, JournalRequest, Object, Query, QueryRequest, ReviseRequest, Selection,
    SetBuildRequest, Settings, UndoRequest,
};

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "agentdrafter-find-replace-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_system_fonts() -> Settings {
    Settings::default()
}

/// Builds a made-up 3-sheet controls drawing set with:
/// - a shared template with a title block (`TITLE_BLOCK` carrying `TITLE`, `REV`, `DATE`, and
///   repeated `NOTE` tags, plus a constant `PLANT` tag), a `heading` text bound to `{row: TITLE}`,
///   a composite `label` text bound to `['CABINET ', {row: CAB}, ' LOOP ', {row: LOOP}]`,
///   an `mtext` note bound to `{row: NOTES}`, and a literal template footer `footer`.
/// - row-owned `text` and `insert` objects per sheet.
fn write_controls_set(root: &Path) {
    fs::write(
        root.join("set.yaml"),
        "units: mm\noutput: out\nrows: rows.yaml\n",
    )
    .unwrap();
    fs::write(
        root.join("blocks.yaml"),
        r#"units: mm
blocks:
  TITLE_BLOCK:
    base_point: [0, 0, 0]
    objects:
      - polyline: {points: [[0, 0, 0], [80, 0, 0], [80, 20, 0], [0, 20, 0]], closed: true}
      - attdef: {tag: TITLE, position: [2, 14, 0], height: 3, default: ""}
      - attdef: {tag: REV, position: [2, 8, 0], height: 2.5, default: "0"}
      - attdef: {tag: DATE, position: [30, 8, 0], height: 2.5, default: "2026-01-01"}
      - attdef: {tag: NOTE, position: [2, 3, 0], height: 2, default: "-"}
      - attdef: {tag: NOTE, position: [40, 3, 0], height: 2, default: "-"}
      - attdef: {tag: PLANT, position: [60, 14, 0], height: 2, default: "DEMO", constant: true}
  INSTRUMENT:
    base_point: [0, 0, 0]
    objects:
      - circle: {center: [0, 0, 0], radius: 5}
      - attdef: {tag: TAG, position: [0, 0, 0], height: 2, default: "FT-000"}
      - attdef: {tag: CABINET, position: [0, -7, 0], height: 2, default: "JB-101"}
"#,
    )
    .unwrap();
    fs::write(
        root.join("template.yaml"),
        r#"units: mm
paper: [210, 148]
block_library: blocks.yaml
layers:
  0: {}
  locked_notes: {locked: true}
objects:
  - polyline: {id: border, points: [[0, 0, 0], [210, 0, 0], [210, 148, 0], [0, 148, 0]], closed: true}
  - text: {id: heading, position: [10, 136, 0], height: 4, contents: {row: TITLE}}
  - text: {id: label, position: [10, 126, 0], height: 3, contents: ['CABINET ', {row: CAB}, ' LOOP ', {row: LOOP}]}
  - mtext: {id: notes, position: [10, 112, 0], width: 90, height: 2.5, contents: {row: NOTES}}
  - text: {id: footer, position: [10, 5, 0], height: 2, contents: "STANDARD SPEC REV A"}
  - insert:
      id: tb
      name: title-block
      block: TITLE_BLOCK
      position: [120, 5, 0]
      attributes:
        TITLE: {row: TITLE}
        REV: {row: REV}
        DATE: {row: DATE}
        NOTE: [{row: NOTE1}, {row: NOTE2}]
"#,
    )
    .unwrap();
    fs::write(
        root.join("rows.yaml"),
        r#"rows:
  - sheet: LOOP-001
    template: template.yaml
    values:
      TITLE: "AREA 100 JB-101 FLOW FT-101"
      CAB: "JB-101"
      LOOP: "FT-101"
      NOTES: "WIRE FROM JB-101 TO P1\nCHECK P10 AND P1-A"
      REV: "A"
      DATE: "2026-10-01"
      NOTE1: "ISSUED FOR JB-101"
      NOTE2: "PRELIMINARY"
    objects:
      - insert: {id: inst1, block: INSTRUMENT, position: [40, 70, 0], attributes: {TAG: "FT-101", CABINET: "JB-101"}}
      - text: {id: wire-note, position: [40, 50, 0], height: 2.5, contents: "CABLE TO JB-101"}
  - sheet: LOOP-002
    template: template.yaml
    values:
      TITLE: "AREA 100 JB-101 FLOW FT-102"
      CAB: "JB-101"
      LOOP: "FT-102"
      NOTES: "WIRE FROM JB-101 TO P1"
      REV: "A"
      DATE: "2026-10-01"
      NOTE1: "ISSUED FOR JB-101"
      NOTE2: "PRELIMINARY"
    objects:
      - insert: {id: inst2, block: INSTRUMENT, position: [40, 70, 0], attributes: {TAG: "FT-102", CABINET: "JB-101"}}
  - sheet: LOOP-003
    template: template.yaml
    values:
      TITLE: "AREA 200 JB-1010 LEVEL LT-301"
      CAB: "JB-1010"
      LOOP: "LT-301"
      NOTES: "WIRE FROM JB-1010"
      REV: "A"
      DATE: "2026-10-01"
      NOTE1: "ISSUED FOR JB-1010"
      NOTE2: "HOLD"
    objects:
      - insert: {id: inst3, block: INSTRUMENT, position: [40, 70, 0], attributes: {TAG: "LT-301", CABINET: "JB-1010"}}
"#,
    )
    .unwrap();
}

#[test]
fn replace_text_whole_word_dry_run_apply_and_undo_across_set() {
    let dir = scratch("whole-word");
    write_controls_set(&dir);
    let settings = no_system_fonts();

    let built = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: false,
            clean: None,
        },
        &settings,
    );
    assert!(built.ok, "{built:?}");
    let rows_before = fs::read(dir.join("rows.yaml")).unwrap();

    let cs = json!({
        "operations": [
            {
                "op": "FIND",
                "find": "JB-101",
                "replace": "JB-201",
                "match": "whole_word"
            }
        ]
    });

    // 1. Dry run reports every match across LOOP-001 and LOOP-002 without touching LOOP-003 (JB-1010) or disk.
    let dry = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: cs.clone(),
            dry_run: true,
        },
        &settings,
    );
    assert!(dry.ok, "{dry:?}");
    let dry_report = dry.result.unwrap();
    assert!(dry_report.dry_run);
    assert_eq!(dry_report.landed, 2);
    let dry_op = &dry_report.operations[0];
    assert_eq!(dry_op.op, "replace_text");
    assert_eq!(dry_op.sheets, vec!["LOOP-001", "LOOP-002"]);
    // LOOP-003 has JB-1010 which must NOT match whole_word "JB-101".
    assert!(
        dry_op.replacements.iter().all(|r| r.sheet != "LOOP-003"),
        "{:?}",
        dry_op.replacements
    );
    // Must include texts (heading, label, wire-note), mtexts (notes), and attributes (tb.TITLE, tb.NOTE, inst.CABINET).
    assert!(dry_op
        .replacements
        .iter()
        .any(|r| r.id == "LOOP-001/heading" && r.kind == "text"));
    assert!(dry_op
        .replacements
        .iter()
        .any(|r| r.id == "LOOP-001/notes" && r.kind == "mtext"));
    assert!(dry_op.replacements.iter().any(|r| r.id == "LOOP-001/tb"
        && r.kind == "attribute"
        && r.tag.as_deref() == Some("TITLE")));
    assert!(dry_op.replacements.iter().any(|r| r.id == "LOOP-001/tb"
        && r.kind == "attribute"
        && r.tag.as_deref() == Some("NOTE")
        && r.occurrence == Some(1)));
    assert!(dry_op
        .replacements
        .iter()
        .any(|r| r.id == "LOOP-001/inst1" && r.tag.as_deref() == Some("CABINET")));
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);

    // 2. Apply for real.
    let applied = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: cs,
            dry_run: false,
        },
        &settings,
    );
    assert!(applied.ok, "{applied:?}");
    let report = applied.result.unwrap();
    let entry = report.journal_entry.expect("journal entry");
    assert_eq!(report.landed, 2);
    assert_eq!(
        report.operations[0].replacements.len(),
        dry_op.replacements.len()
    );

    // Verify rebuilt DWG carries the updated text and attributes, and LOOP-003 stayed JB-1010.
    let dwg1 = read_dwg(&dir.join("out/LOOP-001.dwg")).unwrap();
    let heading1 = dwg1
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Text(t) if t.contents.contains("FLOW FT-101") => Some(t.contents.as_str()),
            _ => None,
        })
        .unwrap();
    assert_eq!(heading1, "AREA 100 JB-201 FLOW FT-101");

    let dwg3 = read_dwg(&dir.join("out/LOOP-003.dwg")).unwrap();
    let heading3 = dwg3
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Text(t) if t.contents.contains("LEVEL LT-301") => Some(t.contents.as_str()),
            _ => None,
        })
        .unwrap();
    assert_eq!(heading3, "AREA 200 JB-1010 LEVEL LT-301");

    // 3. Journal records the entry and undo restores rows.yaml byte-for-byte.
    let j = journal(&JournalRequest { set: dir.clone() });
    assert!(j.ok, "{j:?}");
    let undone = undo(&UndoRequest {
        set: dir.clone(),
        change_set: entry,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{undone:?}");
    assert_eq!(fs::read(dir.join("rows.yaml")).unwrap(), rows_before);
}

#[test]
fn replace_text_pattern_scopes_filters_and_shared_template_guards() {
    let dir = scratch("pattern-scopes");
    write_controls_set(&dir);
    let settings = no_system_fonts();

    // 1. Pattern with capture groups, restricted to attributes with tag TAG across LOOP-001 & LOOP-002.
    let rep = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "replace_text",
                        "find": "FT-(\\d{3})",
                        "replace": "FIT-$1",
                        "match": "pattern",
                        "in": ["attribute"],
                        "tag": "TAG",
                        "sheets": ["LOOP-001", "LOOP-002"]
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(rep.ok, "{rep:?}");
    let r = rep.result.unwrap();
    assert_eq!(r.operations[0].replacements.len(), 2);
    assert_eq!(r.operations[0].replacements[0].before, "FT-101");
    assert_eq!(r.operations[0].replacements[0].after, "FIT-101");
    assert_eq!(r.operations[0].replacements[1].before, "FT-102");
    assert_eq!(r.operations[0].replacements[1].after, "FIT-102");

    // Heading text was NOT modified because `in: ["attribute"]`.
    let q = query_with(
        &QueryRequest {
            source: dir.clone(),
            sheet: Some("LOOP-001".to_owned()),
            query: Query::Find(Selection {
                id: Some("LOOP-001/heading".to_owned()),
                ..Selection::default()
            }),
        },
        &settings,
    );
    assert!(q.is_ok(), "{q:?}");
    assert!(format!("{q:?}").contains("FT-101"));

    // 2. Trying to replace literal shared template text ("STANDARD SPEC REV A") on a subset of sheets is refused.
    let subset_refused = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "replace_text",
                        "find": "REV A",
                        "replace": "REV B",
                        "sheets": ["LOOP-001"]
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!subset_refused.ok, "{subset_refused:?}");
    let report = subset_refused.result.unwrap();
    let err = report.operations[0].error.as_ref().unwrap();
    assert_eq!(err.code, ErrorCode::BadValue);
    assert!(err.message.contains("shared template"), "{}", err.message);

    // 3. Replacing that literal shared template text across the whole set succeeds and edits template.yaml once.
    let whole_set = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "replace_text",
                        "find": "rev a",
                        "replace": "REV B",
                        "case_sensitive": false
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(whole_set.ok, "{whole_set:?}");
    let ws = whole_set.result.unwrap();
    assert_eq!(ws.landed, 3);
    assert_eq!(ws.operations[0].replacements.len(), 3);
    assert!(fs::read_to_string(dir.join("template.yaml"))
        .unwrap()
        .contains("STANDARD SPEC REV B"));

    // 4. Zero matches succeeds cleanly with 0 touched sheets.
    let zero = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "replace_text",
                        "find": "DOES-NOT-EXIST",
                        "replace": "NOTHING"
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(zero.ok, "{zero:?}");
    let zr = zero.result.unwrap();
    assert_eq!(zr.landed, 0);
    assert!(zr.operations[0].replacements.is_empty());
}

#[test]
fn set_attributes_updates_every_matching_insert_across_set() {
    let dir = scratch("set-attributes");
    write_controls_set(&dir);
    let settings = no_system_fonts();

    // 1. Set DATE on every TITLE_BLOCK insert across the whole set via GATTE alias.
    let rep = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "GATTE",
                        "where": {"block": "TITLE_BLOCK"},
                        "tag": "date",
                        "value": "2026-11-24"
                    },
                    {
                        "op": "set_attributes",
                        "where": {"block": "TITLE_BLOCK"},
                        "tag": "NOTE",
                        "occurrence": 2,
                        "value": "ISSUED FOR CONSTRUCTION",
                        "sheets": ["LOOP-001", "LOOP-002"]
                    },
                    {
                        "op": "set_attributes",
                        "where": {"block": "INSTRUMENT", "tag": "CABINET", "value": "JB-101"},
                        "tag": "CABINET",
                        "value": "JB-500"
                    }
                ]
            }),
            dry_run: false,
        },
        &settings,
    );
    assert!(rep.ok, "{rep:?}");
    let report = rep.result.unwrap();
    assert_eq!(report.landed, 3);
    assert_eq!(report.operations[0].op, "set_attributes");
    assert_eq!(report.operations[0].attributes_set.len(), 3);
    assert_eq!(report.operations[1].attributes_set.len(), 2);
    assert_eq!(report.operations[1].attributes_set[0].occurrence, Some(2));
    assert_eq!(report.operations[2].attributes_set.len(), 2);

    // Check rebuilt DWG for LOOP-001 and LOOP-003.
    let dwg1 = read_dwg(&dir.join("out/LOOP-001.dwg")).unwrap();
    let tb1 = dwg1
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Insert(ins) if ins.block == "TITLE_BLOCK" => Some(ins),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        tb1.attributes
            .iter()
            .find(|a| a.tag == "DATE")
            .unwrap()
            .value,
        "2026-11-24"
    );
    let notes1: Vec<&str> = tb1
        .attributes
        .iter()
        .filter(|a| a.tag == "NOTE")
        .map(|a| a.value.as_str())
        .collect();
    assert_eq!(notes1, vec!["ISSUED FOR JB-101", "ISSUED FOR CONSTRUCTION"]);

    let dwg3 = read_dwg(&dir.join("out/LOOP-003.dwg")).unwrap();
    let tb3 = dwg3
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Insert(ins) if ins.block == "TITLE_BLOCK" => Some(ins),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        tb3.attributes
            .iter()
            .find(|a| a.tag == "DATE")
            .unwrap()
            .value,
        "2026-11-24"
    );
    let notes3: Vec<&str> = tb3
        .attributes
        .iter()
        .filter(|a| a.tag == "NOTE")
        .map(|a| a.value.as_str())
        .collect();
    assert_eq!(notes3, vec!["ISSUED FOR JB-1010", "HOLD"]);

    // 2. Setting a non-existent tag on a matched insert fails cleanly with UnknownName.
    let bad_tag = change_with(
        &ChangeRequest {
            set: dir.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "set_attributes",
                        "where": {"block": "TITLE_BLOCK"},
                        "tag": "NO_SUCH_TAG",
                        "value": "X"
                    }
                ]
            }),
            dry_run: true,
        },
        &settings,
    );
    assert!(!bad_tag.ok, "{bad_tag:?}");
    let report = bad_tag.result.unwrap();
    let err = report.operations[0].error.as_ref().unwrap();
    assert_eq!(err.code, ErrorCode::UnknownName);
}

#[test]
fn replace_text_and_set_attributes_work_on_original_dwg() {
    let dir = scratch("original-dwg");
    write_controls_set(&dir);
    let settings = no_system_fonts();

    let built = build_set_with(
        &SetBuildRequest {
            set: dir.clone(),
            overwrite: false,
            clean: None,
        },
        &settings,
    );
    assert!(built.ok, "{built:?}");

    let original = dir.join("LOOP-001.dwg");
    fs::copy(dir.join("out/LOOP-001.dwg"), &original).unwrap();
    let original_bytes = fs::read(&original).unwrap();

    let rev = revise_with(
        &ReviseRequest {
            set: original.clone(),
            change_set: json!({
                "operations": [
                    {
                        "op": "FIND",
                        "find": "JB-101",
                        "replace": "JB-909",
                        "match": "whole_word"
                    },
                    {
                        "op": "GATTE",
                        "where": {"block": "TITLE_BLOCK"},
                        "tag": "REV",
                        "value": "B"
                    }
                ]
            }),
            output: Some(dir.join("LOOP-001.rev1.dwg")),
            allow_loss: false,
            dry_run: false,
            expected_original_sha256: None,
            plot_style: None,
            no_pdf: true,
        },
        &settings,
    );
    assert!(rev.ok, "{rev:?}");
    let report = rev.result.unwrap();
    assert!(
        report.unexpected_changes.is_empty(),
        "{:?}",
        report.unexpected_changes
    );
    assert!(
        report.requested_failures.is_empty(),
        "{:?}",
        report.requested_failures
    );
    assert!(!report.operations[0].replacements.is_empty());
    assert_eq!(report.operations[1].attributes_set.len(), 1);
    assert_eq!(report.operations[1].attributes_set[0].before, "A");
    assert_eq!(report.operations[1].attributes_set[0].after, "B");

    // Original file is strictly untouched.
    assert_eq!(fs::read(&original).unwrap(), original_bytes);

    // Revision DWG has updated text & attributes.
    let revised = read_dwg(&dir.join("LOOP-001.rev1.dwg")).unwrap();
    let heading = revised
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Text(t) if t.contents.contains("FLOW FT-101") => Some(t.contents.as_str()),
            _ => None,
        })
        .unwrap();
    assert_eq!(heading, "AREA 100 JB-909 FLOW FT-101");
    let tb = revised
        .objects
        .iter()
        .find_map(|o| match o {
            Object::Insert(ins) if ins.block == "TITLE_BLOCK" => Some(ins),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        tb.attributes.iter().find(|a| a.tag == "REV").unwrap().value,
        "B"
    );
}
