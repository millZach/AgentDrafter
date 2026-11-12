//! Rolling back a commit that never finished: the entry a killed process
//! left pending in a drawing set's journal.
//!
//! Zach decided on 2026-11-11 that the next AgentDrafter command that reads
//! the set, whichever it is and through whichever interface, finds a pending
//! entry whose commit never finished, puts every file back byte for byte
//! from the saved undo state, as it was before that commit, marks the entry
//! rolled back in the journal, says so in its reply, and then goes on. If
//! the rollback cannot complete, the command refuses with an error that
//! names the set and the entry, and changes nothing more.
//!
//! A pending entry is the journal's newest. A commit in progress in another
//! process holds the set's journal lock, so a command that finds its entry
//! pending waits for it and reads the journal again: only an entry still
//! pending once the lock is free was left by a process that stopped. The
//! files go back through the commit module's own [`restore`], which a
//! commit that fails while its process runs on uses too.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::commit::{add_line, restore, Before};
use super::journal::{
    lock, pending, prune, read_entries, relative, state_line, CommitState, Entry, EntryKind,
    FileState, JOURNAL_FILE, UNDO_STATES_FOLDER,
};
use super::undo::{FileUndone, UndoAction};
use crate::error::{Error, ErrorCode};

/// What a command rolled back before it ran: an entry of the set's journal
/// whose commit never finished, since the process writing it stopped.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RolledBack {
    /// The drawing set's folder.
    pub set: String,
    /// The entry rolled back, by its number in the journal.
    pub entry: u64,
    /// What it was: a change set, an undo or an accept.
    pub kind: EntryKind,
    /// What was done, in words.
    pub message: String,
    /// Each file put back as it was before the entry (`restored`), or taken
    /// away since the entry's commit made it (`removed`).
    pub files: Vec<FileUndone>,
}

/// Rolls back the entry a commit that never finished left pending in the
/// journal of the set in `folder`, if there is one, and says what it did.
/// When there is none, or the commit is still running in another process
/// and finishes, it does nothing.
///
/// It refuses, with an error that names the set and the entry, when the
/// rollback cannot complete: an undo state it needs is missing or damaged,
/// or a file cannot be put back. Every file is then as it found them.
pub(crate) fn recover(folder: &Path) -> Result<Option<RolledBack>, Error> {
    if !newest_is_pending(folder) {
        return Ok(None);
    }
    let _lock = lock(folder).map_err(|e| cannot(folder, None, &e.message))?;
    let entries = read_entries(folder)?;
    let Some(entry) = pending(&entries) else {
        // The commit finished in the process that was writing it.
        return Ok(None);
    };
    if entries
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
    let files: Vec<FileState> = restored
        .iter()
        .map(|r| FileState {
            path: relative(folder, &r.path),
            before: r.found.clone(),
            after: r.back.clone(),
        })
        .collect();
    add_line(
        &folder.join(JOURNAL_FILE),
        &state_line(entry.entry, CommitState::RolledBack, files),
        &format!("the line that marks entry {} rolled back", entry.entry),
    )
    .map_err(|e| {
        cannot(
            folder,
            Some(entry),
            &format!(
                "every file was put back as it was before it, but {}; the next command marks it \
                 rolled back",
                e.message
            ),
        )
    })?;
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
    Ok(Some(RolledBack {
        set: folder.display().to_string(),
        entry: entry.entry,
        kind: entry.kind,
        message: format!(
            "entry {} of the set's journal, {}, was pending: the process writing it stopped \
             before its commit finished. It was rolled back: {put_back}, and the journal marks it \
             rolled back; it changed nothing. The command then ran as asked",
            entry.entry,
            kind_words(entry.kind)
        ),
        files,
    }))
}

/// Whether the newest line of the set's journal is an entry left pending:
/// a quick look, which reads no more than the journal's file, before the
/// lock is taken and the journal read whole.
fn newest_is_pending(folder: &Path) -> bool {
    #[derive(Deserialize)]
    struct Line {
        #[serde(default)]
        kind: Option<EntryKind>,
        #[serde(default)]
        state: Option<CommitState>,
    }
    let Ok(text) = fs::read(folder.join(JOURNAL_FILE)) else {
        return false;
    };
    let newest = text
        .split(|&b| b == b'\n')
        .rev()
        .find(|line| !line.trim_ascii().is_empty());
    newest
        .and_then(|line| serde_json::from_slice::<Line>(line).ok())
        .is_some_and(|line| line.kind.is_some() && line.state == Some(CommitState::Pending))
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
