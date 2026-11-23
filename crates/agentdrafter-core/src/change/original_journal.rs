//! Per-original write-ahead journal, crash recovery, stack undo, accept, and
//! issued-stamp lookup for original-DWG revisions (#71 Part 2).
//!
//! For an original DWG `<dir>/<stem>.dwg` (resolved through
//! [`crate::original::on_disk_path`]), its journal lives in
//! `<dir>/.agentdrafter/originals/<safe-stem>/` (`journal.jsonl`,
//! `journal.lock`, `undo-states/`), so multiple originals in the same folder
//! never collide while case aliases of the same file on a case-insensitive
//! volume share one journal.
//!
//! Safety invariants (ADR-0004, ADR-0005):
//! - The original DWG itself is NEVER written to, modified, or unlinked by
//!   `change`, `undo`, `accept`, or crash recovery.
//! - Symbolic links and Windows reparse points are NEVER followed, modified,
//!   or removed.
//! - `--dry-run` on `change` never acquires a write lock or creates
//!   `.agentdrafter/originals/`, and refuses if an unfinished pending entry or
//!   torn last line exists.
//! - `journal` on an original with no journal yet returns an empty report
//!   without creating any folder or file.
//! - Issued-stamp lookup for `render` and `plot` is strictly read-only.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[cfg(any(unix, windows))]
use crate::build::file_id_of_file;
use crate::build::{is_same_file, FileId};
use crate::check::check_dwg_bytes;
use crate::error::{Error, ErrorCode};
use crate::interface::Interface;
use crate::original::{is_dwg, on_disk_path, read_regular_no_follow, stem_of, NoFollowRead};
use crate::reply::Reply;
use crate::settings::Settings;
use crate::stamp::fingerprint;

use super::accept::{
    failed_words, issued, moved_on, relative_to, rule_text, AcceptReport, AcceptRequest,
    StampRevision,
};
use super::commit::{
    add_line, commit_new_files, commit_with_options, cut_back, moved_aside, restore_with_options,
    Ahead, Before, NewFile, Replacement,
};
use super::journal::{
    build_journal_report, code_text, fates, in_place, keeps_state, left_pending, line_of, listed,
    lock_at, next_number, pending, prune_at, read_entries_at, read_journal_at, read_text_at,
    state_line, undo_state_at, CheckFailed, CommitState, Entry, EntryKind, FileIssued, FileState,
    JournalReport, JournalRequest, ObjectChanged, OriginalRevisionMeta, Refusal, SheetFailed,
    KEPT_UNDO_STATES,
};
use super::recover::{kind_words, RolledBack};
use super::undo::{
    can_undo, message as undo_message, FileUndone, UndoAction, UndoReport, UndoRequest,
};

/// Folder, relative to an original DWG's parent folder, that holds per-original
/// revision journals (`<safe-stem>/journal.jsonl`, `journal.lock`,
/// `undo-states/`).
pub const ORIGINALS_JOURNAL_FOLDER: &str = ".agentdrafter/originals";

/// Per-original journal paths and identity of the original DWG.
pub(crate) struct OriginalJournal {
    /// The original DWG path as requested by the caller (for display).
    pub source: PathBuf,
    /// Canonical sheet name of the original (its on-disk stem).
    pub sheet: String,
    /// Parent folder of `source` (relative/display base for paths).
    pub base_dir: PathBuf,
    /// `<dir>/journal.jsonl`
    pub journal_file: PathBuf,
    /// `<dir>/journal.lock`
    pub lock_file: PathBuf,
    /// `<dir>/undo-states`
    pub undo_states_dir: PathBuf,
    /// File identity of the original DWG, captured without modifying it.
    pub source_id: Option<FileId>,
}

/// Deterministic, filesystem-safe, collision-free folder key for an original
/// DWG's canonical on-disk path (`<readable>-<hash12>`).
///
/// Uses the exact on-disk file name bytes from [`on_disk_path`] so:
/// - distinct originals in the same folder (`P-101.dwg`, `P-102.dwg`, or
///   special-character / case-distinct names on case-sensitive volumes) get
///   distinct folders;
/// - case aliases of the same file on a case-insensitive volume (`p-101.dwg`
///   and `P-101.dwg`) resolve to the exact same on-disk name and map to the
///   same folder;
/// - reserved device names (Windows `CON`, `NUL`, etc.) and special characters
///   are never emitted as folder names.
pub(crate) fn original_key(resolved: &Path) -> String {
    let file_name = resolved
        .file_name()
        .map(|n| n.as_encoded_bytes())
        .unwrap_or_default();
    let hash = &fingerprint(file_name)[..12];
    let stem = stem_of(resolved);
    let mut readable = String::new();
    let mut last_sep = true;
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            readable.push(c.to_ascii_lowercase());
            last_sep = false;
        } else if !last_sep {
            readable.push('_');
            last_sep = true;
        }
        if readable.len() >= 40 {
            break;
        }
    }
    let readable = readable.trim_matches('_');
    if readable.is_empty() {
        format!("dwg-{hash}")
    } else {
        format!("{readable}-{hash}")
    }
}

impl OriginalJournal {
    /// Constructs an [`OriginalJournal`] from an original DWG already opened
    /// and resolved via [`on_disk_path`]. Never creates any folder or file.
    pub(crate) fn from_opened(source: &Path, resolved: &Path, source_id: Option<FileId>) -> Self {
        let sheet = stem_of(resolved);
        let key = original_key(resolved);
        let base_dir = source
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let dir = base_dir.join(ORIGINALS_JOURNAL_FOLDER).join(key);
        let journal_file = dir.join("journal.jsonl");
        let lock_file = dir.join("journal.lock");
        let undo_states_dir = dir.join("undo-states");
        Self {
            source: source.to_path_buf(),
            sheet,
            base_dir,
            journal_file,
            lock_file,
            undo_states_dir,
            source_id,
        }
    }

    /// Opens `source` read-only to verify it exists and capture its identity,
    /// then computes its journal paths without creating any directory or file.
    pub(crate) fn open(source: &Path) -> Result<Self, Error> {
        let file = File::open(source).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the DWG: {e}"),
            )
            .in_file(source)
        })?;
        #[cfg(any(unix, windows))]
        let source_id = file_id_of_file(&file);
        #[cfg(not(any(unix, windows)))]
        let source_id: Option<FileId> = None;
        drop(file);
        let resolved = on_disk_path(source, source_id);
        Ok(Self::from_opened(source, &resolved, source_id))
    }

    /// Acquires this original's exclusive journal lock (`journal.lock`),
    /// creating its `.agentdrafter/originals/<key>/` folder if needed.
    pub(crate) fn lock(&self) -> Result<File, Error> {
        lock_at(&self.lock_file)
    }

    /// Computes a path relative to `self.base_dir` (with `/` separators) for
    /// recording in `journal.jsonl`.
    pub(crate) fn rel_path(&self, path: &Path) -> String {
        if self.base_dir.is_absolute() == path.is_absolute() {
            relative_to(&self.base_dir, path)
        } else {
            let base_abs = if self.base_dir.as_os_str().is_empty() {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            } else {
                std::path::absolute(&self.base_dir).unwrap_or_else(|_| self.base_dir.clone())
            };
            let path_abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
            relative_to(&base_abs, &path_abs)
        }
    }

    /// Resolves a path recorded in `journal.jsonl` against `self.base_dir`.
    pub(crate) fn resolve_file(&self, rel: &str) -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() || self.base_dir.as_os_str().is_empty() {
            p.to_path_buf()
        } else {
            self.base_dir.join(p)
        }
    }

    /// Read-only check for `--dry-run`: refuses if the original's journal holds
    /// an unfinished pending entry or a torn last line, without acquiring any
    /// lock or creating any directory.
    pub(crate) fn refuse_unfinished(&self) -> Result<(), Error> {
        let Some(read) = read_journal_at(&self.journal_file)? else {
            return Ok(());
        };
        let what = match (pending(&read.entries), &read.torn) {
            (Some(entry), _) => format!(
                "entry {}, {}, of its journal is pending: its commit has not finished, since \
                 another process is still writing it or the one that was stopped",
                entry.entry,
                kind_words(entry.kind)
            ),
            (None, Some(torn)) => format!(
                "line {} of its journal was written only in part, by a process that stopped while \
                 it wrote it",
                torn.line
            ),
            (None, None) => return Ok(()),
        };
        Err(Error::new(
            ErrorCode::RollbackFailed,
            format!(
                "a dry run on `{}` cannot proceed: {what}. A dry run writes nothing, so it does \
                 not roll that back; nothing was changed. Run `journal`, `change`, `undo`, or \
                 `accept` on `{}` to roll it back first; then try again",
                self.source.display(),
                self.source.display()
            ),
        )
        .in_file(&self.journal_file))
    }

    /// Recovers any unfinished pending entry or torn last line in this
    /// original's journal. Returns `Ok(None)` immediately without acquiring a
    /// lock or creating any directory when no journal exists or nothing is
    /// pending/torn.
    pub(crate) fn recover(&self) -> Result<Option<RolledBack>, Error> {
        let Some(read) = read_journal_at(&self.journal_file)? else {
            return Ok(None);
        };
        if pending(&read.entries).is_none() && read.torn.is_none() {
            return Ok(None);
        }
        let locked = self.lock().map_err(|e| self.cannot(None, &e.message))?;
        self.recover_locked(&locked)
    }

    /// Recovers any unfinished pending entry or torn last line while the caller
    /// already holds `self.lock()`.
    pub(crate) fn recover_locked(&self, _locked: &File) -> Result<Option<RolledBack>, Error> {
        let Some(read) = read_journal_at(&self.journal_file)? else {
            return Ok(None);
        };
        let Some(entry) = pending(&read.entries) else {
            let Some(torn) = read.torn else {
                return Ok(None);
            };
            cut_back(&self.journal_file, torn.at).map_err(|e| {
                self.cannot(
                    None,
                    &format!(
                        "line {} of the journal was written only in part, and could not be cut \
                         off ({e}); nothing was changed",
                        torn.line
                    ),
                )
            })?;
            return Ok(Some(RolledBack {
                set: self.source.display().to_string(),
                entry: None,
                kind: None,
                message: format!(
                    "line {} of `{}`'s journal was written only in part, by a process that \
                     stopped while it wrote it, before it changed any file: it was cut off. The \
                     command then ran as asked",
                    torn.line,
                    self.source.display()
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
            return Err(self.cannot(
                Some(entry),
                &format!(
                    "entries were added after it, by a program that does not roll back, so \
                     putting its files back would undo theirs too; nothing was changed. Undo \
                     them, or take entry {} and the lines after it out of the journal by hand",
                    entry.entry
                ),
            ));
        }

        // Verify every file in the pending entry is safe (not the original DWG,
        // not a symlink) and has not been modified by hand since the commit stopped.
        let changed = self.changed_since(entry)?;
        if !changed.is_empty() {
            return Err(Error::new(
                ErrorCode::ChangedByHand,
                format!(
                    "the journal of `{}` cannot be recovered: entry {}, {}, is pending, since the \
                     process writing it stopped before its commit finished, and rolling it back \
                     would lose changes made since, by hand or by another program: {}; nothing \
                     was changed. Put each file back as it was before entry {} or as its commit \
                     would have left it, and the next command rolls the entry back",
                    self.source.display(),
                    entry.entry,
                    kind_words(entry.kind),
                    changed.join(", "),
                    entry.entry
                ),
            )
            .in_file(&self.journal_file));
        }

        let before: Vec<Before> = entry
            .files
            .iter()
            .map(|f| Before {
                path: self.resolve_file(&f.path),
                print: f.before.clone(),
            })
            .collect();
        let made: Vec<PathBuf> = entry
            .folders_made
            .iter()
            .map(|f| self.resolve_file(f))
            .collect();
        let restored =
            restore_with_options(entry.entry, &before, &self.undo_states_dir, &made, false)
                .map_err(|e| {
                    self.cannot(
                        Some(entry),
                        &format!("{}; nothing more was changed", e.message),
                    )
                })?;

        let rest = |what: String| {
            self.cannot(
                Some(entry),
                &format!(
                    "every file was put back as it was before it, but {what}; the next command \
                     marks it rolled back"
                ),
            )
        };
        if let Some(torn) = read.torn {
            cut_back(&self.journal_file, torn.at).map_err(|e| {
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
                path: self.rel_path(&r.path),
                before: r.found.clone(),
                after: r.back.clone(),
            })
            .collect();
        add_line(
            &self.journal_file,
            &state_line(
                Some(&read.text),
                entry.entry,
                CommitState::RolledBack,
                files,
            ),
            &format!("the line that marks entry {} rolled back", entry.entry),
        )
        .map_err(|e| rest(e.message))?;
        if let Ok(entries) = read_entries_at(&self.journal_file) {
            prune_at(&self.undo_states_dir, &entries);
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
            0 => {
                "none of its files had been put in place, so no file needed putting back".to_owned()
            }
            1 => "1 file was put back as it was before it, byte for byte, from its undo state"
                .to_owned(),
            n => format!(
                "{n} files were put back as they were before it, byte for byte, from its undo \
                 state"
            ),
        };
        let cut = read.torn.map_or_else(String::new, |torn| {
            format!(
                "; line {} of the journal, written only in part after it, was cut off",
                torn.line
            )
        });
        Ok(Some(RolledBack {
            set: self.source.display().to_string(),
            entry: Some(entry.entry),
            kind: Some(entry.kind),
            message: format!(
                "entry {} of `{}`'s journal, {}, was pending: the process writing it stopped \
                 before its commit finished. It was rolled back: {put_back}, and the journal \
                 marks it rolled back{cut}; it changed nothing. The command then ran as asked",
                entry.entry,
                self.source.display(),
                kind_words(entry.kind)
            ),
            files,
            cut_off_line: read.torn.map(|torn| torn.line),
        }))
    }

    /// Commits an original-DWG revision write-ahead while holding `_locked`:
    /// appends `pending`, atomically places the new revision files with
    /// non-clobbering [`crate::original::place_new_file`], appends `done`, and
    /// prunes old undo states.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_revision(
        &self,
        _locked: &File,
        change_set: &Value,
        objects: Vec<ObjectChanged>,
        meta: OriginalRevisionMeta,
        new_files: &[(&Path, &[u8], &'static str)],
    ) -> Result<u64, Error> {
        let (text, entries_before) = read_text_at(&self.journal_file)?.unzip();
        let entries_before = entries_before.unwrap_or_default();
        if let Some(entry) = pending(&entries_before) {
            return Err(left_pending(entry));
        }
        let number = next_number(&entries_before);

        let files: Vec<FileState> = new_files
            .iter()
            .map(|(path, bytes, _)| FileState {
                path: self.rel_path(path),
                before: None,
                after: Some(fingerprint(bytes)),
            })
            .collect();
        let before: Vec<Before> = new_files
            .iter()
            .map(|(path, _, _)| Before {
                path: (*path).to_path_buf(),
                print: None,
            })
            .collect();
        let placed_specs: Vec<NewFile<'_>> = new_files
            .iter()
            .map(|(path, bytes, what)| NewFile { path, bytes, what })
            .collect();

        let fates = fates(&entries_before);
        let change_sets: Vec<&Entry> = entries_before
            .iter()
            .filter(|e| e.kind == EntryKind::ChangeSet && e.finished())
            .collect();
        let outside = (change_sets.len() + 1).saturating_sub(KEPT_UNDO_STATES);
        let pruned: Vec<u64> = change_sets[..outside]
            .iter()
            .filter(|e| keeps_state(e, &fates))
            .map(|e| e.entry)
            .collect();

        let mut entry = Entry {
            change_set: Some(change_set.clone()),
            landed_on: vec![self.sheet.clone()],
            files,
            pruned,
            objects,
            original_revision: Some(meta),
            ..Entry::new(number, EntryKind::ChangeSet)
        };
        entry.state = Some(CommitState::Pending);
        let pending_line = line_of(text.as_deref(), &entry);
        let done_line = state_line(None, number, CommitState::Done, Vec::new());

        let committed = commit_new_files(
            &Ahead {
                journal: &self.journal_file,
                entry: number,
                pending: &pending_line,
                done: &done_line,
                before: &before,
                states: &self.undo_states_dir,
                folders_made: &[],
            },
            &placed_specs,
            self.source_id,
        );
        if committed.is_ok() {
            let mut entries = entries_before;
            entry.state = Some(CommitState::Done);
            entries.push(entry);
            prune_at(&self.undo_states_dir, &entries);
        } else if let Ok(entries) = read_entries_at(&self.journal_file) {
            prune_at(&self.undo_states_dir, &entries);
        }
        committed?;
        Ok(number)
    }

    /// Records a failed revision attempt (refused decode gate, bad operation,
    /// verification failure, or write refusal) while holding `_locked`,
    /// changing no drawing files. Returns the recorded entry number if written.
    pub(crate) fn record_failed_attempt(
        &self,
        _locked: &File,
        change_set: &Value,
        error: &Error,
    ) -> Option<u64> {
        let read = read_text_at(&self.journal_file).ok()?;
        let (text, entries) = read.unzip();
        let entries = entries.unwrap_or_default();
        if pending(&entries).is_some() {
            return None;
        }
        let number = next_number(&entries);
        let entry = Entry {
            change_set: Some(change_set.clone()),
            failed_on: vec![SheetFailed {
                sheet: self.sheet.clone(),
                code: code_text(error.code),
                message: error.message.clone(),
            }],
            error: Some(Refusal {
                code: code_text(error.code),
                message: error.message.clone(),
                field: error.field.clone(),
            }),
            ..Entry::new(number, EntryKind::FailedAttempt)
        };
        let line = line_of(text.as_deref(), &entry);
        add_line(&self.journal_file, &line, "a failed attempt")
            .ok()
            .map(|()| number)
    }

    fn changed_since(&self, entry: &Entry) -> Result<Vec<String>, Error> {
        let mut changed = Vec::new();
        for (j, file) in entry.files.iter().enumerate() {
            let path = self.resolve_file(&file.path);
            let found = match read_regular_no_follow(&self.source, self.source_id, &path) {
                Ok(NoFollowRead::File(bytes)) => Some(fingerprint(&bytes)),
                Ok(NoFollowRead::Missing) => None,
                Ok(NoFollowRead::SameAsOriginal) => {
                    return Err(self.cannot(
                        Some(entry),
                        &format!(
                            "`{}` is the original DWG itself, which is never modified or removed \
                             (ADR-0004); nothing was changed",
                            path.display()
                        ),
                    ));
                }
                Ok(NoFollowRead::Symlink) => {
                    return Err(self.cannot(
                        Some(entry),
                        &format!(
                            "`{}` is a symbolic link; AgentDrafter never follows, modifies or \
                             removes a symbolic link; nothing was changed",
                            path.display()
                        ),
                    ));
                }
                Ok(NoFollowRead::NotRegular) => {
                    return Err(self.cannot(
                        Some(entry),
                        &format!(
                            "`{}` is not a regular file; nothing was changed",
                            path.display()
                        ),
                    ));
                }
                Err(e) => {
                    return Err(self.cannot(
                        Some(entry),
                        &format!(
                            "could not read {} to see whether it changed since: {e}; nothing was \
                             changed",
                            path.display()
                        ),
                    ));
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
            if found.is_none() && moved_aside(entry.entry, &path, j, &prints) {
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

    fn cannot(&self, entry: Option<&Entry>, why: &str) -> Error {
        let what = entry.map_or_else(
            || "an entry".to_owned(),
            |e| format!("entry {}, {},", e.entry, kind_words(e.kind)),
        );
        Error::new(
            ErrorCode::RollbackFailed,
            format!(
                "the journal of `{}` cannot be read: {what} is pending, since the process writing \
                 it stopped before its commit finished, and it could not be rolled back: {why}",
                self.source.display()
            ),
        )
        .in_file(&self.journal_file)
    }
}

// ── journal <original.dwg> ────────────────────────────────────────────────

pub(super) fn journal_original(request: &JournalRequest) -> Reply<JournalReport> {
    let oj = match OriginalJournal::open(&request.set) {
        Ok(oj) => oj,
        Err(error) => return Reply::from(Err(error)),
    };
    let rolled_back = match oj.recover() {
        Ok(rb) => rb,
        Err(error) => return Reply::from(Err(error)),
    };
    let entries = match read_entries_at(&oj.journal_file) {
        Ok(entries) => entries,
        Err(error) => return Reply::from(Err(error)).after(rolled_back),
    };
    let report = build_journal_report(
        &oj.source,
        &oj.journal_file,
        entries,
        None,
        "the journal is empty: no revision has been applied to the original",
    );
    Reply::from(Ok(report)).after(rolled_back)
}

// ── undo <original.dwg> <change-set> [--dry-run] ──────────────────────────

struct Plan {
    path: String,
    target: Option<String>,
    left: Option<String>,
    left_by: u64,
}

pub(super) fn undo_original(request: &UndoRequest) -> Reply<UndoReport> {
    let oj = match OriginalJournal::open(&request.set) {
        Ok(oj) => oj,
        Err(error) => return Reply::from(Err(error)),
    };
    // Acquire lock only on non-dry-run when a journal exists; recover under
    // the lock so no concurrent revision can interleave.
    let (locked, rolled_back) = if !request.dry_run && oj.journal_file.exists() {
        let lock = match oj.lock() {
            Ok(l) => l,
            Err(error) => return Reply::from(Err(error)),
        };
        match oj.recover_locked(&lock) {
            Ok(rb) => (Some(lock), rb),
            Err(error) => return Reply::from(Err(error)),
        }
    } else {
        match oj.recover() {
            Ok(rb) => (None, rb),
            Err(error) => return Reply::from(Err(error)),
        }
    };
    Reply::from(undo_original_inner(&oj, request, locked.as_ref())).after(rolled_back)
}

fn undo_original_inner(
    oj: &OriginalJournal,
    request: &UndoRequest,
    locked: Option<&File>,
) -> Result<UndoReport, Error> {
    let (text, entries) = read_text_at(&oj.journal_file)?.unzip();
    let entries = entries.unwrap_or_default();
    if let Some(entry) = pending(&entries) {
        return Err(left_pending(entry));
    }
    let fates = fates(&entries);
    let wanted = request.change_set;
    let refuse = |code: ErrorCode, why: String| {
        Error::new(code, format!("{why}; {}", can_undo(&entries, &fates))).at_field("change_set")
    };
    let Some(entry) = entries.iter().find(|e| e.entry == wanted) else {
        return Err(refuse(
            ErrorCode::UnknownName,
            format!("the original's journal has no entry {wanted}"),
        ));
    };
    if !entry.finished() {
        return Err(refuse(
            ErrorCode::BadValue,
            format!(
                "entry {wanted} was rolled back, since its commit never finished, so it changed \
                 nothing and there is nothing to undo"
            ),
        ));
    }
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
        EntryKind::Accept => {
            return Err(refuse(
                ErrorCode::BadValue,
                format!(
                    "entry {wanted} is an accept, which issued the revisions before it; an accept \
                     is not undone"
                ),
            ))
        }
        EntryKind::Discard => {
            return Err(refuse(
                ErrorCode::BadValue,
                format!(
                    "entry {wanted} is a proposal's discard, which changed nothing, so there is \
                     nothing to undo"
                ),
            ))
        }
    }
    if let Some(by) = fates.get(&wanted).and_then(|f| f.undone_by) {
        return Err(refuse(
            ErrorCode::BadValue,
            format!("change set {wanted} was undone already, by entry {by}"),
        ));
    }

    // Stack undo: entry `wanted` and every later active revision entry.
    let stack: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            e.kind == EntryKind::ChangeSet
                && e.finished()
                && e.entry >= wanted
                && fates.get(&e.entry).is_none_or(|f| f.undone_by.is_none())
        })
        .collect();
    let issued_list: Vec<(u64, u64)> = stack
        .iter()
        .filter_map(|e| Some((e.entry, fates.get(&e.entry)?.issued_by?)))
        .collect();
    if let Some(&(newest, by)) = issued_list.last() {
        let via = entries
            .iter()
            .find(|e| e.entry == by)
            .and_then(|e| e.via)
            .map_or_else(String::new, |via| format!(", through {}", via.words()));
        let why = if newest == wanted {
            format!(
                "change set {wanted} cannot be undone: it was issued by the accept in entry \
                 {by}{via}, and an issued revision cannot be undone; nothing was changed"
            )
        } else {
            format!(
                "change set {wanted} cannot be undone: undoing it undoes every later revision \
                 too, and change set {newest} was issued by the accept in entry {by}{via}, and an \
                 issued revision cannot be undone; nothing was changed"
            )
        };
        return Err(refuse(ErrorCode::Issued, why));
    }
    let pruned_list: Vec<(u64, u64)> = stack
        .iter()
        .filter_map(|e| Some((e.entry, fates.get(&e.entry)?.pruned_by?)))
        .collect();
    if let Some(&(first, by)) = pruned_list.first() {
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

    // Build per-file undo plans across the stack.
    let mut plans: Vec<Plan> = Vec::new();
    let mut changed: Vec<String> = Vec::new();
    for change_set in &stack {
        for file in &change_set.files {
            match plans.iter_mut().find(|p| p.path == file.path) {
                Some(plan) => {
                    if plan.left != file.before {
                        changed.push(format!(
                            "`{}` (changed between change set {} and change set {})",
                            oj.resolve_file(&file.path).display(),
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

    // Read and verify every file without following symlinks and refusing the
    // original DWG itself (ADR-0004).
    let mut now_bytes: Vec<Option<Vec<u8>>> = Vec::with_capacity(plans.len());
    let mut now_prints: Vec<Option<String>> = Vec::with_capacity(plans.len());
    for plan in &plans {
        let full = oj.resolve_file(&plan.path);
        match read_regular_no_follow(&oj.source, oj.source_id, &full) {
            Ok(NoFollowRead::SameAsOriginal) => {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "change set {wanted} cannot be undone: `{}` is the original DWG itself, \
                         which is never modified or removed (ADR-0004); nothing was changed",
                        full.display()
                    ),
                )
                .in_file(&full)
                .at_field("change_set"));
            }
            Ok(NoFollowRead::Symlink) => {
                return Err(Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "change set {wanted} cannot be undone: `{}` is a symbolic link; \
                         AgentDrafter never follows or removes a symbolic link; nothing was \
                         changed",
                        full.display()
                    ),
                )
                .in_file(&full)
                .at_field("change_set"));
            }
            Ok(NoFollowRead::NotRegular) => {
                return Err(Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "change set {wanted} cannot be undone: `{}` is not a regular file; \
                         nothing was changed",
                        full.display()
                    ),
                )
                .in_file(&full)
                .at_field("change_set"));
            }
            Ok(NoFollowRead::File(bytes)) => {
                let print = Some(fingerprint(&bytes));
                if print != plan.left {
                    let how = if plan.left.is_none() {
                        "made again"
                    } else {
                        "changed"
                    };
                    changed.push(format!(
                        "`{}` ({how} since change set {} left it)",
                        full.display(),
                        plan.left_by
                    ));
                }
                now_prints.push(print);
                now_bytes.push(Some(bytes));
            }
            Ok(NoFollowRead::Missing) => {
                if plan.left.is_some() {
                    changed.push(format!(
                        "`{}` (taken away since change set {} left it)",
                        full.display(),
                        plan.left_by
                    ));
                }
                now_prints.push(None);
                now_bytes.push(None);
            }
            Err(e) => {
                return Err(Error::new(
                    ErrorCode::ReadFailed,
                    format!(
                        "change set {wanted} cannot be undone: could not read `{}`: {e}; nothing \
                         was changed",
                        full.display()
                    ),
                )
                .in_file(&full)
                .at_field("change_set"));
            }
        }
    }
    if !changed.is_empty() {
        return Err(Error::new(
            ErrorCode::ChangedByHand,
            format!(
                "change set {wanted} cannot be undone: the undo would put back or remove files \
                 changed since, by hand or by another program, and lose those changes: {}; \
                 nothing was changed",
                changed.join(", ")
            ),
        )
        .at_field("change_set"));
    }

    let mut replacements: Vec<Replacement> = Vec::new();
    let mut states: Vec<FileState> = Vec::new();
    let mut files: Vec<FileUndone> = Vec::new();
    let mut undo_state_blobs: Vec<(String, Vec<u8>)> = Vec::new();
    for ((plan, print), current_bytes) in plans.iter().zip(&now_prints).zip(now_bytes) {
        if *print == plan.target {
            continue;
        }
        let target_bytes = match &plan.target {
            Some(target) => Some(undo_state_at(&oj.undo_states_dir, target).ok_or_else(|| {
                Error::new(
                    ErrorCode::ReadFailed,
                    format!(
                        "change set {wanted} cannot be undone: the undo state of `{}` is missing \
                         or damaged; nothing was changed",
                        oj.resolve_file(&plan.path).display()
                    ),
                )
                .in_file(&oj.undo_states_dir.join(target))
            })?),
            None => None,
        };
        if let (Some(p), Some(b)) = (print, current_bytes) {
            undo_state_blobs.push((p.clone(), b));
        }
        let full = oj.resolve_file(&plan.path);
        files.push(FileUndone {
            file: full.display().to_string(),
            action: if target_bytes.is_some() {
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
            path: full,
            bytes: target_bytes,
            what: "a revision file the undo removes",
        });
    }

    let undid: Vec<u64> = stack.iter().rev().map(|e| e.entry).collect();
    let msg = undo_message(request.dry_run, &undid, files.len());
    if request.dry_run {
        return Ok(UndoReport {
            set: oj.source.display().to_string(),
            dry_run: true,
            journal_entry: None,
            message: msg,
            undid,
            files,
            folders_removed: Vec::new(),
            inconsistent: None,
        });
    }

    // Commit write-ahead undo while holding the journal lock: save current
    // revision files' bytes into `undo-states/` first so a killed undo can be
    // rolled back byte-for-byte, then remove the revision files, then mark
    // done and prune undo states.
    let _owned_lock;
    let _locked_ref = match locked {
        Some(l) => l,
        None => {
            _owned_lock = oj.lock()?;
            &_owned_lock
        }
    };
    let number = next_number(&entries);
    let mut placing: Vec<Replacement> = Vec::new();
    for (print, bytes) in undo_state_blobs {
        let state_path = oj.undo_states_dir.join(&print);
        if state_path.is_file() || placing.iter().any(|r| r.path == state_path) {
            continue;
        }
        placing.push(Replacement {
            path: state_path,
            bytes: Some(bytes),
            what: "an undo state",
        });
    }
    let before: Vec<Before> = states
        .iter()
        .map(|f| Before {
            path: oj.resolve_file(&f.path),
            print: f.before.clone(),
        })
        .collect();
    placing.extend(replacements);
    let mut undo_entry = Entry {
        undid: undid.clone(),
        files: states,
        ..Entry::new(number, EntryKind::Undo)
    };
    undo_entry.state = Some(CommitState::Pending);
    let pending_line = line_of(text.as_deref(), &undo_entry);
    let done_line = state_line(None, number, CommitState::Done, Vec::new());

    let committed = commit_with_options(
        &Ahead {
            journal: &oj.journal_file,
            entry: number,
            pending: &pending_line,
            done: &done_line,
            before: &before,
            states: &oj.undo_states_dir,
            folders_made: &[],
        },
        &placing,
        false,
    );
    if committed.is_ok() {
        let mut entries_after = entries;
        undo_entry.state = Some(CommitState::Done);
        entries_after.push(undo_entry);
        prune_at(&oj.undo_states_dir, &entries_after);
    } else if let Ok(entries_now) = read_entries_at(&oj.journal_file) {
        prune_at(&oj.undo_states_dir, &entries_now);
    }
    committed?;

    Ok(UndoReport {
        set: oj.source.display().to_string(),
        dry_run: false,
        journal_entry: Some(number),
        message: msg,
        undid,
        files,
        folders_removed: Vec::new(),
        inconsistent: None,
    })
}

// ── accept <original.dwg> ─────────────────────────────────────────────────

enum OriginalAccepted {
    Done(AcceptReport),
    ChecksFailed(AcceptReport, Error),
}

pub(super) fn accept_original(
    request: &AcceptRequest,
    settings: &Settings,
    via: Interface,
) -> Reply<AcceptReport> {
    let oj = match OriginalJournal::open(&request.set) {
        Ok(oj) => oj,
        Err(error) => return Reply::from(Err(error)),
    };
    let (locked, rolled_back) = if !request.dry_run && oj.journal_file.exists() {
        let lock = match oj.lock() {
            Ok(l) => l,
            Err(error) => return Reply::from(Err(error)),
        };
        match oj.recover_locked(&lock) {
            Ok(rb) => (Some(lock), rb),
            Err(error) => return Reply::from(Err(error)),
        }
    } else {
        match oj.recover() {
            Ok(rb) => (None, rb),
            Err(error) => return Reply::from(Err(error)),
        }
    };
    let reply = match accept_original_inner(&oj, request, settings, via, locked.as_ref()) {
        Ok(OriginalAccepted::Done(report)) => Reply::from(Ok(report)),
        Ok(OriginalAccepted::ChecksFailed(report, error)) => Reply::partly(report, error),
        Err(error) => Reply::from(Err(error)),
    };
    reply.after(rolled_back)
}

fn accept_original_inner(
    oj: &OriginalJournal,
    request: &AcceptRequest,
    settings: &Settings,
    via: Interface,
    locked: Option<&File>,
) -> Result<OriginalAccepted, Error> {
    let (text, entries) = read_text_at(&oj.journal_file)?.unzip();
    let entries = entries.unwrap_or_default();
    if let Some(entry) = pending(&entries) {
        return Err(left_pending(entry));
    }
    let newest = entries.last().map_or(0, |e| e.entry);
    if let Some(seen) = request.newest_seen {
        if seen != newest {
            return Err(moved_on(&entries, seen, newest));
        }
    }

    let fates = fates(&entries);
    let active_unissued: Vec<&Entry> = in_place(&entries, &fates)
        .filter(|e| fates.get(&e.entry).is_none_or(|f| f.issued_by.is_none()))
        .collect();
    let last_accept = entries
        .iter()
        .rev()
        .find(|e| e.kind == EntryKind::Accept && e.finished());
    if active_unissued.is_empty() {
        return Err(if let Some(last) = last_accept {
            Error::new(
                ErrorCode::NothingToAccept,
                format!(
                    "there is nothing to accept: no revision has landed since the accept in entry \
                     {}; nothing was issued",
                    last.entry
                ),
            )
        } else {
            Error::new(
                ErrorCode::NothingToAccept,
                format!(
                    "there is nothing to accept: no revision of `{}` is in place to issue; \
                     nothing was issued",
                    oj.source.display()
                ),
            )
        });
    }

    // Verify every active un-issued revision DWG still exists, is not a
    // symlink or the original itself, and matches its recorded SHA-256.
    let mut revision_files: Vec<FileIssued> = Vec::with_capacity(active_unissued.len());
    let mut latest_dwg: Option<(PathBuf, Vec<u8>)> = None;
    for entry in &active_unissued {
        let Some((rev_rel, expected_sha)) = revision_dwg_of_entry(entry) else {
            return Err(Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "entry {} of `{}`'s journal does not name a revision DWG; nothing was issued",
                    entry.entry,
                    oj.source.display()
                ),
            )
            .in_file(&oj.journal_file));
        };
        let full = oj.resolve_file(&rev_rel);
        let bytes = match read_regular_no_follow(&oj.source, oj.source_id, &full) {
            Ok(NoFollowRead::File(bytes)) => {
                if fingerprint(&bytes) != expected_sha {
                    return Err(Error::new(
                        ErrorCode::ChangedByHand,
                        format!(
                            "the revision `{}` was changed outside AgentDrafter since change set \
                             {} wrote it; nothing was issued",
                            full.display(),
                            entry.entry
                        ),
                    )
                    .in_file(&full));
                }
                bytes
            }
            Ok(NoFollowRead::Missing) => {
                return Err(Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "the revision `{}` is missing since change set {} wrote it; nothing was \
                         issued",
                        full.display(),
                        entry.entry
                    ),
                )
                .in_file(&full));
            }
            Ok(NoFollowRead::SameAsOriginal) => {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "`{}` is the original DWG itself, not a revision; nothing was issued",
                        full.display()
                    ),
                )
                .in_file(&full));
            }
            Ok(NoFollowRead::Symlink) => {
                return Err(Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "the revision `{}` is a symbolic link; AgentDrafter never follows a \
                         symbolic link; nothing was issued",
                        full.display()
                    ),
                )
                .in_file(&full));
            }
            Ok(NoFollowRead::NotRegular) => {
                return Err(Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "the revision `{}` is not a regular file; nothing was issued",
                        full.display()
                    ),
                )
                .in_file(&full));
            }
            Err(e) => {
                return Err(Error::new(
                    ErrorCode::ReadFailed,
                    format!(
                        "could not read the revision `{}` to issue it: {e}; nothing was issued",
                        full.display()
                    ),
                )
                .in_file(&full));
            }
        };
        if !revision_files.iter().any(|f| f.path == rev_rel) {
            revision_files.push(FileIssued {
                path: rev_rel,
                sha256: expected_sha,
            });
        }
        latest_dwg = Some((full, bytes));
    }

    let (latest_path, latest_bytes) = latest_dwg.expect("active_unissued is non-empty");
    let checks = check_dwg_bytes(&latest_path, oj.sheet.clone(), &latest_bytes, settings)?;
    let failed: Vec<CheckFailed> = checks
        .findings
        .iter()
        .map(|f| CheckFailed {
            sheet: f.sheet.clone(),
            rule: rule_text(f.rule),
            id: f.object.as_ref().map(|o| o.id.clone()),
            message: f.message.clone(),
        })
        .collect();
    let numbers: Vec<u64> = active_unissued.iter().map(|e| e.entry).collect();
    let mut report = AcceptReport {
        set: oj.source.display().to_string(),
        dry_run: request.dry_run,
        journal_entry: None,
        via,
        message: String::new(),
        issued: numbers.clone(),
        sheets: vec![oj.sheet.clone()],
        follows: last_accept.map(|e| e.entry),
        checks,
        revision: revision_files,
    };

    if !failed.is_empty() && !request.despite_failed_checks {
        report.message = format!(
            "{}; nothing was issued. Accept again with despite_failed_checks to accept the \
             revision despite them",
            failed_words(&failed)
        );
        let error = Error::new(ErrorCode::ChecksFailed, report.message.clone());
        return Ok(OriginalAccepted::ChecksFailed(report, error));
    }

    let what = original_issues_words(&report);
    let despite = if failed.is_empty() {
        "Every check passes.".to_owned()
    } else {
        format!("It was accepted despite {}.", failed_words(&failed))
    };
    if request.dry_run {
        report.message =
            format!("a dry run, which wrote nothing: the accept would issue {what}. {despite}");
        return Ok(OriginalAccepted::Done(report));
    }

    let _owned_lock;
    let _locked_ref = match locked {
        Some(l) => l,
        None => {
            _owned_lock = oj.lock()?;
            &_owned_lock
        }
    };
    let number = next_number(&entries);
    if number != newest + 1 {
        return Err(Error::new(
            ErrorCode::MovedOn,
            format!(
                "the journal moved on while the accept was being worked out: its newest entry is \
                 now {}; nothing was issued",
                number - 1
            ),
        ));
    }
    let mut accept_entry = Entry {
        via: Some(via),
        issued: numbers,
        failed_checks: failed,
        revision: report.revision.clone(),
        ..Entry::new(number, EntryKind::Accept)
    };
    accept_entry.state = Some(CommitState::Pending);
    let pending_line = line_of(text.as_deref(), &accept_entry);
    let done_line = state_line(None, number, CommitState::Done, Vec::new());

    commit_with_options(
        &Ahead {
            journal: &oj.journal_file,
            entry: number,
            pending: &pending_line,
            done: &done_line,
            before: &[],
            states: &oj.undo_states_dir,
            folders_made: &[],
        },
        &[],
        false,
    )?;

    report.journal_entry = Some(number);
    report.message = format!(
        "accepted: issued {what}, recorded in entry {number}; an issued revision cannot be \
         undone. {despite}"
    );
    Ok(OriginalAccepted::Done(report))
}

fn revision_dwg_of_entry(entry: &Entry) -> Option<(String, String)> {
    if let Some(meta) = &entry.original_revision {
        return Some((meta.revision_dwg.clone(), meta.revision_sha256.clone()));
    }
    entry.files.iter().find_map(|f| {
        if is_dwg(Path::new(&f.path)) {
            f.after.as_ref().map(|sha| (f.path.clone(), sha.clone()))
        } else {
            None
        }
    })
}

fn original_issues_words(report: &AcceptReport) -> String {
    let files = match report.revision.len() {
        1 => "1 revision DWG".to_owned(),
        n => format!("{n} revision DWGs"),
    };
    let sheet = report.sheets.first().map_or("the original", String::as_str);
    match report.issued.as_slice() {
        [one] => format!("revision change set {one} on `{sheet}` ({files})"),
        more => format!(
            "revision change sets {} on `{sheet}` ({files})",
            listed(more)
        ),
    }
}

// ── Read-only issued-stamp lookup for render / plot of a revision DWG ─────

/// Strictly read-only lookup for `render <rev.dwg>` and `plot <rev.dwg>`:
/// checks whether `dwg` (with bytes `bytes`) matches a finished `accept` entry
/// in its parent folder's `.agentdrafter/originals/*/journal.jsonl`.
///
/// Never acquires a lock, never modifies or creates any file or directory.
pub(super) fn issued_of_revision_dwg(dwg: &Path, bytes: &[u8]) -> Option<StampRevision> {
    let parent = dwg
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let originals_dir = parent.join(ORIGINALS_JOURNAL_FOLDER);
    if !originals_dir.is_dir() {
        return None;
    }
    let mut subdirs: Vec<PathBuf> = fs::read_dir(&originals_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort();

    let print = fingerprint(bytes);
    let file_name = dwg.file_name().map(|n| n.to_string_lossy().into_owned());
    let mut best: Option<Entry> = None;

    for sub in subdirs {
        let journal_path = sub.join("journal.jsonl");
        let Ok(Some(read)) = read_journal_at(&journal_path) else {
            continue;
        };
        let found = read.entries.iter().rev().find(|e| {
            e.kind == EntryKind::Accept
                && e.finished()
                && e.revision.iter().any(|f| {
                    f.sha256 == print
                        && (file_name.as_deref() == Some(f.path.as_str())
                            || is_same_file(&parent.join(&f.path), dwg))
                })
        });
        if let Some(entry) = found {
            let replace = match &best {
                None => true,
                Some(prev) => (entry.at.as_str(), entry.entry) > (prev.at.as_str(), prev.entry),
            };
            if replace {
                best = Some(entry.clone());
            }
        }
    }

    best.as_ref().map(|entry| issued(Some(entry)))
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn test_pending_journal_path(source: &Path) -> Result<PathBuf, Error> {
    Ok(OriginalJournal::open(source)?.journal_file)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn test_original_key(resolved: &Path) -> String {
    original_key(resolved)
}

/// Ensures `io` is not unused when no OS-specific helper uses it directly.
const _: fn() -> io::ErrorKind = || io::ErrorKind::NotFound;
