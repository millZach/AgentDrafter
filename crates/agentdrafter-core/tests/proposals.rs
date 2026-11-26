//! Proposals (#155): a change set that waits. `propose` records one beside
//! the journal and writes nothing else; `proposals` lists those waiting;
//! `keep` lands one through `change`'s own path, on the set as it stands,
//! and its journal entry says who proposed and who kept it; `discard` ends
//! one with a line in the journal that nothing undoes, and an undo passes
//! over. A proposal that has ended is not kept or discarded again.

use std::fs;
use std::path::{Path, PathBuf};

use agentdrafter_core::{
    build_set_with, change_with, discard, journal, keep_with, proposals, propose_with, undo,
    ChangeRequest, DiscardRequest, EntryKind, ErrorCode, Interface, JournalReport, JournalRequest,
    KeepRequest, ProposalsReport, ProposalsRequest, ProposeRequest, SetBuildRequest, Settings,
    UndoRequest, JOURNAL_FILE, LOCK_FILE, PROPOSALS_FOLDER,
};
use agentdrafter_core::{display_list_with, DisplayListRequest};
use serde_json::{json, Value};

#[path = "support/demo_set.rs"]
mod demo_set;
#[path = "support/snapshot.rs"]
mod snapshot;

/// A built copy of the demo set in a folder of its own.
fn demo_set(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("core-proposals")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    let set = dir.join("demo-set");
    demo_set::copy_demo_set(&set);
    let reply = build_set_with(
        &SetBuildRequest {
            set: set.clone(),
            overwrite: false,
            clean: None,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    set
}

/// The template's border moved on every sheet.
fn move_border() -> Value {
    json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]}]})
}

/// A circle added to one sheet's row.
fn add_circle(sheet: &str) -> Value {
    json!({"operations": [{"op": "add_circle", "sheet": sheet, "center": [5, 5, 0], "radius": 0.5}]})
}

fn propose(set: &Path, change_set: Value, by: Option<&str>) -> u64 {
    let reply = propose_with(
        &ProposeRequest {
            set: set.to_owned(),
            change_set,
            by: by.map(str::to_owned),
        },
        &Settings::default(),
        Interface::Mcp,
    );
    reply.expect("the proposal is recorded").proposal
}

fn waiting(set: &Path) -> ProposalsReport {
    proposals(&ProposalsRequest {
        set: set.to_owned(),
    })
    .expect("the proposals are listed")
}

fn numbers(set: &Path) -> Vec<u64> {
    waiting(set).proposals.iter().map(|p| p.proposal).collect()
}

fn keep(set: &Path, proposal: u64) -> agentdrafter_core::Reply<agentdrafter_core::KeepReport> {
    keep_with(
        &KeepRequest {
            set: set.to_owned(),
            proposal,
            by: Some("drafter".to_owned()),
        },
        &Settings::default(),
        Interface::Viewer,
    )
}

fn discarded(
    set: &Path,
    proposal: u64,
) -> Result<agentdrafter_core::DiscardReport, agentdrafter_core::Error> {
    discard(
        &DiscardRequest {
            set: set.to_owned(),
            proposal,
            by: None,
        },
        Interface::Viewer,
    )
}

fn lands(set: &Path, change_set: Value) -> u64 {
    let reply = change_with(
        &ChangeRequest {
            set: set.to_owned(),
            change_set,
            dry_run: false,
        },
        &Settings::default(),
    );
    assert!(reply.ok, "{:#?}", reply.error);
    reply.result.unwrap().journal_entry.unwrap()
}

fn journal_of(set: &Path) -> JournalReport {
    let reply = journal(&JournalRequest {
        set: set.to_owned(),
    });
    assert!(reply.ok, "{:#?}", reply.error);
    reply.result.unwrap()
}

/// Every file under the set's own folder, `.agentdrafter`.
fn own_files(set: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![set.join(".agentdrafter")];
    while let Some(dir) = stack.pop() {
        let Ok(listing) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in listing {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(
                    path.strip_prefix(set)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    found.sort();
    found
}

#[test]
fn a_proposal_is_recorded_listed_kept_and_another_discarded() {
    let set = demo_set("propose-keep-discard");
    let before = snapshot::snapshot(&set);

    let reply = propose_with(
        &ProposeRequest {
            set: set.clone(),
            change_set: move_border(),
            by: Some("Assistant".to_owned()),
        },
        &Settings::default(),
        Interface::Mcp,
    )
    .unwrap();
    assert_eq!(reply.proposal, 1);
    assert_eq!(reply.by, "Assistant");
    assert_eq!(reply.base_entry, 0, "made against an empty journal");
    assert_eq!(reply.sheets, ["DEMO-001", "DEMO-002", "DEMO-003"]);
    assert!(
        reply.would.dry_run,
        "its dry run says what keeping it would do"
    );
    assert_eq!(reply.would.landed, 3);

    // Nothing is written but the proposal: no journal, no lock, no file of
    // the set.
    assert_eq!(snapshot::snapshot(&set), before);
    assert_eq!(
        own_files(&set),
        [
            ".agentdrafter/proposals/1.json",
            ".agentdrafter/proposals/proposals.lock"
        ]
    );
    assert!(!set.join(JOURNAL_FILE).exists());
    assert!(!set.join(LOCK_FILE).exists());

    // With no `by`, the interface it came through.
    assert_eq!(propose(&set, add_circle("DEMO-002"), None), 2);
    let listed = waiting(&set);
    assert_eq!(listed.proposals.len(), 2, "{listed:#?}");
    let second = &listed.proposals[1];
    assert_eq!(second.by, "agent (MCP)");
    assert_eq!(second.sheets, ["DEMO-002"]);
    assert_eq!(second.change_set, add_circle("DEMO-002"));
    assert!(second.changed_since.is_empty());
    assert!(
        listed.message.contains("proposals 1 and 2 are waiting"),
        "{}",
        listed.message
    );

    // Kept: it lands as a change set, and its entry says who proposed it
    // and who kept it.
    let kept = keep(&set, 1);
    assert!(kept.ok, "{:#?}", kept.error);
    let kept = kept.result.unwrap();
    assert_eq!(
        (
            kept.proposal,
            kept.proposed_by.as_str(),
            kept.kept_by.as_str()
        ),
        (1, "Assistant", "drafter")
    );
    assert_eq!(kept.change.journal_entry, Some(1));
    assert_eq!(kept.change.landed, 3);
    assert_ne!(snapshot::snapshot(&set), before, "it landed");
    assert_eq!(numbers(&set), [2]);

    // Discarded: a line in the journal, and no file changed.
    let landed = snapshot::snapshot(&set);
    let gone = discarded(&set, 2).unwrap();
    assert_eq!(gone.journal_entry, 2);
    assert_eq!(
        (gone.proposed_by.as_str(), gone.discarded_by.as_str()),
        ("agent (MCP)", "drafter")
    );
    assert_eq!(snapshot::snapshot(&set), landed);
    assert!(numbers(&set).is_empty());
    assert_eq!(waiting(&set).message, "no proposal is waiting");

    // The journal's fields.
    let read = journal_of(&set);
    let entries: Vec<Value> = read
        .entries
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    assert_eq!(entries[0]["kind"], "change_set");
    assert_eq!(entries[0]["proposal"], 1);
    assert_eq!(entries[0]["proposed_by"], "Assistant");
    assert_eq!(entries[0]["kept_by"], "drafter");
    assert_eq!(entries[0]["change_set"], move_border());
    assert!(entries[0].get("discarded_by").is_none());
    assert_eq!(entries[1]["kind"], "discard");
    assert_eq!(entries[1]["proposal"], 2);
    assert_eq!(entries[1]["proposed_by"], "agent (MCP)");
    assert_eq!(entries[1]["discarded_by"], "drafter");
    for key in ["change_set", "kept_by", "files", "state"] {
        assert!(
            entries[1].get(key).is_none(),
            "a discard has no {key}: {}",
            entries[1]
        );
    }
    assert!(
        read.message.contains("1 discarded proposal"),
        "{}",
        read.message
    );
    assert_eq!(
        read.can_undo,
        [1],
        "a kept proposal is undone as any change set"
    );
}

#[test]
fn a_proposal_kept_after_the_set_changed_is_applied_to_the_set_as_it_stands() {
    let set = demo_set("kept-after-change");
    let proposal = propose(&set, move_border(), Some("Assistant"));
    let other = lands(&set, add_circle("DEMO-001"));
    let listed = waiting(&set);
    assert_eq!(listed.proposals[0].base_entry, 0);
    assert_eq!(listed.proposals[0].changed_since, [other]);

    let kept = keep(&set, proposal);
    assert!(kept.ok, "{:#?}", kept.error);
    // The circle that landed after it was proposed is still there, and the
    // border moved: it applied to the set as it stood.
    let rows = fs::read_to_string(set.join("rows.yaml")).unwrap();
    assert!(rows.contains("circle"), "{rows}");
    let template = fs::read_to_string(set.join("templates/ansi-b.yaml")).unwrap();
    assert!(template.contains("[0.6, 0.5, 0]"), "{template}");
}

#[test]
fn a_keep_the_set_refuses_is_a_failed_attempt_and_the_proposal_still_waits() {
    let set = demo_set("keep-refused");
    let moved = json!({"operations": [{"op": "move", "target": {"id": "DEMO-002/heading"}, "by": [0.5, 0, 0]}]});
    let proposal = propose(&set, moved.clone(), Some("Assistant"));
    // The text it moves is taken away from the template.
    lands(
        &set,
        json!({"operations": [{"op": "delete", "target": {"id": "DEMO-001/heading"}}]}),
    );
    let before = snapshot::snapshot(&set);
    let kept = keep(&set, proposal);
    assert!(!kept.ok, "{kept:#?}");
    assert_eq!(snapshot::snapshot(&set), before, "nothing landed");
    assert_eq!(numbers(&set), [proposal], "it still waits");

    // Recorded as a failed attempt, which names the proposal.
    let read = journal_of(&set);
    let last = serde_json::to_value(read.entries.last().unwrap()).unwrap();
    assert_eq!(last["kind"], "failed_attempt", "{last}");
    assert_eq!(last["proposal"], proposal);
    assert_eq!(last["kept_by"], "drafter");
    assert_eq!(last["change_set"], moved);

    // And it can still be discarded.
    assert!(discarded(&set, proposal).is_ok());
}

#[test]
fn a_proposal_that_has_ended_is_not_kept_or_discarded_again() {
    let set = demo_set("ended");
    let one = propose(&set, add_circle("DEMO-001"), None);
    let two = propose(&set, add_circle("DEMO-002"), None);
    assert!(keep(&set, one).ok);
    discarded(&set, two).unwrap();
    let entries = journal_of(&set).entries.len();

    for (proposal, how) in [
        (one, "kept, by drafter, as change set 1"),
        (two, "discarded, by drafter, in entry 2"),
    ] {
        let again = keep(&set, proposal);
        let error = again.error.unwrap();
        assert_eq!(error.code, ErrorCode::ProposalEnded);
        assert!(error.message.contains(how), "{}", error.message);
        assert!(again.result.is_none());
        let error = discarded(&set, proposal).unwrap_err();
        assert_eq!(error.code, ErrorCode::ProposalEnded);
    }
    assert_eq!(
        journal_of(&set).entries.len(),
        entries,
        "nothing was recorded"
    );

    let error = discarded(&set, 9).unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownName);
    assert!(
        error.message.contains("none is waiting"),
        "{}",
        error.message
    );
}

#[test]
fn undoing_a_kept_proposal_does_not_bring_it_back() {
    let set = demo_set("undo-kept");
    let before = snapshot::snapshot(&set);
    let proposal = propose(&set, move_border(), Some("Assistant"));
    let entry = keep(&set, proposal)
        .result
        .unwrap()
        .change
        .journal_entry
        .unwrap();
    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: entry,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(snapshot::snapshot(&set), before, "undone byte for byte");
    assert!(numbers(&set).is_empty(), "the proposal does not come back");
    assert_eq!(
        keep(&set, proposal).error.unwrap().code,
        ErrorCode::ProposalEnded
    );
}

#[test]
fn an_undo_passes_over_a_discard_and_a_discard_is_not_undone() {
    let set = demo_set("undo-over-discard");
    let first = lands(&set, add_circle("DEMO-001"));
    let proposal = propose(&set, add_circle("DEMO-002"), None);
    let line = discarded(&set, proposal).unwrap().journal_entry;
    let third = lands(&set, add_circle("DEMO-003"));
    assert_eq!((first, line, third), (1, 2, 3));

    let refused = undo(&UndoRequest {
        set: set.clone(),
        change_set: line,
        newest_seen: None,
        dry_run: false,
    });
    let error = refused.error.unwrap();
    assert_eq!(error.code, ErrorCode::BadValue);
    assert!(error.message.contains("discard"), "{}", error.message);

    let undone = undo(&UndoRequest {
        set: set.clone(),
        change_set: first,
        newest_seen: None,
        dry_run: false,
    });
    assert!(undone.ok, "{:#?}", undone.error);
    assert_eq!(undone.result.unwrap().undid, [third, first]);
    let read = journal_of(&set);
    assert_eq!(read.entries[1].recorded.kind, EntryKind::Discard);
    assert!(
        numbers(&set).is_empty(),
        "the discarded proposal stays ended"
    );
}

#[test]
fn a_change_set_the_set_would_refuse_is_not_proposed() {
    let set = demo_set("refused");
    let before = snapshot::snapshot(&set);
    for change_set in [
        json!({"operations": [{"op": "no_such_op"}]}),
        json!({"operations": [{"op": "move", "target": {"id": "DEMO-001/nothing"}, "by": [1, 0, 0]}]}),
        json!({"operations": [{"op": "add_circle", "sheet": "DEMO-001", "center": [5, 5, 0], "radius": 0.5, "layer": "nope"}]}),
    ] {
        let refused = propose_with(
            &ProposeRequest {
                set: set.clone(),
                change_set: change_set.clone(),
                by: None,
            },
            &Settings::default(),
            Interface::Cli,
        );
        assert!(refused.is_err(), "{change_set}: {refused:?}");
    }
    assert_eq!(snapshot::snapshot(&set), before);
    assert!(!set.join(PROPOSALS_FOLDER).exists(), "nothing was recorded");

    // Who it is must be words on one line.
    for by in ["", "  ", "two\nlines"] {
        let refused = propose_with(
            &ProposeRequest {
                set: set.clone(),
                change_set: add_circle("DEMO-001"),
                by: Some(by.to_owned()),
            },
            &Settings::default(),
            Interface::Cli,
        );
        assert_eq!(refused.unwrap_err().code, ErrorCode::Usage, "{by:?}");
    }
}

#[test]
fn a_proposal_waits_through_an_accept() {
    let set = demo_set("through-accept");
    let proposal = propose(&set, add_circle("DEMO-002"), None);
    lands(&set, add_circle("DEMO-001"));
    let accepted = agentdrafter_core::accept_with(
        &agentdrafter_core::AcceptRequest {
            set: set.clone(),
            despite_failed_checks: true,
            newest_seen: None,
            dry_run: false,
        },
        &Settings::default(),
        Interface::Cli,
    );
    assert!(accepted.ok, "{:#?}", accepted.error);
    assert_eq!(numbers(&set), [proposal]);
    let kept = keep(&set, proposal);
    assert!(kept.ok, "{:#?}", kept.error);
    assert_eq!(
        journal_of(&set).can_undo,
        [kept.result.unwrap().change.journal_entry.unwrap()]
    );
}

#[test]
fn a_keep_lands_nothing_when_any_sheet_it_reaches_would_refuse_it() {
    // Two independent edits: a circle on DEMO-001's row, and a move of a
    // circle on DEMO-002's row, whose layer is then locked.
    let set = demo_set("keep-whole");
    lands(
        &set,
        json!({"operations": [{"op": "add_circle", "sheet": "DEMO-002", "center": [5, 5, 0], "radius": 0.5, "layer": "notes", "id": "washer"}]}),
    );
    let both = json!({"operations": [
        {"op": "add_circle", "sheet": "DEMO-001", "center": [7, 5, 0], "radius": 0.4},
        {"op": "move", "target": {"id": "DEMO-002/washer"}, "by": [0.5, 0, 0]}
    ]});
    // Proposed while DEMO-002's DWG cannot be written: its dry run, as a
    // keep would land it, holds DEMO-001 back too, and no operation lands.
    let dwg = set.join("dwg/DEMO-002.dwg");
    let mut permissions = fs::metadata(&dwg).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&dwg, permissions.clone()).unwrap();
    let proposed = propose_with(
        &ProposeRequest {
            set: set.clone(),
            change_set: both.clone(),
            by: Some("Assistant".to_owned()),
        },
        &Settings::default(),
        Interface::Mcp,
    )
    .unwrap();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&dwg, permissions).unwrap();
    let would = &proposed.would;
    assert_eq!((would.landed, would.failed), (0, 2), "{would:#?}");
    let held = would.sheets.iter().find(|s| s.sheet == "DEMO-001").unwrap();
    assert_eq!(
        held.error.as_ref().unwrap().code,
        ErrorCode::SharedEditFailed
    );
    assert!(held.result.is_none(), "nothing of it is built");
    assert!(
        would.operations.iter().all(|o| !o.ok),
        "{:#?}",
        would.operations
    );
    assert!(
        proposed
            .message
            .contains("holds back 1 other sheet it reaches"),
        "{}",
        proposed.message
    );
    let proposal = proposed.proposal;
    lands(
        &set,
        json!({"operations": [{"op": "change_layer", "sheet": "DEMO-001", "name": "notes", "set": {"locked": true}}]}),
    );
    // Its preview, as the viewer asks for a proposal's: DEMO-001 held
    // back, not would land; without all_or_none, as change applies it.
    let shown = |all_or_none: bool| {
        let mut request = DisplayListRequest::new(&set);
        request.sheet = Some("DEMO-001".to_owned());
        request.change_set = Some(both.clone());
        request.all_or_none = all_or_none;
        let drawn = display_list_with(&request, &Settings::default()).unwrap();
        let sheet = drawn
            .preview
            .unwrap()
            .change
            .sheets
            .into_iter()
            .find(|s| s.sheet == "DEMO-001")
            .unwrap();
        sheet.error.map(|e| (e.code, e.message))
    };
    let (code, message) = shown(true).expect("held back");
    assert_eq!(code, ErrorCode::SharedEditFailed);
    assert_eq!(
        message,
        "left as it was: a proposal is kept on every sheet it reaches or on none, and \
         `DEMO-002` would refuse it"
    );
    assert!(
        shown(false).is_none(),
        "as change applies it, DEMO-001 would land"
    );
    let before = snapshot::snapshot(&set);
    let kept = keep(&set, proposal);
    assert!(!kept.ok);
    assert_eq!(kept.error.as_ref().unwrap().code, ErrorCode::SheetsFailed);
    let report = kept.result.unwrap().change;
    assert_eq!((report.landed, report.failed), (0, 2), "{report:#?}");
    let one = report
        .sheets
        .iter()
        .find(|s| s.sheet == "DEMO-001")
        .unwrap();
    assert_eq!(
        one.error.as_ref().unwrap().code,
        ErrorCode::SharedEditFailed
    );
    assert!(one
        .error
        .as_ref()
        .unwrap()
        .message
        .contains("`DEMO-002` would refuse it"));
    let two = report
        .sheets
        .iter()
        .find(|s| s.sheet == "DEMO-002")
        .unwrap();
    assert_eq!(two.error.as_ref().unwrap().code, ErrorCode::Locked);
    // DEMO-001 was never built, and no operation landed: the circle's is
    // held back, the move's refused by the lock.
    assert!(one.result.is_none() && !one.dwg_written, "{one:#?}");
    assert!(
        report.operations.iter().all(|o| !o.ok),
        "{:#?}",
        report.operations
    );
    let circle = report.operations[0].error.as_ref().unwrap();
    assert_eq!(circle.code, ErrorCode::SharedEditFailed);
    assert!(
        circle.message.contains("`DEMO-002` would refuse it"),
        "{}",
        circle.message
    );
    assert!(report.operations[0].record.is_none() && report.operations[0].after.is_none());
    assert_eq!(
        report.operations[1].error.as_ref().unwrap().code,
        ErrorCode::Locked
    );
    assert_eq!(
        snapshot::snapshot(&set),
        before,
        "nothing landed on DEMO-001 either"
    );
    assert_eq!(numbers(&set), [proposal], "it still waits");
    let last = serde_json::to_value(journal_of(&set).entries.last().unwrap()).unwrap();
    assert_eq!(last["kind"], "failed_attempt", "{last}");
    assert_eq!(last["proposal"], proposal);
    assert_eq!(last["failed_on"].as_array().unwrap().len(), 2, "{last}");
    assert!(last.get("landed_on").is_none());
    // A change set sent as one still lands where it can, as before.
    let partly = change_with(
        &ChangeRequest {
            set: set.clone(),
            change_set: both,
            dry_run: false,
        },
        &Settings::default(),
    );
    assert_eq!(partly.result.unwrap().landed, 1);
}

#[test]
fn a_dwg_open_elsewhere_does_not_stop_a_template_edit_being_proposed() {
    // DEMO-002's DWG cannot be written, as one open in AutoCAD: a template
    // edit fails there and holds back DEMO-001 and DEMO-003. The records
    // would take it, so it is proposed, and the reply says what holds it.
    let set = demo_set("dwg-open");
    let dwg = set.join("dwg/DEMO-002.dwg");
    let mut permissions = fs::metadata(&dwg).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&dwg, permissions.clone()).unwrap();
    let reply = propose_with(
        &ProposeRequest {
            set: set.clone(),
            change_set: move_border(),
            by: Some("Assistant".to_owned()),
        },
        &Settings::default(),
        Interface::Mcp,
    )
    .expect("proposed, though a DWG cannot be written now");
    assert_eq!(reply.sheets, ["DEMO-001", "DEMO-002", "DEMO-003"]);
    assert!(
        reply
            .message
            .contains("the DWG of `DEMO-002` cannot be written now")
            && reply
                .message
                .contains("holds back 2 other sheets it reaches"),
        "{}",
        reply.message
    );
    // Kept while it cannot be written: refused whole, still waiting.
    assert!(!keep(&set, reply.proposal).ok);
    assert_eq!(numbers(&set), [reply.proposal]);
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(&dwg, permissions).unwrap();
    let kept = keep(&set, reply.proposal);
    assert!(kept.ok, "{:#?}", kept.error);
    assert_eq!(kept.result.unwrap().change.landed, 3);
}

#[test]
fn by_and_the_proposal_number_are_refused_as_the_schema_refuses_them() {
    let set = demo_set("by");
    let long = "x".repeat(79);
    for (by, ok) in [
        (format!(" {long}"), true),
        (format!("  {long}"), false),
        ("tab\there".to_owned(), false),
        ("\u{85}next".to_owned(), false),
        ("\u{a0}".to_owned(), false),
    ] {
        let reply = propose_with(
            &ProposeRequest {
                set: set.clone(),
                change_set: add_circle("DEMO-001"),
                by: Some(by.clone()),
            },
            &Settings::default(),
            Interface::Cli,
        );
        assert_eq!(reply.is_ok(), ok, "{by:?}: {reply:?}");
    }
    let zero = keep(&set, 0);
    assert_eq!(zero.error.unwrap().code, ErrorCode::Usage);
    assert_eq!(discarded(&set, 0).unwrap_err().code, ErrorCode::Usage);
}
