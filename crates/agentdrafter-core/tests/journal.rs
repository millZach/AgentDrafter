//! The journal and undo on the demo set: every change set and failed
//! attempt is recorded in the set's folder; an undo puts every file a change
//! set wrote, and every later change set's, back byte for byte, the
//! inconsistent mark with them; old undo states are pruned so that the 50
//! newest change sets keep theirs; and an undo that would lose a change made
//! by hand is refused, naming the file.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, journal, undo, ChangeReport, ChangeRequest, EntryKind, ErrorCode,
    JournalReport, JournalRequest, Reply, SetBuildRequest, Settings, UndoAction, UndoReport,
    UndoRequest, UndoState, JOURNAL_FILE, KEPT_UNDO_STATES, MARK_FILE, UNDO_STATES_FOLDER,
};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;
#[path = "support/snapshot.rs"]
mod snapshot;

use snapshot::snapshot;

/// A copy of the demo set in a folder of its own, under Cargo's scratch
/// folder in `target/`, built when `built` says so.
fn demo_set(test: &str, built: bool) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-journal")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
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

fn apply(set: &Path, change_set: Value) -> Reply<ChangeReport> {
    change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set,
            dry_run: false,
        },
        &Settings::default(),
    )
}

/// Applies a change set that lands, and gives its entry in the journal.
fn lands(set: &Path, change_set: Value) -> u64 {
    let reply = apply(set, change_set);
    assert!(reply.ok, "{:#?}", reply.error);
    reply.result.unwrap().journal_entry.unwrap()
}

fn undo_of(set: &Path, change_set: u64, dry_run: bool) -> Reply<UndoReport> {
    undo(&UndoRequest {
        set: set.to_owned(),
        change_set,
        dry_run,
    })
}

/// Undoes a change set, which must be undone.
fn undoes(set: &Path, change_set: u64) -> UndoReport {
    let reply = undo_of(set, change_set, false);
    assert!(reply.ok, "{:#?}", reply.error);
    reply.result.unwrap()
}

fn read_journal(set: &Path) -> JournalReport {
    let reply = journal(&JournalRequest {
        set: set.to_owned(),
    });
    assert!(reply.ok, "{:#?}", reply.error);
    reply.result.unwrap()
}

fn add_circle(sheet: &str, id: &str) -> Value {
    json!({"operations": [{"op": "add_circle", "sheet": sheet, "center": [5, 5, 0], "radius": 0.5, "layer": "notes", "id": id}]})
}

#[test]
fn the_journal_lists_each_change_set_and_failed_attempt_with_its_operations_and_sheets() {
    let set = demo_set("lists", true);
    let moved = json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]}]});
    assert_eq!(lands(&set, moved.clone()), 1);

    // Lands on one sheet and fails on another.
    let partial = json!({"operations": [
        {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
        {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
    ]});
    let reply = apply(&set, partial.clone());
    assert_eq!(reply.error.unwrap().code, ErrorCode::SheetsFailed);
    assert_eq!(reply.result.unwrap().journal_entry, Some(2));

    // Refused whole: nothing to fail, nothing done.
    let refused = json!({"operations": [{"op": "nope"}]});
    let reply = apply(&set, refused.clone());
    assert!(reply.result.is_none());

    // Fails on every sheet it touches, so changes nothing.
    let failing = json!({"operations": [
        {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]},
        {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
    ]});
    let reply = apply(&set, failing.clone());
    let report = reply.result.unwrap();
    assert_eq!((report.landed, report.failed), (0, 3));
    assert_eq!(report.journal_entry, Some(4));

    // A dry run changes nothing and is not recorded.
    let dry = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: moved.clone(),
            dry_run: true,
        },
        &Settings::default(),
    );
    assert!(dry.ok);
    assert_eq!(dry.result.unwrap().journal_entry, None);

    let report = read_journal(&set);
    let kinds: Vec<(u64, EntryKind)> = report
        .entries
        .iter()
        .map(|e| (e.recorded.entry, e.recorded.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            (1, EntryKind::ChangeSet),
            (2, EntryKind::ChangeSet),
            (3, EntryKind::FailedAttempt),
            (4, EntryKind::FailedAttempt),
        ]
    );
    let [first, second, third, fourth] = &report.entries[..] else {
        panic!("four entries");
    };
    assert_eq!(first.recorded.change_set.as_ref(), Some(&moved));
    assert_eq!(
        first.recorded.landed_on,
        ["DEMO-001", "DEMO-002", "DEMO-003"]
    );
    let written: Vec<&str> = first
        .recorded
        .files
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(
        written,
        [
            "templates/ansi-b.yaml",
            "dwg/DEMO-001.dwg",
            "dwg/DEMO-002.dwg",
            "dwg/DEMO-003.dwg"
        ]
    );
    assert_eq!(first.undo_state, Some(UndoState::Kept));

    assert_eq!(second.recorded.change_set.as_ref(), Some(&partial));
    assert_eq!(second.recorded.landed_on, ["DEMO-001"]);
    assert_eq!(second.recorded.failed_on.len(), 1);
    assert_eq!(second.recorded.failed_on[0].sheet, "DEMO-002");
    assert_eq!(second.recorded.failed_on[0].code, "unknown_name");
    assert!(second
        .recorded
        .files
        .iter()
        .any(|f| f.path == MARK_FILE && f.before.is_none() && f.after.is_some()));

    assert_eq!(third.recorded.change_set.as_ref(), Some(&refused));
    let error = third.recorded.error.as_ref().unwrap();
    assert_eq!(error.code, "unknown_name");
    assert_eq!(error.field.as_deref(), Some("operations[0].op"));
    assert!(third.recorded.files.is_empty());
    assert_eq!(third.undo_state, None);

    assert_eq!(fourth.recorded.change_set.as_ref(), Some(&failing));
    let failed: Vec<&str> = fourth
        .recorded
        .failed_on
        .iter()
        .map(|f| f.sheet.as_str())
        .collect();
    assert_eq!(failed, ["DEMO-001", "DEMO-002", "DEMO-003"]);
    assert!(fourth.recorded.files.is_empty());

    assert_eq!(report.can_undo, [1, 2]);
    assert_eq!(report.kept_undo_states, 50);
    assert!(report.inconsistent.is_some(), "change set 2 marked the set");
    assert!(report
        .message
        .contains("2 change sets, 2 failed attempts and 0 undos"));

    // The file is JSON lines: an agent reads it a line at a time.
    let text = fs::read_to_string(set.join(JOURNAL_FILE)).unwrap();
    assert_eq!(text.lines().count(), 4);
    for (i, line) in text.lines().enumerate() {
        let entry: Value = serde_json::from_str(line).unwrap();
        assert_eq!(entry["entry"], i + 1);
    }
}

#[test]
fn undoing_an_earlier_change_set_undoes_every_later_one_byte_for_byte() {
    let set = demo_set("stack", true);
    let start = snapshot(&set);
    let first = lands(
        &set,
        json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]}]}),
    );
    let after_first = snapshot(&set);
    let second = lands(&set, add_circle("DEMO-001", "tank"));
    let third = lands(
        &set,
        json!({"operations": [{"op": "set_attribute", "target": {"name": "title-block", "sheet": "DEMO-002"}, "tag": "REV", "value": "B"}]}),
    );
    assert_eq!((first, second, third), (1, 2, 3));

    // A dry run says what would be put back, and writes nothing.
    let before_dry = snapshot(&set);
    let journal_before = fs::read(set.join(JOURNAL_FILE)).unwrap();
    let dry = undo_of(&set, second, true).result.unwrap();
    assert_eq!(dry.undid, [3, 2]);
    assert_eq!(dry.journal_entry, None);
    assert!(dry.message.starts_with("a dry run, which wrote nothing"));
    assert_eq!(snapshot(&set), before_dry);
    assert_eq!(fs::read(set.join(JOURNAL_FILE)).unwrap(), journal_before);

    // Undoing the second takes the third with it.
    let report = undoes(&set, second);
    assert_eq!(report.undid, [3, 2]);
    assert_eq!(report.journal_entry, Some(4));
    assert!(report
        .files
        .iter()
        .all(|f| f.action == UndoAction::Restored));
    assert_eq!(
        snapshot(&set),
        after_first,
        "every file as change set 2 found it"
    );
    assert!(
        report
            .message
            .contains("change sets 3 and 2, the newest first"),
        "{}",
        report.message
    );

    let journal = read_journal(&set);
    let states: Vec<(u64, Option<UndoState>, Option<u64>)> = journal
        .entries
        .iter()
        .map(|e| (e.recorded.entry, e.undo_state, e.undone_by))
        .collect();
    assert_eq!(
        states,
        [
            (1, Some(UndoState::Kept), None),
            (2, Some(UndoState::Undone), Some(4)),
            (3, Some(UndoState::Undone), Some(4)),
            (4, None, None),
        ]
    );
    assert_eq!(journal.entries[3].recorded.kind, EntryKind::Undo);
    assert_eq!(journal.entries[3].recorded.undid, [3, 2]);
    assert_eq!(journal.can_undo, [1]);

    // An undone change set is not undone again.
    let again = undo_of(&set, third, false).error.unwrap();
    assert_eq!(again.code, ErrorCode::BadValue);
    assert!(again.message.contains("undone already, by entry 4"));

    // A change set after the undo goes on the stack in their place.
    let fifth = lands(&set, add_circle("DEMO-003", "drum"));
    assert_eq!(fifth, 5);
    let report = undoes(&set, first);
    assert_eq!(report.undid, [5, 1]);
    assert_eq!(
        snapshot(&set),
        start,
        "every file as it was before change set 1"
    );
    assert!(
        !set.join(UNDO_STATES_FOLDER).exists(),
        "an undone change set's undo state is pruned with it"
    );
}

#[test]
fn undoing_a_change_set_puts_back_the_inconsistent_mark_it_caused() {
    let set = demo_set("mark", true);
    let start = snapshot(&set);
    let partial = apply(
        &set,
        json!({"operations": [
            {"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5},
            {"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"},
        ]}),
    );
    let failed = partial.result.unwrap().journal_entry.unwrap();
    assert!(set.join(MARK_FILE).is_file());
    let marked = snapshot(&set);
    let fix = lands(
        &set,
        json!({"operations": [{"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "notes"}], "fixes": ["DEMO-002"]}),
    );
    assert!(!set.join(MARK_FILE).exists());

    // Undoing the fix puts the mark back, byte for byte.
    let report = undoes(&set, fix);
    assert_eq!(snapshot(&set), marked);
    let mark = report.inconsistent.expect("the set is marked again");
    assert_eq!(mark.failures[0].sheet, "DEMO-002");

    // Undoing the change set that failed takes the mark away.
    let report = undoes(&set, failed);
    assert!(report.inconsistent.is_none());
    assert!(report
        .files
        .iter()
        .any(|f| f.file.ends_with("inconsistent.yaml") && f.action == UndoAction::Removed));
    assert!(!set.join(MARK_FILE).exists());
    assert_eq!(snapshot(&set), start);
    assert!(read_journal(&set).inconsistent.is_none());
}

#[test]
fn undo_takes_away_the_files_and_folders_a_change_set_made() {
    // A set never built: the change set makes the output folder and a DWG.
    let set = demo_set("made", false);
    let start = snapshot(&set);
    let entry = lands(&set, add_circle("DEMO-001", "tank"));
    assert!(set.join("dwg/DEMO-001.dwg").is_file());
    let report = undoes(&set, entry);
    assert!(report
        .files
        .iter()
        .any(|f| f.file.ends_with("DEMO-001.dwg") && f.action == UndoAction::Removed));
    assert_eq!(report.folders_removed.len(), 1);
    assert!(!set.join("dwg").exists());
    assert_eq!(snapshot(&set), start);
}

#[test]
fn old_undo_states_are_pruned_at_the_51st_change_set_and_the_journal_still_records_them() {
    assert_eq!(KEPT_UNDO_STATES, 50);
    let set = demo_set("prune", true);
    let rows_before = fs::read(set.join("rows.yaml")).unwrap();
    let first = lands(&set, add_circle("DEMO-001", "tank"));
    let after_first = snapshot(&set);
    // A failed attempt is no change set, and does not count.
    assert!(apply(&set, json!({"operations": [{"op": "nope"}]}))
        .result
        .is_none());
    let mut last = first;
    for _ in 1..KEPT_UNDO_STATES {
        last = lands(
            &set,
            json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/tank"}, "by": [0.01, 0, 0]}]}),
        );
    }
    assert_eq!(last, 51, "50 change sets and a failed attempt");
    let journal = read_journal(&set);
    assert_eq!(journal.can_undo.len(), 50);
    assert_eq!(journal.can_undo[0], first);
    assert!(journal.entries.iter().all(|e| e.pruned_by.is_none()));

    // The 51st change set prunes the first's undo state.
    let fifty_first = lands(
        &set,
        json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/tank"}, "by": [0.01, 0, 0]}]}),
    );
    let journal = read_journal(&set);
    let entry = |n: u64| {
        journal
            .entries
            .iter()
            .find(|e| e.recorded.entry == n)
            .unwrap()
    };
    assert_eq!(entry(fifty_first).recorded.pruned, [first]);
    let pruned = entry(first);
    assert_eq!(pruned.undo_state, Some(UndoState::Pruned));
    assert_eq!(pruned.pruned_by, Some(fifty_first));
    assert_eq!(pruned.recorded.landed_on, ["DEMO-001"], "still recorded");
    assert!(!pruned.recorded.files.is_empty());
    assert_eq!(journal.can_undo.len(), 50);
    assert!(!journal.can_undo.contains(&first));
    // The rows as they were before the first change set are needed by no
    // change set that keeps its undo state, so they are gone.
    let states: Vec<PathBuf> = fs::read_dir(set.join(UNDO_STATES_FOLDER))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(!states.iter().any(|p| fs::read(p).unwrap() == rows_before));

    // Undoing the first is refused, and says why; nothing changes.
    let before = snapshot(&set);
    let error = undo_of(&set, first, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::Pruned);
    assert!(
        error.message.contains(&format!(
            "change set {first} cannot be undone: its undo state was pruned when change set \
             {fifty_first} landed, since the undo states of the 50 newest change sets only are kept"
        )),
        "{}",
        error.message
    );
    assert_eq!(snapshot(&set), before);

    // The oldest change set that keeps its undo state can be undone, and
    // takes every later one with it.
    let report = undoes(&set, journal.can_undo[0]);
    assert_eq!(report.undid.len(), 50);
    assert_eq!(
        snapshot(&set),
        after_first,
        "every file as the first change set, which stays, left it"
    );
}

#[test]
fn undo_refuses_when_a_file_was_changed_by_hand_and_names_it() {
    let set = demo_set("by-hand", true);
    let entry = lands(&set, add_circle("DEMO-001", "tank"));
    let rows = set.join("rows.yaml");
    let left = fs::read(&rows).unwrap();
    fs::write(
        &rows,
        [left.as_slice(), b"# a note added by hand\n"].concat(),
    )
    .unwrap();
    let before = snapshot(&set);
    let journal_before = fs::read(set.join(JOURNAL_FILE)).unwrap();
    let error = undo_of(&set, entry, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::ChangedByHand);
    assert!(
        error.message.contains(&format!(
            "`{}` (changed since change set {entry} left it)",
            rows.display()
        )),
        "{}",
        error.message
    );
    assert!(
        !error.message.contains("DEMO-001.dwg"),
        "the DWG was not changed: {}",
        error.message
    );
    assert!(error.message.ends_with("nothing was changed"));
    assert_eq!(snapshot(&set), before);
    assert_eq!(
        fs::read(set.join(JOURNAL_FILE)).unwrap(),
        journal_before,
        "a refused undo is not recorded"
    );
    // Put back as the change set left it, the undo goes ahead.
    fs::write(&rows, &left).unwrap();
    undoes(&set, entry);
}

#[test]
fn a_file_the_change_set_made_and_someone_took_away_counts_as_changed() {
    let set = demo_set("taken-away", false);
    let entry = lands(&set, add_circle("DEMO-001", "tank"));
    let dwg = set.join("dwg/DEMO-001.dwg");
    fs::remove_file(&dwg).unwrap();
    let error = undo_of(&set, entry, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::ChangedByHand);
    assert!(
        error.message.contains(&format!(
            "`{}` (taken away since change set {entry} left it)",
            dwg.display()
        )),
        "{}",
        error.message
    );
    assert!(set.join("rows.yaml").is_file());
    assert!(fs::read_to_string(set.join("rows.yaml"))
        .unwrap()
        .contains("id: tank"));
}

#[test]
fn a_change_by_hand_between_two_change_sets_refuses_an_undo_that_would_lose_it() {
    let set = demo_set("between", true);
    let first = lands(&set, add_circle("DEMO-001", "tank"));
    let rows = set.join("rows.yaml");
    let by_hand = [fs::read(&rows).unwrap().as_slice(), b"# by hand\n"].concat();
    fs::write(&rows, &by_hand).unwrap();
    let second = lands(&set, add_circle("DEMO-002", "drum"));
    let error = undo_of(&set, first, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::ChangedByHand);
    assert!(
        error.message.contains(&format!(
            "`{}` (changed between change set {first} and change set {second})",
            rows.display()
        )),
        "{}",
        error.message
    );
    // The later one alone goes back to the rows as they were by hand.
    undoes(&set, second);
    assert_eq!(fs::read(&rows).unwrap(), by_hand);
    let error = undo_of(&set, first, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::ChangedByHand);
}

#[test]
fn an_entry_that_is_no_change_set_in_place_is_not_undone() {
    let set = demo_set("not-undone", true);
    let reply = apply(&set, json!({"operations": [{"op": "nope"}]}));
    assert!(reply.result.is_none());
    let error = undo_of(&set, 1, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::BadValue);
    assert!(
        error
            .message
            .starts_with("entry 1 is a failed attempt, which changed nothing"),
        "{}",
        error.message
    );
    let error = undo_of(&set, 9, false).error.unwrap();
    assert_eq!(error.code, ErrorCode::UnknownName);
    assert_eq!(error.field.as_deref(), Some("change_set"));
    assert!(
        error
            .message
            .ends_with("no change set of the set can be undone"),
        "{}",
        error.message
    );
    let entry = lands(&set, add_circle("DEMO-001", "tank"));
    let undone = undoes(&set, entry).journal_entry.unwrap();
    let error = undo_of(&set, undone, false).error.unwrap();
    assert!(
        error
            .message
            .contains("is an undo, and an undo is not undone"),
        "{}",
        error.message
    );
    // A folder that is not a drawing set has no journal.
    let error = journal(&JournalRequest {
        set: set.join("templates"),
    })
    .error
    .unwrap();
    assert_eq!(error.code, ErrorCode::ReadFailed);
}

#[test]
fn a_journal_line_that_is_no_entry_is_reported_with_its_line() {
    let set = demo_set("damaged", true);
    lands(&set, add_circle("DEMO-001", "tank"));
    let path = set.join(JOURNAL_FILE);
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("not json\n");
    fs::write(&path, text).unwrap();
    let error = journal(&JournalRequest { set: set.clone() }).error.unwrap();
    assert_eq!(error.code, ErrorCode::ReadFailed);
    assert_eq!(error.line, Some(2));
    // A change set does not land on a journal that cannot be read: it could
    // not be undone.
    let before = snapshot(&set);
    let reply = apply(&set, add_circle("DEMO-002", "drum"));
    assert_eq!(reply.error.unwrap().code, ErrorCode::ReadFailed);
    assert_eq!(snapshot(&set), before);
}
