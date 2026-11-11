//! A change set lands on disk whole or not at all, with its undo state and
//! its entry in the journal: a step of its commit that fails puts back every
//! file the commit had already put in place, and records the change set as
//! a failed attempt.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::commit::FAIL_AT;
use super::journal::{read_entries, EntryKind};
use super::{change_with, ChangeRequest, JOURNAL_FILE};
use crate::error::ErrorCode;
use crate::set::{build_set_with, SetBuildRequest};
use crate::settings::Settings;

/// A copy of the demo set's records in a folder of its own, built when
/// `built` says so.
fn demo_set(test: &str, built: bool) -> PathBuf {
    let set = std::env::temp_dir()
        .join(format!("agentdrafter-change-{}", std::process::id()))
        .join(test);
    let _ = fs::remove_dir_all(&set);
    copy_yaml(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-set"),
        &set,
    );
    if built {
        let reply = build_set_with(
            &SetBuildRequest {
                set: set.clone(),
                overwrite: false,
            },
            &Settings::default(),
        );
        assert!(reply.ok, "{:#?}", reply.error);
    }
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

/// Every file and folder under `folder`, each file with its bytes, but the
/// journal and the folder it is in, since a failed attempt is recorded.
fn snapshot(folder: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let own = folder.join(".agentdrafter");
    let journal = folder.join(JOURNAL_FILE);
    let mut found = BTreeMap::new();
    let mut stack = vec![folder.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path != own {
                    found.insert(path.clone(), None);
                }
                stack.push(path);
            } else if path != journal {
                found.insert(path.clone(), Some(fs::read(&path).unwrap()));
            }
        }
    }
    found
}

fn apply(
    set: &Path,
    change_set: &Value,
    fail_at: Option<usize>,
) -> super::Reply<super::ChangeReport> {
    FAIL_AT.with(|f| f.set(fail_at));
    let reply = change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set: change_set.clone(),
            dry_run: false,
        },
        &Settings::default(),
    );
    FAIL_AT.with(|f| f.set(None));
    reply
}

/// Fails each step of the change set's commit in turn, each time checking
/// that every file and folder is as it was and that the journal records a
/// failed attempt; then lets it land, recorded as a change set. The last
/// step adds the journal's entry.
fn every_failed_step_puts_everything_back(set: &Path, change_set: Value, steps: usize) {
    let before = snapshot(set);
    for step in 0..steps {
        let reply = apply(set, &change_set, Some(step));
        let error = reply.error.unwrap_or_else(|| panic!("step {step} fails"));
        assert_eq!(error.code, ErrorCode::WriteFailed, "{error:?}");
        assert!(
            error.message.contains("nothing was changed"),
            "{}",
            error.message
        );
        assert!(reply.result.is_none());
        assert!(
            snapshot(set) == before,
            "step {step} left every file as it was"
        );
        let last = read_entries(set).unwrap().pop().unwrap();
        assert_eq!(last.kind, EntryKind::FailedAttempt);
        assert_eq!(last.error.unwrap().code, "write_failed");
    }
    // There is no step `steps`: every step was failed in turn.
    let reply = apply(set, &change_set, Some(steps));
    let report = reply.result.unwrap_or_else(|| panic!("{:#?}", reply.error));
    assert!(report.landed > 0);
    assert!(snapshot(set) != before);
    let last = read_entries(set).unwrap().pop().unwrap();
    assert_eq!(last.kind, EntryKind::ChangeSet);
    assert_eq!(Some(last.entry), report.journal_entry);
}

#[test]
fn a_failed_step_puts_back_a_template_and_every_dwg_already_replaced() {
    // A template edit: the template, then the three sheets' DWGs, then the
    // undo state of each, then the journal's entry.
    let set = demo_set("template", true);
    every_failed_step_puts_everything_back(
        &set,
        json!({"operations": [
            {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]},
        ]}),
        9,
    );
}

#[test]
fn a_failed_step_takes_away_a_dwg_and_a_folder_the_change_set_made() {
    // A set never built: the rows, then a DWG in an output folder the
    // commit makes, then the rows' undo state (the DWG had none), then the
    // journal's entry.
    let set = demo_set("unbuilt", false);
    every_failed_step_puts_everything_back(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
        ]}),
        4,
    );
    assert!(set.join("dwg/DEMO-001.dwg").is_file());
}

#[test]
fn a_partial_change_lands_with_its_mark_or_not_at_all() {
    // The rows, DEMO-001's DWG, then the new mark, then the undo states of
    // the rows and the DWG, then the journal's entry.
    let set = demo_set("partial", true);
    every_failed_step_puts_everything_back(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
        6,
    );
    assert!(set.join(super::MARK_FILE).is_file());
    // A fix: the rows, DEMO-002's DWG, then the mark taken away, then the
    // undo states of all three, then the journal's entry. A failed step
    // keeps the mark with the sheet it marks.
    every_failed_step_puts_everything_back(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "notes"},
        ], "fixes": ["DEMO-002"]}),
        7,
    );
    assert!(!set.join(super::MARK_FILE).exists());
}
