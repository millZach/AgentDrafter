//! Undo: puts every file a change set wrote back as it was before it, in a
//! session as in a later one, from the journal alone.
//!
//! Undo is a stack (Zach's decision on #16): undoing a change set undoes
//! every later change set that is still in place too, the newest first, so
//! each file goes back to how it was before the oldest of them that wrote
//! it. Its records, its DWGs, the set's inconsistent mark, and files it
//! made, which are taken away, come back byte for byte, through the same
//! all-or-nothing step a change set lands through, and the undo is added to
//! the journal in that step.
//!
//! Undo refuses, and changes nothing, when it cannot put the bytes back:
//! when a change set it would undo has had its undo state pruned, and, as
//! Zach decided on 2026-11-11, when a file it would put back was changed
//! since the change set left it, by hand or by another program, which the
//! undo would lose. A file a change set made that has since been taken away
//! counts as changed. The refusal names the files.
//!
//! Nothing is issued yet, so every change set still in place can be undone.
//! Once the reviewer can accept, an issued change set cannot be undone.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::commit::Replacement;
use super::journal::{
    commit_undo, fates, keeps_state, listed, read_entries, read_state, undo_state, Entry,
    EntryKind, Fate, FileState, KEPT_UNDO_STATES, UNDO_STATES_FOLDER,
};
use super::{inconsistent, mark_report, Inconsistent, MARK_FILE};
use crate::description::{parse_mark, read_set_settings};
use crate::error::{Error, ErrorCode};
use crate::reply::Reply;
use crate::set::SET_SETTINGS_FILE;
use crate::stamp::fingerprint;

/// A request to undo a change set, and with it every later one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
    /// The change set to undo, by its entry's number in the journal.
    pub change_set: u64,
    /// Report what the undo would do, and write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// What an undo did, or would do in a dry run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UndoReport {
    /// The drawing set's folder.
    pub set: String,
    /// Whether this was a dry run, which wrote nothing.
    pub dry_run: bool,
    /// The journal entry that records the undo; absent for a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_entry: Option<u64>,
    /// What the undo did, in words.
    pub message: String,
    /// The change sets undone, the newest first.
    pub undid: Vec<u64>,
    /// Each file put back, or taken away because a change set made it.
    pub files: Vec<FileUndone>,
    /// The folders the change sets made that the undo took away, or would
    /// take away once it left them empty.
    pub folders_removed: Vec<String>,
    /// The set's inconsistent mark as it stands after the undo, or would
    /// after a dry run. Absent when the set is consistent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inconsistent: Option<Inconsistent>,
}

/// One file an undo put back.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FileUndone {
    pub file: String,
    pub action: UndoAction,
}

/// What an undo did to a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoAction {
    /// Its bytes were put back as they were before the change sets.
    Restored,
    /// It was taken away, since a change set made it.
    Removed,
}

/// Undoes a change set and every later one still in place, or reports what
/// that would do in a dry run.
///
/// The reply is `ok` with the report when every file was put back, and not
/// `ok`, with an error and nothing changed, when the undo is refused.
pub fn undo(request: &UndoRequest) -> Reply<UndoReport> {
    Reply::from(undo_set(request))
}

/// What one file goes back to.
struct Plan {
    /// Its path, relative to the set's folder.
    path: String,
    /// Its SHA-256 before the oldest change set undone that wrote it.
    target: Option<String>,
    /// Its SHA-256 as the newest change set undone that wrote it left it.
    left: Option<String>,
    /// That change set.
    left_by: u64,
}

fn undo_set(request: &UndoRequest) -> Result<UndoReport, Error> {
    let folder = &request.set;
    read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let entries = read_entries(folder)?;
    let fates = fates(&entries);
    let wanted = request.change_set;
    let refuse = |code: ErrorCode, why: String| {
        Error::new(code, format!("{why}; {}", can_undo(&entries, &fates))).at_field("change_set")
    };
    let Some(entry) = entries.iter().find(|e| e.entry == wanted) else {
        return Err(refuse(
            ErrorCode::UnknownName,
            format!("the set's journal has no entry {wanted}"),
        ));
    };
    match entry.kind {
        EntryKind::ChangeSet => {}
        EntryKind::FailedAttempt => {
            return Err(refuse(
                ErrorCode::BadValue,
                format!(
                    "entry {wanted} is a failed attempt, which changed nothing, so there is \
                     nothing to undo"
                ),
            ))
        }
        EntryKind::Undo => {
            return Err(refuse(
                ErrorCode::BadValue,
                format!("entry {wanted} is an undo, and an undo is not undone"),
            ))
        }
    }
    if let Some(by) = fates.get(&wanted).and_then(|f| f.undone_by) {
        return Err(refuse(
            ErrorCode::BadValue,
            format!("change set {wanted} was undone already, by entry {by}"),
        ));
    }

    // The change set and every later one still in place, the oldest first.
    let stack: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            e.kind == EntryKind::ChangeSet
                && e.entry >= wanted
                && fates.get(&e.entry).is_none_or(|f| f.undone_by.is_none())
        })
        .collect();
    let pruned: Vec<(u64, u64)> = stack
        .iter()
        .filter_map(|e| Some((e.entry, fates.get(&e.entry)?.pruned_by?)))
        .collect();
    if let Some(&(first, by)) = pruned.first() {
        let why = if first == wanted {
            format!(
                "change set {wanted} cannot be undone: its undo state was pruned when change \
                 set {by} landed, since the undo states of the {KEPT_UNDO_STATES} newest change \
                 sets only are kept; nothing was changed"
            )
        } else {
            format!(
                "change set {wanted} cannot be undone: undoing it undoes every later change set \
                 too, and the undo state of change set {first} was pruned when change set {by} \
                 landed, since the undo states of the {KEPT_UNDO_STATES} newest change sets only \
                 are kept; nothing was changed"
            )
        };
        return Err(Error::new(ErrorCode::Pruned, why).at_field("change_set"));
    }

    // Each file goes back to how it was before the oldest change set that
    // wrote it, and must be as the newest left it.
    let mut plans: Vec<Plan> = Vec::new();
    let mut changed: Vec<String> = Vec::new();
    for change_set in &stack {
        for file in &change_set.files {
            match plans.iter_mut().find(|p| p.path == file.path) {
                Some(plan) => {
                    if plan.left != file.before {
                        changed.push(format!(
                            "`{}` (changed between change set {} and change set {})",
                            shown(folder, &file.path),
                            plan.left_by,
                            change_set.entry
                        ));
                    }
                    plan.left.clone_from(&file.after);
                    plan.left_by = change_set.entry;
                }
                None => plans.push(Plan {
                    path: file.path.clone(),
                    target: file.before.clone(),
                    left: file.after.clone(),
                    left_by: change_set.entry,
                }),
            }
        }
    }
    let mut now: Vec<Option<String>> = Vec::with_capacity(plans.len());
    for plan in &plans {
        let state = read_state(&folder.join(&plan.path))?;
        let print = state.as_deref().map(fingerprint);
        if print != plan.left {
            let how = match (&print, &plan.left) {
                (None, _) => "taken away",
                (Some(_), None) => "made again",
                (Some(_), Some(_)) => "changed",
            };
            changed.push(format!(
                "`{}` ({how} since change set {} left it)",
                shown(folder, &plan.path),
                plan.left_by
            ));
        }
        now.push(print);
    }
    if !changed.is_empty() {
        return Err(Error::new(
            ErrorCode::ChangedByHand,
            format!(
                "change set {wanted} cannot be undone: the undo would put back files changed \
                 since, by hand or by another program, and lose those changes: {}; nothing was \
                 changed",
                changed.join(", ")
            ),
        )
        .at_field("change_set"));
    }

    // The bytes each file goes back to.
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut states: Vec<FileState> = Vec::new();
    let mut files: Vec<FileUndone> = Vec::new();
    let mut mark_after: Option<Option<Vec<u8>>> = None;
    for (plan, print) in plans.iter().zip(&now) {
        if *print == plan.target {
            continue;
        }
        let bytes = match &plan.target {
            Some(target) => Some(undo_state(folder, target).ok_or_else(|| {
                Error::new(
                    ErrorCode::ReadFailed,
                    format!(
                        "change set {wanted} cannot be undone: the undo state of `{}` is missing \
                         or damaged in {UNDO_STATES_FOLDER}; nothing was changed",
                        shown(folder, &plan.path)
                    ),
                )
                .in_file(&folder.join(UNDO_STATES_FOLDER).join(target))
            })?),
            None => None,
        };
        if plan.path == MARK_FILE {
            mark_after = Some(bytes.clone());
        }
        files.push(FileUndone {
            file: shown(folder, &plan.path),
            action: if bytes.is_some() {
                UndoAction::Restored
            } else {
                UndoAction::Removed
            },
        });
        states.push(FileState {
            path: plan.path.clone(),
            before: print.clone(),
            after: plan.target.clone(),
        });
        replacements.push(Replacement {
            path: folder.join(&plan.path),
            bytes,
            what: "a file the undo puts back",
        });
    }
    let undid: Vec<u64> = stack.iter().rev().map(|e| e.entry).collect();
    // The folders the change sets made, the deepest first.
    let mut folders: Vec<&String> = stack.iter().flat_map(|e| &e.folders_made).collect();
    folders.sort_by(|a, b| {
        b.split('/')
            .count()
            .cmp(&a.split('/').count())
            .then(a.cmp(b))
    });
    folders.dedup();

    let message = message(request.dry_run, &undid, files.len());
    if request.dry_run {
        let mark = match mark_after {
            Some(bytes) => bytes.and_then(|b| {
                let failures = parse_mark(&String::from_utf8_lossy(&b)).ok()?;
                (!failures.is_empty()).then(|| mark_report(folder, &failures))
            }),
            None => inconsistent(folder),
        };
        return Ok(UndoReport {
            set: folder.display().to_string(),
            dry_run: true,
            journal_entry: None,
            message,
            undid,
            files,
            folders_removed: folders.iter().map(|f| shown(folder, f)).collect(),
            inconsistent: mark,
        });
    }
    let number = commit_undo(folder, undid.clone(), states, &replacements)?;
    let mut folders_removed = Vec::new();
    for made in folders {
        let path = folder.join(made);
        if fs::remove_dir(&path).is_ok() || fs::symlink_metadata(&path).is_err() {
            folders_removed.push(shown(folder, made));
        }
    }
    Ok(UndoReport {
        set: folder.display().to_string(),
        dry_run: false,
        journal_entry: Some(number),
        message,
        undid,
        files,
        folders_removed,
        inconsistent: inconsistent(folder),
    })
}

/// The change sets an undo can take, in words.
fn can_undo(entries: &[Entry], fates: &BTreeMap<u64, Fate>) -> String {
    let can: Vec<u64> = entries
        .iter()
        .filter(|e| keeps_state(e, fates))
        .map(|e| e.entry)
        .collect();
    if can.is_empty() {
        "no change set of the set can be undone".to_owned()
    } else {
        format!(
            "the change sets an undo can take, each with every later one: {}",
            listed(&can)
        )
    }
}

/// A path relative to the set's folder, as a reply shows a file of the set.
fn shown(folder: &Path, path: &str) -> String {
    folder.join(path).display().to_string()
}

/// What an undo did, or would do, in words.
fn message(dry_run: bool, undid: &[u64], files: usize) -> String {
    let oldest = undid.last().copied().unwrap_or_default();
    let what = match undid {
        [one] => format!("change set {one}"),
        _ => format!(
            "change sets {}, the newest first, since an undo takes every later change set too",
            listed(undid)
        ),
    };
    let files = match files {
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    };
    if dry_run {
        format!(
            "a dry run, which wrote nothing: the undo would undo {what}, putting back {files} as \
             they were before change set {oldest}"
        )
    } else {
        format!(
            "undid {what}: {files} put back as they were before change set {oldest}, byte for \
             byte"
        )
    }
}
