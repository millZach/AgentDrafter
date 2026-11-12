//! Rolling back a commit that never finished: the entry a killed process
//! left pending in a drawing set's journal.
//!
//! Zach decided on 2026-11-11 that the next AgentDrafter command that reads
//! the set, whichever it is and through whichever interface, finds a pending
//! entry whose commit never finished, puts every file back byte for byte
//! from the saved undo state, as it was before that commit, marks the entry
//! rolled back in the journal, says so in its reply, and then goes on. If
//! the rollback cannot complete, the command refuses with an error that
//! names the set and the entry, and changes nothing more. And, as he decided
//! the same day, when a file the rollback would put back was changed by hand
//! since the commit stopped, the rollback refuses as undo does: it changes
//! nothing, and names the files and the entry.
//!
//! A pending entry is the journal's newest. A commit in progress in another
//! process holds the set's journal lock, so a command that finds its entry
//! pending waits for it and reads the journal again: only an entry still
//! pending once the lock is free was left by a process that stopped. The
//! files go back through the commit module's own [`restore`], which a
//! commit that fails while its process runs on uses too.
//!
//! A process can also stop while it writes a line of the journal, leaving
//! the last line written in part. That is a commit that never finished too:
//! the line is cut off, and the entry before it, if it is pending, is
//! rolled back. A line written in part anywhere but at the end is never
//! left by a stop, and the journal is refused as unreadable.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::commit::{add_line, cut_back, moved_aside, restore, Before};
use super::journal::{
    lock, pending, prune, read_entries, read_journal_file, relative, state_line, CommitState,
    Entry, EntryKind, FileState, JOURNAL_FILE, UNDO_STATES_FOLDER,
};
use super::undo::{FileUndone, UndoAction};
use crate::error::{Error, ErrorCode};
use crate::stamp::fingerprint;

/// What a command rolled back before it ran: an entry of the set's journal
/// whose commit never finished, since the process writing it stopped, or a
/// last line of the journal written only in part, cut off.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RolledBack {
    /// The drawing set's folder.
    pub set: String,
    /// The entry rolled back, by its number in the journal; none when only
    /// a line written in part was cut off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<u64>,
    /// What it was: a change set, an undo or an accept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<EntryKind>,
    /// What was done, in words.
    pub message: String,
    /// Each file put back as it was before the entry (`restored`), or taken
    /// away since the entry's commit made it (`removed`).
    pub files: Vec<FileUndone>,
    /// The last line of the journal, written only in part by a process that
    /// stopped while it wrote it, which was cut off: its number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cut_off_line: Option<usize>,
}

/// Rolls back the entry a commit that never finished left pending in the
/// journal of the set in `folder`, and cuts off a last line written in
/// part, if there is either, and says what it did. When there is neither,
/// or the commit is still running in another process and finishes, it does
/// nothing.
///
/// It refuses when the rollback cannot complete: an undo state it needs is
/// missing or damaged, or a file cannot be put back (`rollback_failed`); a
/// file was changed since the commit stopped (`changed_by_hand`); or the
/// journal cannot be read (`read_failed`). Every file is then as it found
/// them.
pub(crate) fn recover(folder: &Path) -> Result<Option<RolledBack>, Error> {
    if !worth_a_look(folder)? {
        return Ok(None);
    }
    let _lock = lock(folder).map_err(|e| cannot(folder, None, &e.message))?;
    let Some(read) = read_journal_file(folder)? else {
        return Ok(None);
    };
    let journal = folder.join(JOURNAL_FILE);
    let Some(entry) = pending(&read.entries) else {
        // No entry left pending: at most a line written in part, such as an
        // entry whose process stopped before it changed any file.
        let Some(torn) = read.torn else {
            // The commit finished in the process that was writing it.
            return Ok(None);
        };
        cut_back(&journal, torn.at).map_err(|e| {
            cannot(
                folder,
                None,
                &format!(
                    "line {} of the journal was written only in part, and could not be cut off \
                     ({e}); nothing was changed",
                    torn.line
                ),
            )
        })?;
        return Ok(Some(RolledBack {
            set: folder.display().to_string(),
            entry: None,
            kind: None,
            message: format!(
                "line {} of the set's journal was written only in part, by a process that \
                 stopped while it wrote it, before it changed any file of the set: it was cut \
                 off. The command then ran as asked",
                torn.line
            ),
            files: Vec::new(),
            cut_off_line: Some(torn.line),
        }));
    };
    if read
        .entries
        .last()
        .is_some_and(|newest| newest.entry != entry.entry)
    {
        return Err(cannot(
            folder,
            Some(entry),
            &format!(
                "entries were added after it, by a program that does not roll back, so putting \
                 its files back would undo theirs too; nothing was changed. Undo them, or take \
                 entry {} and the lines after it out of the journal by hand",
                entry.entry
            ),
        ));
    }
    let changed = changed_since(folder, entry)?;
    if !changed.is_empty() {
        return Err(Error::new(
            ErrorCode::ChangedByHand,
            format!(
                "the drawing set `{}` cannot be read: entry {}, {}, of its journal is pending, \
                 since the process writing it stopped before its commit finished, and rolling it \
                 back would lose changes made since, by hand or by another program: {}; nothing \
                 was changed. Put each file back as it was before entry {} or as its commit would \
                 have left it, and the next command rolls the entry back",
                folder.display(),
                entry.entry,
                kind_words(entry.kind),
                changed.join(", "),
                entry.entry
            ),
        )
        .in_file(&journal));
    }
    let before: Vec<Before> = entry
        .files
        .iter()
        .map(|f| Before {
            path: folder.join(&f.path),
            print: f.before.clone(),
        })
        .collect();
    let made: Vec<PathBuf> = entry.folders_made.iter().map(|f| folder.join(f)).collect();
    let restored = restore(
        entry.entry,
        &before,
        &folder.join(UNDO_STATES_FOLDER),
        &made,
    )
    .map_err(|e| {
        cannot(
            folder,
            Some(entry),
            &format!("{}; nothing more was changed", e.message),
        )
    })?;
    let rest = |what: String| {
        cannot(
            folder,
            Some(entry),
            &format!(
                "every file was put back as it was before it, but {what}; the next command marks \
                 it rolled back"
            ),
        )
    };
    if let Some(torn) = read.torn {
        cut_back(&journal, torn.at).map_err(|e| {
            rest(format!(
                "line {} of the journal, written only in part after it, could not be cut off \
                 ({e})",
                torn.line
            ))
        })?;
    }
    let files: Vec<FileState> = restored
        .iter()
        .map(|r| FileState {
            path: relative(folder, &r.path),
            before: r.found.clone(),
            after: r.back.clone(),
        })
        .collect();
    add_line(
        &journal,
        &state_line(
            Some(&read.text),
            entry.entry,
            CommitState::RolledBack,
            files,
        ),
        &format!("the line that marks entry {} rolled back", entry.entry),
    )
    .map_err(|e| rest(e.message))?;
    // The undo state the entry saved is needed by nothing now.
    if let Ok(entries) = read_entries(folder) {
        prune(folder, &entries);
    }
    let files: Vec<FileUndone> = restored
        .iter()
        .map(|r| FileUndone {
            file: r.path.display().to_string(),
            action: if r.back.is_some() {
                UndoAction::Restored
            } else {
                UndoAction::Removed
            },
        })
        .collect();
    let put_back = match files.len() {
        0 => "none of its files had been put in place, so no file needed putting back".to_owned(),
        1 => {
            "1 file was put back as it was before it, byte for byte, from its undo state".to_owned()
        }
        n => format!(
            "{n} files were put back as they were before it, byte for byte, from its undo state"
        ),
    };
    let cut = read.torn.map_or_else(String::new, |torn| {
        format!(
            "; line {} of the journal, written only in part after it, was cut off",
            torn.line
        )
    });
    Ok(Some(RolledBack {
        set: folder.display().to_string(),
        entry: Some(entry.entry),
        kind: Some(entry.kind),
        message: format!(
            "entry {} of the set's journal, {}, was pending: the process writing it stopped \
             before its commit finished. It was rolled back: {put_back}, and the journal marks it \
             rolled back{cut}; it changed nothing. The command then ran as asked",
            entry.entry,
            kind_words(entry.kind)
        ),
        files,
        cut_off_line: read.torn.map(|torn| torn.line),
    }))
}

/// The files `entry` wrote that a rollback cannot put back without losing a
/// change made since its commit stopped, in words. A file is as the commit
/// left it when it holds the bytes the entry records for it before, which
/// the commit had not replaced yet, or after, which it had; or when it is
/// missing where the commit leaves it so: one it had not made yet, one it
/// takes away, or one it had moved aside, which waits beside it. Anything
/// else was changed by hand or by another program.
fn changed_since(folder: &Path, entry: &Entry) -> Result<Vec<String>, Error> {
    let mut changed = Vec::new();
    for file in &entry.files {
        let path = folder.join(&file.path);
        let found = match fs::read(&path) {
            Ok(bytes) => Some(fingerprint(&bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(cannot(
                    folder,
                    Some(entry),
                    &format!(
                        "could not read {} to see whether it changed since: {e}; nothing was \
                         changed",
                        path.display()
                    ),
                ))
            }
        };
        if found == file.before || found == file.after {
            continue;
        }
        let prints: Vec<&str> = [&file.before, &file.after]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        if found.is_none() && moved_aside(entry.entry, &path, &prints) {
            continue;
        }
        let how = if found.is_some() {
            "changed"
        } else {
            "taken away"
        };
        changed.push(format!(
            "`{}` ({how} since entry {}'s commit stopped)",
            path.display(),
            entry.entry
        ));
    }
    Ok(changed)
}

/// Whether the set's journal may hold an entry left pending, or a last
/// line written in part: a quick look at its last line, before the lock is
/// taken and the journal read whole. A journal that cannot be read is
/// refused, as `journal` refuses it.
fn worth_a_look(folder: &Path) -> Result<bool, Error> {
    #[derive(Deserialize)]
    struct Line {
        #[serde(default)]
        kind: Option<EntryKind>,
        #[serde(default)]
        state: Option<CommitState>,
    }
    let path = folder.join(JOURNAL_FILE);
    let text = match fs::read(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "could not read the set's journal to see whether a commit is left pending: \
                     {e}"
                ),
            )
            .in_file(&path))
        }
    };
    let ends_whole = text.last().is_none_or(|&b| b == b'\n');
    let Some(last) = text
        .split(|&b| b == b'\n')
        .rev()
        .find(|line| !line.trim_ascii().is_empty())
    else {
        return Ok(false);
    };
    Ok(match serde_json::from_slice::<Line>(last) {
        Ok(line) => line.kind.is_some() && line.state == Some(CommitState::Pending),
        // A last line that is no JSON and ends with no line break was
        // written only in part.
        Err(_) => !ends_whole,
    })
}

/// What an entry is, in words.
fn kind_words(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::ChangeSet => "a change set",
        EntryKind::FailedAttempt => "a failed attempt",
        EntryKind::Undo => "an undo",
        EntryKind::Accept => "an accept",
    }
}

/// The error a command refuses with when it cannot roll back the entry a
/// commit that never finished left pending.
fn cannot(folder: &Path, entry: Option<&Entry>, why: &str) -> Error {
    let what = entry.map_or_else(
        || "an entry".to_owned(),
        |e| format!("entry {}, {},", e.entry, kind_words(e.kind)),
    );
    Error::new(
        ErrorCode::RollbackFailed,
        format!(
            "the drawing set `{}` cannot be read: {what} of its journal is pending, since the \
             process writing it stopped before its commit finished, and it could not be rolled \
             back: {why}",
            folder.display()
        ),
    )
    .in_file(&folder.join(JOURNAL_FILE))
}
