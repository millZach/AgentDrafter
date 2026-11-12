//! A change set lands on disk whole or not at all, with its undo state and
//! its entry in the journal: a step of its commit that fails puts back every
//! file the commit had already put in place, and records the change set as
//! a failed attempt.
//!
//! A commit stopped at any step, as a killed process stops, leaves its entry
//! pending: the next command that reads the set, whichever it is, puts every
//! file back byte for byte, marks the entry rolled back and says so, and a
//! further change set lands as it would have. So for an undo and an accept.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use serde_json::{json, Value};

use super::commit::{Stopped, FAIL_AT, FAIL_DONE, FAIL_SYNC, STOP_AT, STOP_RESTORE_AT};
use super::journal::{
    lock, read_entries, state_line, CommitState, EntryKind, FileState, LOCK_FILE,
};
use super::undo::AFTER_PLAN;
use super::{
    accept_with, change_with, undo, AcceptRequest, ChangeRequest, UndoRequest, JOURNAL_FILE,
    UNDO_STATES_FOLDER,
};
use crate::commands::{find_command, JsonReply};
use crate::error::ErrorCode;
use crate::interface::Interface;
use crate::set::{build_set_with, SetBuildRequest};
use crate::settings::Settings;
use crate::stamp::fingerprint;

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
/// journal, its lock and the folder they are in, since a failed attempt is
/// recorded and every commit takes the lock.
fn snapshot(folder: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let own = folder.join(".agentdrafter");
    let journal = folder.join(JOURNAL_FILE);
    let lock = folder.join(LOCK_FILE);
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
            } else if path != journal && path != lock {
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

/// Applies a change set that lands, and gives its entry in the journal.
fn lands(set: &Path, change_set: &Value) -> u64 {
    let reply = apply(set, change_set, None);
    let report = reply.result.unwrap_or_else(|| panic!("{:#?}", reply.error));
    report.journal_entry.unwrap()
}

/// Fails each step of the change set's commit in turn, each time checking
/// that every file and folder is as it was and that the journal records a
/// failed attempt; then lets it land, recorded as a change set. The last
/// step marks the journal's entry done.
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
    assert_eq!(last.state, Some(CommitState::Done));
    assert_eq!(Some(last.entry), report.journal_entry);
}

#[test]
fn a_failed_step_puts_back_a_template_and_every_dwg_already_replaced() {
    // A template edit: the undo state of the template and of each of the
    // three sheets' DWGs, then the template, then the DWGs, then the line
    // that marks the journal's entry done.
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
    // A set never built: the rows' undo state (the DWG had none), then the
    // rows, then a DWG in an output folder the commit makes, then the line
    // that marks the entry done.
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
    // The undo states of the rows and the DWG, then the rows, DEMO-001's
    // DWG, then the new mark, then the line that marks the entry done.
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
    // A fix: the undo states of all three, then the rows, DEMO-002's DWG,
    // then the mark taken away, then the line that marks the entry done. A
    // failed step keeps the mark with the sheet it marks.
    every_failed_step_puts_everything_back(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "notes"},
        ], "fixes": ["DEMO-002"]}),
        7,
    );
    assert!(!set.join(super::MARK_FILE).exists());
}

/// The template edit most tests here send: the undo state of the template
/// and of the three sheets' DWGs, then the template, then the DWGs.
fn template_move() -> Value {
    json!({"operations": [
        {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]},
    ]})
}

fn add_circle(sheet: &str, id: &str) -> Value {
    json!({"operations": [{"op": "add_circle", "sheet": sheet, "center": [5, 5, 0], "radius": 0.5, "layer": "notes", "id": id}]})
}

#[test]
fn a_failed_attempt_keeps_the_sheets_its_commit_could_not_write() {
    // The move reaches all three sheets; saving the first undo state, step
    // 0, fails, so none of them is written.
    let set = demo_set("attempt-sheets", true);
    let reply = apply(&set, &template_move(), Some(0));
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::WriteFailed);
    assert!(error.message.contains("an undo state"), "{}", error.message);
    let last = read_entries(&set).unwrap().pop().unwrap();
    assert_eq!(last.kind, EntryKind::FailedAttempt);
    assert!(last.landed_on.is_empty(), "nothing landed");
    let sheets: Vec<&str> = last.failed_on.iter().map(|f| f.sheet.as_str()).collect();
    assert_eq!(sheets, ["DEMO-001", "DEMO-002", "DEMO-003"]);
    for failed in &last.failed_on {
        assert_eq!(failed.code, "write_failed");
        assert_eq!(failed.message, error.message);
    }
    assert_eq!(last.error.unwrap().code, "write_failed");
}

#[test]
fn a_journal_that_cannot_be_cut_back_keeps_the_entry_pending_for_the_next_command() {
    let set = demo_set("cut-back", true);
    let journal = set.join(JOURNAL_FILE);
    let before = snapshot(&set);

    // The entry is written, its sync fails, and cutting it back fails too:
    // no file was put in place, but the journal may still hold the entry,
    // and the error says so and names it.
    FAIL_SYNC.with(|f| f.set(Some(true)));
    let reply = apply(&set, &template_move(), None);
    FAIL_SYNC.with(|f| f.set(None));
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::WriteFailed);
    assert!(
        !error.message.contains("nothing was changed"),
        "{}",
        error.message
    );
    assert!(
        error.message.contains(&format!(
            "the journal could not be cut back (a failed cut the test asked for), so it may still \
             hold entry 1, or part of it, pending; the next command rolls a whole entry back, and \
             part of one must be taken out of {}",
            journal.display()
        )),
        "{}",
        error.message
    );
    assert_eq!(error.file, Some(journal.display().to_string()));
    assert!(snapshot(&set) == before, "no drawing file was changed");
    let entries = read_entries(&set).unwrap();
    assert_eq!(entries.len(), 1, "no failed attempt after a pending entry");
    assert_eq!(entries[0].entry, 1);
    assert_eq!(entries[0].state, Some(CommitState::Pending));

    // The next change set rolls entry 1 back first, and says so; its own
    // entry then fails to sync and is cut back as it should be, so nothing
    // was changed, and it is recorded as a failed attempt.
    let length = fs::read(&journal).unwrap().len();
    FAIL_SYNC.with(|f| f.set(Some(false)));
    let reply = apply(&set, &template_move(), None);
    FAIL_SYNC.with(|f| f.set(None));
    let rolled = reply.rolled_back.expect("entry 1 was rolled back first");
    assert_eq!(
        (rolled.entry, rolled.kind),
        (Some(1), Some(EntryKind::ChangeSet))
    );
    assert!(rolled.files.is_empty(), "no file had been put in place");
    let error = reply.error.unwrap();
    assert!(
        error.message.contains("so nothing was changed"),
        "{}",
        error.message
    );
    assert!(snapshot(&set) == before);
    let entries = read_entries(&set).unwrap();
    assert_eq!(entries[0].state, Some(CommitState::RolledBack));
    let last = entries.last().unwrap();
    assert_eq!(last.kind, EntryKind::FailedAttempt);
    let cut = fs::read(&journal).unwrap();
    let added = String::from_utf8(cut[length..].to_vec()).unwrap();
    let added: Vec<Value> = added
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(added.len(), 2, "the rollback and the failed attempt");
    assert_eq!(added[0]["entry"], 1);
    assert_eq!(added[0]["state"], "rolled_back");
    assert_eq!(added[1]["kind"], "failed_attempt");
}

/// Runs `run` with its commit stopped at `step` as a killed process stops,
/// leaving everything it had written where it was: true when it stopped,
/// false when the commit finished before it reached that step.
fn stopped_at<T>(step: usize, run: impl FnOnce() -> T) -> bool {
    STOP_AT.with(|s| s.set(Some(step)));
    let outcome = panic::catch_unwind(AssertUnwindSafe(run));
    STOP_AT.with(|s| s.set(None));
    match outcome {
        Ok(_) => false,
        Err(payload)
            if payload
                .downcast_ref::<Stopped>()
                .is_some_and(|s| s.0 == step) =>
        {
            true
        }
        Err(payload) => panic::resume_unwind(payload),
    }
}

/// Runs command `name` of the registry on `input`, as every interface does.
fn command(name: &str, input: Value) -> JsonReply {
    find_command(name)
        .unwrap()
        .run(input, &Settings::default(), Interface::Cli)
}

/// The command run after the commit stopped at `step`: each of those that
/// read a set and write nothing to it in turn, so each is seen to roll back.
/// `build` writes nothing only to a set built already, as `before` holds it.
fn next_command(
    step: usize,
    set: &Path,
    before: &BTreeMap<PathBuf, Option<Vec<u8>>>,
) -> (&'static str, Value) {
    let built = before.contains_key(&set.join("dwg"));
    let commands: [(&str, Value); 8] = [
        ("journal", json!({"set": set})),
        ("query", json!({"source": set, "query": {"kind": "blocks"}})),
        ("check", json!({"source": set})),
        ("display-list", json!({"source": set})),
        (
            "change",
            json!({"set": set, "change_set": add_circle("DEMO-003", "drum"), "dry_run": true}),
        ),
        (
            "undo",
            json!({"set": set, "change_set": 1, "dry_run": true}),
        ),
        ("accept", json!({"set": set, "dry_run": true})),
        if built {
            ("build", json!({"source": set}))
        } else {
            ("journal", json!({"set": set}))
        },
    ];
    commands[step % commands.len()].clone()
}

/// Stops `run`, which commits entry `entry` of `kind`, at each step of its
/// commit in turn; each time the next command puts every file back as
/// `before` holds it, says so, and the journal marks the entry rolled back.
/// Gives how many steps there were, once `run` went through.
fn every_stopped_step_is_rolled_back(
    set: &Path,
    kind: EntryKind,
    before: &BTreeMap<PathBuf, Option<Vec<u8>>>,
    run: impl Fn(),
) -> usize {
    let mut step = 0;
    while stopped_at(step, &run) {
        let entries = read_entries(set).unwrap();
        let pending = entries.last().unwrap();
        assert_eq!(pending.state, Some(CommitState::Pending), "step {step}");
        assert_eq!(pending.kind, kind);
        let entry = pending.entry;

        let (name, input) = next_command(step, set, before);
        let reply = command(name, input);
        let Some(rolled) = reply.rolled_back.clone() else {
            panic!("step {step}: `{name}` rolled back first: {reply:?}");
        };
        assert_eq!(
            (rolled.entry, rolled.kind),
            (Some(entry), Some(kind)),
            "step {step}"
        );
        assert!(
            rolled
                .message
                .contains(&format!("entry {entry} of the set's journal")),
            "{}",
            rolled.message
        );
        assert!(
            snapshot(set) == *before,
            "step {step}: every file as it was before entry {entry}, after `{name}`"
        );
        let entries = read_entries(set).unwrap();
        let marked = entries.iter().find(|e| e.entry == entry).unwrap();
        assert_eq!(marked.state, Some(CommitState::RolledBack), "step {step}");
        // The journal says so plainly, a line of its own.
        let text = fs::read_to_string(set.join(JOURNAL_FILE)).unwrap();
        let last: Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        assert_eq!(
            (&last["entry"], &last["state"]),
            (&json!(entry), &json!("rolled_back"))
        );
        step += 1;
    }
    step
}

#[test]
fn a_change_set_stopped_at_any_step_of_its_commit_is_rolled_back_by_the_next_command() {
    let set = demo_set("stopped-change", true);
    let first = lands(&set, &add_circle("DEMO-001", "tank"));
    let before = snapshot(&set);
    let steps = every_stopped_step_is_rolled_back(&set, EntryKind::ChangeSet, &before, || {
        apply(&set, &template_move(), None);
    });
    // The pending entry, 4 undo states and 4 files written beside their
    // places, the undo states put in place, each of the template and the
    // DWGs moved aside and put in its place, and every file in place.
    assert_eq!(steps, 1 + 8 + 4 + 8 + 1);

    // The run that went through landed as a change set does.
    let entries = read_entries(&set).unwrap();
    let landed = entries.last().unwrap();
    assert_eq!(landed.kind, EntryKind::ChangeSet);
    assert_eq!(landed.state, Some(CommitState::Done));
    assert!(snapshot(&set) != before);
    // A further change set lands, and the journal lists every entry rolled
    // back, none of which can be undone.
    let further = lands(&set, &add_circle("DEMO-002", "drum"));
    let journal = command("journal", json!({"set": &set}));
    assert!(journal.rolled_back.is_none(), "nothing left to roll back");
    let report: Value = serde_json::from_str(journal.result.unwrap().get()).unwrap();
    assert_eq!(report["can_undo"], json!([first, landed.entry, further]));
    let rolled: Vec<&Value> = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["state"] == "rolled_back")
        .collect();
    assert_eq!(rolled.len(), steps);
    assert!(rolled.iter().all(|e| e.get("undo_state").is_none()));
    assert!(
        report["message"]
            .as_str()
            .unwrap()
            .contains("were rolled back, since their commits never finished"),
        "{}",
        report["message"]
    );
    // Its undo state and the one before are kept as they would have been.
    let report = undo(&UndoRequest {
        set: set.clone(),
        change_set: first,
        dry_run: false,
    });
    assert!(report.ok, "{:#?}", report.error);
}

#[test]
fn a_change_set_that_makes_a_folder_and_is_stopped_is_rolled_back_with_it() {
    // A set never built: the commit makes the output folder for its DWG.
    let set = demo_set("stopped-unbuilt", false);
    let before = snapshot(&set);
    let steps = every_stopped_step_is_rolled_back(&set, EntryKind::ChangeSet, &before, || {
        apply(&set, &add_circle("DEMO-001", "tank"), None);
    });
    assert!(steps > 4);
    assert!(set.join("dwg/DEMO-001.dwg").is_file());
}

#[test]
fn an_undo_stopped_at_any_step_is_rolled_back_and_its_change_sets_stay_in_place() {
    let set = demo_set("stopped-undo", true);
    let start = snapshot(&set);
    let first = lands(&set, &template_move());
    // The second lands on one sheet and marks the set inconsistent, so the
    // undo puts back the mark too.
    let partial = apply(
        &set,
        &json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
        None,
    );
    let second = partial.result.unwrap().journal_entry.unwrap();
    let before = snapshot(&set);
    let undoes = || {
        undo(&UndoRequest {
            set: set.clone(),
            change_set: first,
            dry_run: false,
        });
    };
    let steps = every_stopped_step_is_rolled_back(&set, EntryKind::Undo, &before, undoes);
    assert!(steps > 8, "{steps} steps");
    // Both change sets stayed in place each time; the undo that went
    // through put every file back as it was before the first.
    assert!(snapshot(&set) == start);
    let journal = command("journal", json!({"set": &set}));
    let report: Value = serde_json::from_str(journal.result.unwrap().get()).unwrap();
    let entries = report["entries"].as_array().unwrap();
    let undone: Vec<&Value> = entries.iter().filter(|e| e["kind"] == "undo").collect();
    assert_eq!(undone.len(), steps + 1);
    let done = undone.last().unwrap();
    assert_eq!(done["state"], "done");
    assert_eq!(done["undid"], json!([second, first]));
    for e in &undone[..steps] {
        assert_eq!(e["state"], "rolled_back");
    }
    // The undo state the undo saved went once it was done.
    assert!(!set.join(UNDO_STATES_FOLDER).exists());
    // A further change set lands.
    lands(&set, &add_circle("DEMO-003", "drum"));
}

#[test]
fn an_accept_stopped_at_any_step_is_rolled_back_and_issues_nothing() {
    let set = demo_set("stopped-accept", true);
    let first = lands(&set, &add_circle("DEMO-001", "tank"));
    let before = snapshot(&set);
    let accepts = || {
        accept_with(
            &AcceptRequest {
                set: set.clone(),
                despite_failed_checks: true,
                newest_seen: None,
                dry_run: false,
            },
            &Settings::default(),
            Interface::Cli,
        );
    };
    let steps = every_stopped_step_is_rolled_back(&set, EntryKind::Accept, &before, accepts);
    // The pending entry, and every file in place, of which it has none.
    assert_eq!(steps, 2);
    let entries = read_entries(&set).unwrap();
    let accept = entries.last().unwrap();
    assert_eq!(accept.kind, EntryKind::Accept);
    assert_eq!(accept.state, Some(CommitState::Done));
    assert_eq!(
        accept.issued,
        [first],
        "the rolled-back accepts issued nothing"
    );
    // A further change set lands, after the accept.
    let further = lands(&set, &add_circle("DEMO-002", "drum"));
    let journal = command("journal", json!({"set": &set}));
    let report: Value = serde_json::from_str(journal.result.unwrap().get()).unwrap();
    assert_eq!(report["can_undo"], json!([further]));
}

#[test]
fn every_command_that_reads_the_set_rolls_back_first_through_any_interface() {
    let set = demo_set("every-command", true);
    let outputs = |s: BTreeMap<PathBuf, Option<Vec<u8>>>| -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        s.into_iter()
            .filter(|(p, _)| !p.extension().is_some_and(|e| e == "pdf" || e == "png"))
            .collect()
    };
    let before = outputs(snapshot(&set));
    let commands: [(&str, Value); 10] = [
        (
            "change",
            json!({"set": &set, "change_set": add_circle("DEMO-003", "drum"), "dry_run": true}),
        ),
        ("undo", json!({"set": &set, "change_set": 1})),
        ("accept", json!({"set": &set, "dry_run": true})),
        ("journal", json!({"set": &set})),
        (
            "query",
            json!({"source": &set, "query": {"kind": "extents"}}),
        ),
        ("build", json!({"source": &set})),
        ("plot", json!({"source": &set})),
        ("check", json!({"source": &set})),
        ("display-list", json!({"source": &set, "sheet": "DEMO-002"})),
        // A file of the set, not its folder.
        ("render", json!({"source": set.join("dwg/DEMO-001.dwg")})),
    ];
    let interfaces = [
        Interface::Cli,
        Interface::Http,
        Interface::Mcp,
        Interface::Python,
        Interface::Viewer,
    ];
    for (i, (name, input)) in commands.into_iter().enumerate() {
        // Stopped with the template in place and the DWGs not yet: the
        // set's records and its DWGs disagree.
        assert!(stopped_at(14, || apply(&set, &template_move(), None)));
        assert!(outputs(snapshot(&set)) != before);
        let reply = find_command(name).unwrap().run(
            input,
            &Settings::default(),
            interfaces[i % interfaces.len()],
        );
        let rolled = reply
            .rolled_back
            .unwrap_or_else(|| panic!("`{name}` rolled back first"));
        assert_eq!(rolled.kind, Some(EntryKind::ChangeSet));
        assert_eq!(rolled.files.len(), 1, "`{name}`: the template was put back");
        assert!(outputs(snapshot(&set)) == before, "`{name}`");
    }
}

#[test]
fn a_rollback_that_cannot_complete_refuses_names_the_set_and_the_entry_and_changes_nothing_more() {
    let set = demo_set("cannot-roll-back", true);
    let before = snapshot(&set);
    assert!(stopped_at(16, || apply(&set, &template_move(), None)));
    let entry = read_entries(&set).unwrap().last().unwrap().entry;
    // The template's undo state is lost.
    let template = fs::read(set.join("templates/ansi-b.yaml")).unwrap();
    let states: Vec<PathBuf> = fs::read_dir(set.join(UNDO_STATES_FOLDER))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let original = demo_set_template();
    let lost = states
        .iter()
        .find(|p| fs::read(p).unwrap() == original)
        .unwrap()
        .clone();
    fs::rename(&lost, set.join("lost")).unwrap();
    let stopped = snapshot(&set);

    let reply = command(
        "query",
        json!({"source": &set, "query": {"kind": "blocks"}}),
    );
    assert!(!reply.ok);
    assert!(reply.rolled_back.is_none() && reply.result.is_none());
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::RollbackFailed);
    assert!(
        error.message.starts_with(&format!(
            "the drawing set `{}` cannot be read: entry {entry}, a change set, of its journal is \
             pending",
            set.display()
        )),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains("is missing or damaged; nothing more was changed"),
        "{}",
        error.message
    );
    assert_eq!(
        error.file,
        Some(set.join(JOURNAL_FILE).display().to_string())
    );
    assert!(snapshot(&set) == stopped, "nothing more was changed");
    assert_eq!(
        fs::read(set.join("templates/ansi-b.yaml")).unwrap(),
        template
    );
    let pending = read_entries(&set).unwrap();
    assert_eq!(pending.last().unwrap().state, Some(CommitState::Pending));

    // With the undo state back, the next command rolls it back.
    fs::rename(set.join("lost"), &lost).unwrap();
    let reply = command("check", json!({"source": &set}));
    assert_eq!(reply.rolled_back.unwrap().entry, Some(entry));
    assert!(snapshot(&set) == before);
}

/// The demo set's template, as the repository holds it.
fn demo_set_template() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-set/templates/ansi-b.yaml"),
    )
    .unwrap()
}

#[test]
fn a_command_waits_for_a_commit_still_running_and_does_not_roll_it_back() {
    let set = demo_set("still-running", true);
    // Stopped once every file is in place, just before its entry is marked
    // done: the commit's process is taken to be running still, and holds
    // the lock.
    assert!(stopped_at(21, || apply(&set, &template_move(), None)));
    let landed = snapshot(&set);
    let entry = read_entries(&set).unwrap().last().unwrap().entry;
    let held = lock(&set).unwrap();
    let reader = {
        let set = set.clone();
        std::thread::spawn(move || command("journal", json!({"set": set})))
    };
    std::thread::sleep(Duration::from_millis(300));
    assert!(!reader.is_finished(), "the command waits for the lock");
    // The commit finishes, marks its entry done, and lets the lock go.
    let journal = set.join(JOURNAL_FILE);
    let mut text = fs::read(&journal).unwrap();
    text.extend(state_line(None, entry, CommitState::Done, Vec::new()));
    fs::write(&journal, text).unwrap();
    drop(held);
    let reply = reader.join().unwrap();
    assert!(reply.ok);
    assert!(reply.rolled_back.is_none(), "nothing was rolled back");
    assert!(snapshot(&set) == landed);
    let entries = read_entries(&set).unwrap();
    assert_eq!(entries.last().unwrap().state, Some(CommitState::Done));
}

#[test]
fn a_rollback_refuses_when_a_file_was_changed_by_hand_since_the_commit_stopped() {
    // Zach's decision of 2026-11-11: the rollback refuses as undo does, and
    // changes nothing.
    let set = demo_set("hand-after-stop", true);
    let before = snapshot(&set);
    // The template and DEMO-001's DWG in place, the other DWGs not yet.
    assert!(stopped_at(16, || apply(&set, &template_move(), None)));
    let template = set.join("templates/ansi-b.yaml");
    let as_left = fs::read(&template).unwrap();
    fs::write(&template, [&as_left[..], b"# by hand\n"].concat()).unwrap();
    let edited = snapshot(&set);
    let journal = set.join(JOURNAL_FILE);
    let journal_before = fs::read(&journal).unwrap();

    let reply = command(
        "query",
        json!({"source": &set, "query": {"kind": "blocks"}}),
    );
    assert!(reply.rolled_back.is_none() && reply.result.is_none());
    let error = reply.error.unwrap();
    assert_eq!(error.code, ErrorCode::ChangedByHand);
    assert!(
        error.message.contains(&format!(
            "entry 1, a change set, of its journal is pending, since the process writing it \
             stopped before its commit finished, and rolling it back would lose changes made \
             since, by hand or by another program: `{}` (changed since entry 1's commit \
             stopped); nothing was changed",
            template.display()
        )),
        "{}",
        error.message
    );
    assert!(
        !error.message.contains("DEMO-00"),
        "the DWGs are as the commit left them: {}",
        error.message
    );
    assert!(snapshot(&set) == edited, "nothing was changed");
    assert_eq!(fs::read(&journal).unwrap(), journal_before);
    // Put back as its commit left it, the next command rolls it back.
    fs::write(&template, &as_left).unwrap();
    let reply = command("check", json!({"source": &set}));
    assert_eq!(reply.rolled_back.unwrap().entry, Some(1));
    assert!(snapshot(&set) == before);
}

#[test]
fn an_undo_is_worked_out_and_committed_holding_the_lock_so_no_change_set_lands_between() {
    let set = demo_set("undo-holds-lock", true);
    let first = lands(&set, &template_move());
    let template = set.join("templates/ansi-b.yaml");
    let as_first_left = fs::read(&template).unwrap();
    let other = json!({"operations": [
        {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.2, 0, 0]},
    ]});

    // Once the undo is worked out, another process sends a change set to
    // the same template, and the undo is killed once its entry is written.
    type Sent = (
        bool,
        std::thread::JoinHandle<super::Reply<super::ChangeReport>>,
    );
    let sent: Rc<RefCell<Option<Sent>>> = Rc::default();
    let hook = {
        let sent = Rc::clone(&sent);
        let set = set.clone();
        let other = other.clone();
        move || {
            let handle = std::thread::spawn(move || apply(&set, &other, None));
            for _ in 0..30 {
                if handle.is_finished() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            *sent.borrow_mut() = Some((handle.is_finished(), handle));
        }
    };
    AFTER_PLAN.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
    let undoes = || {
        undo(&UndoRequest {
            set: set.clone(),
            change_set: first,
            dry_run: false,
        })
    };
    assert!(stopped_at(0, undoes));
    let (landed_between, handle) = sent.borrow_mut().take().unwrap();
    assert!(
        !landed_between,
        "no change set lands between an undo's working out and its commit"
    );
    // It waited for the lock, then found the undo pending, and changed
    // nothing.
    let refused = handle.join().unwrap();
    assert_eq!(refused.error.unwrap().code, ErrorCode::RollbackFailed);
    assert_eq!(fs::read(&template).unwrap(), as_first_left);

    // The next command rolls the undo back, and loses nothing.
    let reply = command("journal", json!({"set": &set}));
    let rolled = reply.rolled_back.unwrap();
    assert_eq!(rolled.kind, Some(EntryKind::Undo));
    assert!(rolled.files.is_empty(), "the undo had replaced nothing");
    assert_eq!(fs::read(&template).unwrap(), as_first_left);
    // The other change set, sent again, lands.
    lands(&set, &other);
}

#[test]
fn a_last_line_written_in_part_is_cut_off_and_the_pending_entry_before_it_rolled_back() {
    let set = demo_set("torn-done", true);
    let before = snapshot(&set);
    // Every file in place, and the process killed half-way through the
    // line that marks the entry done.
    assert!(stopped_at(21, || apply(&set, &template_move(), None)));
    let journal = set.join(JOURNAL_FILE);
    let pending = fs::read(&journal).unwrap();
    let done = state_line(None, 1, CommitState::Done, Vec::new());
    fs::write(&journal, [&pending[..], &done[..done.len() / 2]].concat()).unwrap();

    // `query`, which reads no entry, rolls it back as `journal` would.
    let reply = command(
        "query",
        json!({"source": &set, "query": {"kind": "blocks"}}),
    );
    let rolled = reply.rolled_back.expect("rolled back first");
    assert_eq!(rolled.entry, Some(1));
    assert_eq!(rolled.cut_off_line, Some(2));
    assert_eq!(rolled.files.len(), 4, "every file was in place");
    assert!(
        rolled
            .message
            .contains("line 2 of the journal, written only in part after it, was cut off"),
        "{}",
        rolled.message
    );
    assert!(snapshot(&set) == before);
    let text = fs::read(&journal).unwrap();
    assert!(text.starts_with(&pending));
    let marked: Value = serde_json::from_slice(&text[pending.len()..]).unwrap();
    assert_eq!(
        (&marked["entry"], &marked["state"]),
        (&json!(1), &json!("rolled_back"))
    );
    let reply = command("journal", json!({"set": &set}));
    assert!(reply.ok && reply.rolled_back.is_none());
    lands(&set, &template_move());
}

#[test]
fn a_line_written_in_part_alone_is_cut_off_and_one_in_the_middle_is_refused() {
    let set = demo_set("torn-alone", true);
    lands(&set, &add_circle("DEMO-001", "tank"));
    let before = snapshot(&set);
    let journal = set.join(JOURNAL_FILE);
    let whole = fs::read(&journal).unwrap();
    // Killed while it wrote a change set's entry, before any file changed.
    let part = br#"{"entry":2,"kind":"change_set","state":"pend"#;
    fs::write(&journal, [&whole[..], &part[..]].concat()).unwrap();
    let reply = command("check", json!({"source": &set}));
    let rolled = reply.rolled_back.expect("the line was cut off");
    assert_eq!((rolled.entry, rolled.cut_off_line), (None, Some(3)));
    assert!(rolled.files.is_empty());
    assert!(snapshot(&set) == before);
    assert_eq!(fs::read(&journal).unwrap(), whole);

    // A line in part in the middle is left by no stop: refused, unread.
    fs::write(&journal, [&part[..], b"\n", &whole[..]].concat()).unwrap();
    let error = command("journal", json!({"set": &set})).error.unwrap();
    assert_eq!(error.code, ErrorCode::ReadFailed);
    assert_eq!(error.line, Some(1));
    let reply = apply(&set, &add_circle("DEMO-002", "drum"), None);
    assert_eq!(reply.error.unwrap().code, ErrorCode::ReadFailed);
    assert!(snapshot(&set) == before);
}

/// Whether the journal and the files agree on the newest change set: done,
/// with every file as it left them, or rolled back, with every file as it
/// found them.
fn agree(set: &Path) -> CommitState {
    let entries = read_entries(set).unwrap();
    let entry = entries
        .iter()
        .rev()
        .find(|e| e.kind == EntryKind::ChangeSet)
        .unwrap();
    let now = |f: &FileState| fs::read(set.join(&f.path)).ok().map(|b| fingerprint(&b));
    let state = entry.state.unwrap();
    for file in &entry.files {
        let wanted = match state {
            CommitState::Done => &file.after,
            CommitState::RolledBack => &file.before,
            CommitState::Pending => panic!("entry {} is still pending", entry.entry),
        };
        assert_eq!(
            &now(file),
            wanted,
            "{} disagrees with the journal",
            file.path
        );
    }
    state
}

#[test]
fn a_done_line_written_whole_that_cannot_be_synced_nor_taken_back_stands_with_every_file() {
    let set = demo_set("done-stands", true);
    // The done line's sync fails, and so does taking it back; a kill while
    // the files were put back would leave a journal that says done beside
    // them, so none is put back.
    FAIL_DONE.with(|f| f.set(Some(true)));
    STOP_RESTORE_AT.with(|s| s.set(Some(5)));
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| apply(&set, &template_move(), None)));
    FAIL_DONE.with(|f| f.set(None));
    STOP_RESTORE_AT.with(|s| s.set(None));
    let reply =
        outcome.unwrap_or_else(|_| panic!("files were put back while the journal said done"));
    let report = reply.result.unwrap_or_else(|| panic!("{:#?}", reply.error));
    assert_eq!(report.journal_entry, Some(1));
    let next = command("journal", json!({"set": &set}));
    assert!(next.rolled_back.is_none());
    assert_eq!(agree(&set), CommitState::Done);
}

#[test]
fn a_done_line_taken_back_keeps_the_entry_pending_while_the_files_go_back() {
    let set = demo_set("done-taken-back", true);
    let before = snapshot(&set);
    // The done line's sync fails and it is taken back: the files go back
    // with the entry pending, and the process is killed half-way.
    FAIL_DONE.with(|f| f.set(Some(false)));
    STOP_RESTORE_AT.with(|s| s.set(Some(5)));
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| apply(&set, &template_move(), None)));
    FAIL_DONE.with(|f| f.set(None));
    STOP_RESTORE_AT.with(|s| s.set(None));
    assert!(outcome.is_err(), "stopped while it put the files back");
    assert_eq!(
        read_entries(&set).unwrap().last().unwrap().state,
        Some(CommitState::Pending)
    );
    let reply = command("journal", json!({"set": &set}));
    assert_eq!(reply.rolled_back.unwrap().entry, Some(1));
    assert_eq!(agree(&set), CommitState::RolledBack);
    assert!(snapshot(&set) == before);
}
