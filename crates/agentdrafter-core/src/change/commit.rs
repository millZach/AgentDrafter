//! Puts a change set's files in place together: the records it edited, the
//! DWGs of the sheets it landed on, and the set's inconsistent mark, written
//! or taken away, with the undo state it saves, its entry in the set's
//! journal written ahead of them and marked done after. An undo puts its
//! files back through the same step, and so does an accept, which has none.
//!
//! Write-ahead, as Zach decided on 2026-11-11: the entry is added to the end
//! of the journal and synced, marked pending, before any file is put in
//! place, and a line marking it done is added once all are. Then, first,
//! every new file is written whole beside its place, and the undo state put
//! in place. Then, one file at a time, the file already there is moved aside
//! and the new one moved into its place. The files moved aside are taken
//! away, and the entry is marked done. A process killed anywhere between
//! leaves the entry pending, and the next command that reads the set puts
//! every file back from the undo state with [`restore`] and marks the entry
//! rolled back (`recover.rs`).
//!
//! If any step fails while the process runs on, every file is put back as it
//! was through the same [`restore`], and the journal is cut back to its
//! length before, so on disk too a change set lands whole or not at all: no
//! sheet is left with new records beside its old DWG, and no record two
//! sheets share changes for one of them only (ADR-0005), and the mark never
//! lags the sheets. A journal that cannot be cut back keeps the entry
//! pending, and the next command rolls it back.
//!
//! The journal never says done unless every file is in place: files are put
//! back only while the journal holds the entry pending, so a process killed
//! while it puts them back leaves the entry for the next command to roll
//! back. When the line that marks it done cannot be written, what the
//! journal holds decides: a done line that stands whole, every file being in
//! place, stands; one taken back leaves the entry pending, and the files are
//! put back; one that may stand in part is left for the next command, which
//! cuts a line written in part off the journal and rolls the entry back.
//!
//! The files a commit writes beside their places are named by its entry and
//! by the file they stand for, `.agentdrafter-entry-<entry>-f<j>.tmp` and
//! `.old` for the entry's file `j`, as the journal lists its files
//! (`back-f<j>` for a rollback's own, `s<i>` for an undo state), so a
//! rollback knows what a killed commit left beside each file, and whose it
//! is, and takes it away.

use std::cmp::Reverse;
use std::fs;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::build::FileId;
use crate::error::{Error, ErrorCode};
use crate::original::{
    create_scoped_temp_beside, ensure_real_dir_chain, move_to_unique_backup_beside,
    open_no_follow_write, place_new_file, read_regular_no_follow, rename_no_replace_keep_src,
    verify_no_symlink_intermediate_dirs, verify_real_dir_chain, NoFollowRead,
};
use crate::stamp::fingerprint;

/// Safety context for original-DWG revision commits, undos, and crash recovery:
/// scopes temp/backup files to this original's full canonical path token and
/// enforces non-symlink, non-original file-identity checks throughout.
#[derive(Clone, Copy)]
pub(super) struct OriginalGuard<'a> {
    /// Per-folder journal key (`<readable>-<namehash12>`).
    pub key: &'a str,
    /// Full-path-scoped temp/backup prefix key (`<key>-<fullpathhash16>`),
    /// unique even when two originals in different folders share a filename
    /// and write into the same output directory.
    pub temp_scope: &'a str,
    /// Directory containing `.agentdrafter/originals/<key>/`.
    pub base_dir: &'a Path,
    /// Canonical parent directory of the original DWG.
    pub orig_dir_real: &'a Path,
    pub source: &'a Path,
    pub source_id: Option<FileId>,
}

/// One file of a pending original-DWG entry already inspected in a single
/// non-following read, carrying both its SHA-256 fingerprint and opened
/// [`FileId`] so crash recovery performs zero second reads.
pub(super) struct InspectedBefore {
    pub path: PathBuf,
    /// Recorded SHA-256 before the pending entry (`None` if newly created).
    pub print: Option<String>,
    /// Observed SHA-256 from the single inspection read (`None` if absent).
    pub found: Option<String>,
    /// Verified target identity captured in that same read.
    pub target: ExpectedTarget,
}

/// Verified pre-mutation state of a target path during original-DWG undo or
/// recovery, re-checked immediately before moving aside or replacing.
#[derive(Clone, Copy, Debug)]
pub(super) enum ExpectedTarget {
    /// Target path must not exist when placed.
    Absent,
    /// Target path must exist as a regular non-symlink file distinct from the
    /// original, matching `Option<FileId>` (always `Some` on Unix/Windows).
    Existing(Option<FileId>),
}

/// One file a change set puts in place, or takes away.
pub(super) struct Replacement {
    pub path: PathBuf,
    /// The file's new bytes, or `None` to take the file away.
    pub bytes: Option<Vec<u8>>,
    /// What the file is, for an error, such as "the DWG".
    pub what: &'static str,
}

/// One file a commit writes, as it was before: what a rollback puts back.
pub(super) struct Before {
    pub path: PathBuf,
    /// The SHA-256 of its bytes before, which the undo states keep, or
    /// `None` when there was no file.
    pub print: Option<String>,
}

/// An entry of the journal written ahead of its files, with what a rollback
/// needs to put them back.
pub(super) struct Ahead<'a> {
    /// The journal's file.
    pub journal: &'a Path,
    /// The entry's number, which names the files the commit writes beside
    /// their places.
    pub entry: u64,
    /// The entry's line, marked pending.
    pub pending: &'a [u8],
    /// The line that marks it done.
    pub done: &'a [u8],
    /// Each file the commit writes, as it was before.
    pub before: &'a [Before],
    /// The folder of undo states, which keeps those bytes.
    pub states: &'a Path,
    /// The folders the commit makes for its files, which a rollback takes
    /// away once they are empty, the deepest first.
    pub folders_made: &'a [PathBuf],
}

/// One file a rollback put back: its SHA-256 as the rollback found it and
/// as it put it back, `None` where there was no file.
pub(super) struct Restored {
    pub path: PathBuf,
    pub found: Option<String>,
    pub back: Option<String>,
}

/// Why text could not be added to the end of a file: the error, and why the
/// file could not be cut back to its length before, when it could not.
struct AddFailed {
    error: io::Error,
    left: Option<io::Error>,
}

impl From<io::Error> for AddFailed {
    fn from(error: io::Error) -> Self {
        Self { error, left: None }
    }
}

/// Where one replacement got to: the file that was there, moved aside, and the
/// platform `FileId` of any new file this commit placed at `path`.
struct Placed {
    aside: Option<PathBuf>,
    placed_id: Option<FileId>,
}

/// Why files could not all be put in place: the error, and the files that
/// could not be put back as they were. Boxed where it is given, since an
/// error is large.
struct Unplaced {
    error: Error,
    not_back: Vec<String>,
}

/// One brand-new file an original-DWG revision places atomically without
/// overwriting any existing file, symlink, or original.
pub(super) struct NewFile<'a> {
    pub path: &'a Path,
    pub bytes: &'a [u8],
    pub what: &'static str,
}

/// Writes `ahead`'s entry, pending, then puts every file of `replacements`
/// in place, the undo states first, then marks the entry done; or, when a
/// step fails, puts every file back and cuts the journal back, so nothing is
/// changed. On failure the error says what is left.
pub(super) fn commit(ahead: &Ahead, replacements: &[Replacement]) -> Result<(), Error> {
    commit_impl(ahead, replacements, None, None, true)
}

/// Write-ahead commit for original-DWG `undo` and `accept` entries: enforces
/// real non-symlink journal directories, no-follow journal opens, scoped
/// `.rev.tmp`/`.agentdrafter-undo.old` filenames, and pre-mutation `FileId`
/// re-verification against `expected` and `guard.source_id`.
pub(super) fn commit_original_replacements(
    ahead: &Ahead,
    replacements: &[Replacement],
    expected: &[ExpectedTarget],
    guard: OriginalGuard<'_>,
) -> Result<(), Error> {
    debug_assert_eq!(replacements.len(), expected.len());
    ensure_real_dir_chain(guard.base_dir, &[".agentdrafter", "originals", guard.key]).map_err(
        |e| {
            Error::new(
                ErrorCode::WriteFailed,
                format!("could not verify the original's journal folder: {e}"),
            )
            .in_file(ahead.journal)
        },
    )?;
    commit_impl(ahead, replacements, Some(expected), Some(guard), false)
}

fn commit_impl(
    ahead: &Ahead,
    replacements: &[Replacement],
    expected: Option<&[ExpectedTarget]>,
    guard: Option<OriginalGuard<'_>>,
    remove_empty_parents: bool,
) -> Result<(), Error> {
    let source_id = guard.and_then(|g| g.source_id);
    let mut seam = Seam::commit();
    let length = match add_to_end(ahead.journal, ahead.pending, Sync::Pending, source_id) {
        Ok(length) => length,
        Err(failed) => return Err(not_added(ahead, failed)),
    };
    seam.passed(|| "its entry was written, pending".to_owned());
    // Each file of the entry by its place in the entry; an undo state by its
    // own.
    let keys = replacements
        .iter()
        .enumerate()
        .map(
            |(i, r)| match ahead.before.iter().position(|b| b.path == r.path) {
                Some(j) => format!("f{j}"),
                None => format!("s{i}"),
            },
        )
        .collect();
    let names = Names::new(ahead.entry, guard.map(|g| g.temp_scope), keys);
    let placed = match put_in_place(replacements, expected, guard, &names, &mut seam) {
        Ok(placed) => placed,
        Err(unplaced) => {
            if let Some(g) = guard {
                // `put_in_place` already rolled back ONLY the files THIS commit
                // actually moved/modified before failing. Never re-read or
                // overwrite any untouched/rejected file (such as one rejected
                // for a changed `FileId`).
                return Err(cut_back_original_unplaced(ahead, length, *unplaced, g));
            }
            return Err(not_finished(
                ahead,
                length,
                *unplaced,
                guard,
                remove_empty_parents,
            ));
        }
    };
    finish(replacements, &placed, guard, remove_empty_parents);
    seam.passed(|| "every file was in place".to_owned());
    let done = seam
        .fail(replacements.len())
        .map_err(AddFailed::from)
        .and_then(|()| add_to_end(ahead.journal, ahead.done, Sync::Done, source_id).map(|_| ()));
    let Err(failed) = done else {
        return Ok(());
    };
    let base = format!(
        "could not mark entry {} done in the set's journal ({})",
        ahead.entry, failed.error
    );
    match marked(ahead, length, guard) {
        // Written whole, and only its sync failed and could not be taken
        // back: every file is in place, so the entry is done.
        Marked::Done => Ok(()),
        // Taken back: the entry is pending, so the files can be put back.
        Marked::Pending => {
            if let Some(g) = guard {
                let inspected: Vec<InspectedBefore> = ahead
                    .before
                    .iter()
                    .map(|b| {
                        let (found, target) = replacements
                            .iter()
                            .zip(&placed)
                            .find(|(r, _)| r.path == b.path)
                            .map(|(r, p)| match &r.bytes {
                                Some(bytes) => (
                                    Some(fingerprint(bytes)),
                                    ExpectedTarget::Existing(p.placed_id),
                                ),
                                None => (None, ExpectedTarget::Absent),
                            })
                            .unwrap_or((b.print.clone(), ExpectedTarget::Absent));
                        InspectedBefore {
                            path: b.path.clone(),
                            print: b.print.clone(),
                            found,
                            target,
                        }
                    })
                    .collect();
                let err = Error::new(ErrorCode::WriteFailed, base).in_file(ahead.journal);
                return Err(not_finished_original_done_failed(
                    ahead, length, err, &inspected, g,
                ));
            }
            Err(not_finished(
                ahead,
                length,
                Unplaced {
                    error: Error::new(ErrorCode::WriteFailed, base).in_file(ahead.journal),
                    not_back: Vec::new(),
                },
                guard,
                remove_empty_parents,
            ))
        }
        // It may stand in part: putting the files back now could leave a
        // journal that says done beside them, so they are left in place.
        Marked::Unknown => Err(Error::new(
            ErrorCode::WriteFailed,
            format!(
                "{base}, and it could not be taken back ({}): every file is in place, and the \
                 journal may hold the line in part; the next command cuts a line written in part \
                 off and rolls entry {} back",
                failed
                    .left
                    .map_or_else(|| "it could not be read".to_owned(), |e| e.to_string()),
                ahead.entry
            ),
        )
        .in_file(ahead.journal)),
    }
}

/// Write-ahead commit for brand-new revision files beside an original DWG:
/// appends `ahead.pending`, stages each file into an unpredictable scoped temp
/// file beside its destination, places each atomically with non-clobbering
/// [`crate::original::place_new_file`] (never overwriting an existing file,
/// symlink, or the original), and appends `ahead.done`. On any failure, rolls
/// back only files whose identity matches what we placed and cuts the journal
/// back so nothing is changed.
pub(super) fn commit_new_files(
    ahead: &Ahead,
    new_files: &[NewFile<'_>],
    guard: OriginalGuard<'_>,
) -> Result<(), Error> {
    ensure_real_dir_chain(guard.base_dir, &[".agentdrafter", "originals", guard.key]).map_err(
        |e| {
            Error::new(
                ErrorCode::WriteFailed,
                format!("could not make the folder the journal goes in: {e}"),
            )
            .in_file(ahead.journal)
        },
    )?;
    let mut seam = Seam::commit();
    let length = match add_to_end(ahead.journal, ahead.pending, Sync::Pending, guard.source_id) {
        Ok(length) => length,
        Err(failed) => return Err(not_added(ahead, failed)),
    };
    seam.passed(|| "its entry was written, pending".to_owned());

    let keys: Vec<String> = (0..new_files.len()).map(|j| format!("f{j}")).collect();
    let names = Names::new(ahead.entry, Some(guard.temp_scope), keys);

    let mut staged: Vec<(PathBuf, Option<FileId>)> = Vec::with_capacity(new_files.len());
    for (j, file) in new_files.iter().enumerate() {
        let created = create_scoped_temp_beside(file.path, &names.key_prefix(j), names.tmp_suffix);
        let (tmp, mut f) = match created {
            Ok(pair) => pair,
            Err(e) => {
                for (t, _) in &staged {
                    let _ = fs::remove_file(t);
                }
                let err = Error::new(
                    ErrorCode::WriteFailed,
                    format!("could not write the {}: {e}", file.what),
                )
                .in_file(file.path);
                return Err(cut_back_new_files_error(
                    ahead,
                    length,
                    guard.source_id,
                    err,
                ));
            }
        };
        #[cfg(any(unix, windows))]
        let id = crate::build::file_id_of_file(&f);
        #[cfg(not(any(unix, windows)))]
        let id: Option<FileId> = None;
        let wrote = f.write_all(file.bytes).and_then(|()| f.sync_all());
        drop(f);
        if let Err(e) = wrote {
            let _ = fs::remove_file(&tmp);
            for (t, _) in &staged {
                let _ = fs::remove_file(t);
            }
            let err = Error::new(
                ErrorCode::WriteFailed,
                format!("could not write the {}: {e}", file.what),
            )
            .in_file(file.path);
            return Err(cut_back_new_files_error(
                ahead,
                length,
                guard.source_id,
                err,
            ));
        }
        staged.push((tmp, id));
        seam.passed(|| format!("{} was written beside its place", file.path.display()));
    }

    let mut placed: Vec<(&Path, Option<FileId>)> = Vec::with_capacity(new_files.len());
    for (j, (file, (tmp, id))) in new_files.iter().zip(&staged).enumerate() {
        if let Err(e) = seam.fail(j) {
            let err = Error::new(
                ErrorCode::WriteFailed,
                format!("could not put {} in place ({e})", file.what),
            )
            .in_file(file.path);
            let unplaced: Vec<&Path> = staged[j..].iter().map(|(p, _)| p.as_path()).collect();
            let notes = rollback_placed_new_files(&placed, &unplaced, guard.source_id);
            return Err(cut_back_with_notes(
                ahead,
                length,
                guard.source_id,
                err,
                notes,
            ));
        }
        if let Err(err) = place_new_file(tmp, file.path, file.what, ErrorCode::OutputExists) {
            let unplaced: Vec<&Path> = staged[j + 1..].iter().map(|(p, _)| p.as_path()).collect();
            let notes = rollback_placed_new_files(&placed, &unplaced, guard.source_id);
            return Err(cut_back_with_notes(
                ahead,
                length,
                guard.source_id,
                err,
                notes,
            ));
        }
        placed.push((file.path, *id));
        seam.passed(|| format!("{} was put in its place", file.path.display()));
    }

    seam.passed(|| "every file was in place".to_owned());
    let done = seam
        .fail(new_files.len())
        .map_err(AddFailed::from)
        .and_then(|()| {
            add_to_end(ahead.journal, ahead.done, Sync::Done, guard.source_id).map(|_| ())
        });
    let Err(failed) = done else {
        return Ok(());
    };
    let base = format!(
        "could not mark entry {} done in the journal ({})",
        ahead.entry, failed.error
    );
    match marked(ahead, length, Some(guard)) {
        Marked::Done => Ok(()),
        Marked::Pending => {
            let notes = rollback_placed_new_files(&placed, &[], guard.source_id);
            let err = Error::new(ErrorCode::WriteFailed, base).in_file(ahead.journal);
            Err(cut_back_with_notes(
                ahead,
                length,
                guard.source_id,
                err,
                notes,
            ))
        }
        Marked::Unknown => Err(Error::new(
            ErrorCode::WriteFailed,
            format!(
                "{base}, and it could not be taken back ({}): every file is in place, and the \
                 journal may hold the line in part; the next command cuts a line written in part \
                 off and rolls entry {} back",
                failed
                    .left
                    .map_or_else(|| "it could not be read".to_owned(), |e| e.to_string()),
                ahead.entry
            ),
        )
        .in_file(ahead.journal)),
    }
}

pub(crate) fn rollback_placed_new_files(
    placed: &[(&Path, Option<crate::build::FileId>)],
    unplaced: &[&Path],
    source_id: Option<crate::build::FileId>,
) -> Vec<String> {
    let mut notes = Vec::new();
    for &tmp in unplaced {
        if let Err(e) = fs::remove_file(tmp) {
            if e.kind() != io::ErrorKind::NotFound {
                notes.push(format!(
                    "could not remove temporary file `{}`: {e}",
                    tmp.display()
                ));
            }
        }
    }
    for &(dest, expected_id) in placed {
        #[cfg(any(unix, windows))]
        let same = {
            let current = crate::original::regular_file_id_no_follow(dest);
            expected_id.is_some()
                && current == expected_id
                && !(source_id.is_some() && current == source_id)
        };
        #[cfg(not(any(unix, windows)))]
        let same = {
            let _ = (expected_id, source_id);
            fs::symlink_metadata(dest).is_ok_and(|m| m.file_type().is_file())
        };
        if !same {
            notes.push(format!(
                "left `{}` in place during rollback because its file identity changed",
                dest.display()
            ));
            continue;
        }
        if let Err(e) = fs::remove_file(dest) {
            if e.kind() != io::ErrorKind::NotFound {
                notes.push(format!(
                    "could not remove `{}` during rollback: {e}",
                    dest.display()
                ));
            }
        }
    }
    notes
}

fn cut_back_new_files_error(
    ahead: &Ahead,
    length: u64,
    source_id: Option<FileId>,
    mut err: Error,
) -> Error {
    match cut_back_checked(ahead.journal, length, source_id) {
        Ok(()) => {
            err.message.push_str("; nothing was changed");
        }
        Err(e) => {
            err.message.push_str(&format!(
                "; the journal could not be cut back ({e}), so it holds entry {} pending, and \
                 the next command marks it rolled back",
                ahead.entry
            ));
        }
    }
    err
}

fn cut_back_with_notes(
    ahead: &Ahead,
    length: u64,
    source_id: Option<FileId>,
    mut err: Error,
    notes: Vec<String>,
) -> Error {
    if !notes.is_empty() {
        err.message
            .push_str(&format!("; rollback notes: {}", notes.join("; ")));
        err.message.push_str(&format!(
            "; the journal holds entry {} pending so the next command can finish rolling it back",
            ahead.entry
        ));
        return err;
    }
    match cut_back_checked(ahead.journal, length, source_id) {
        Ok(()) => {
            err.message
                .push_str("; every file was put back as it was, so nothing was changed");
        }
        Err(e) => {
            err.message.push_str(&format!(
                "; every file was put back as it was, but the journal could not be cut back \
                 ({e}), so it holds entry {} pending, and the next command marks it rolled back",
                ahead.entry
            ));
        }
    }
    err
}

/// What the journal holds of `ahead`'s entry, written from `length`, once
/// the line that marks it done could not be written.
enum Marked {
    /// The entry and its done line, whole, and nothing after.
    Done,
    /// The entry, and nothing after it.
    Pending,
    /// Something else, such as part of the done line, or a journal that
    /// cannot be read.
    Unknown,
}

fn marked(ahead: &Ahead, length: u64, guard: Option<OriginalGuard<'_>>) -> Marked {
    let bytes = match guard {
        Some(g) => match read_regular_no_follow(g.source, g.source_id, ahead.journal) {
            Ok(NoFollowRead::File(bytes)) => bytes,
            _ => return Marked::Unknown,
        },
        None => match fs::read(ahead.journal) {
            Ok(bytes) => bytes,
            Err(_) => return Marked::Unknown,
        },
    };
    let Some(after) = usize::try_from(length).ok().and_then(|at| bytes.get(at..)) else {
        return Marked::Unknown;
    };
    if after == ahead.pending {
        Marked::Pending
    } else if after.strip_prefix(ahead.pending) == Some(ahead.done) {
        Marked::Done
    } else {
        Marked::Unknown
    }
}

/// The error for an entry that could not be added to the journal: nothing
/// was changed, unless the journal could not be cut back either.
fn not_added(ahead: &Ahead, failed: AddFailed) -> Error {
    let e = failed.error;
    let message = match failed.left {
        None => format!("could not add to the set's journal ({e}), so nothing was changed"),
        // A journal that could not be cut back may still hold the entry:
        // say so, and where.
        Some(left) => format!(
            "could not add to the set's journal ({e}): no file was put in place, but the journal \
             could not be cut back ({left}), so it may still hold entry {}, or part of it, \
             pending; the next command rolls a whole entry back, and part of one must be taken \
             out of {}",
            ahead.entry,
            ahead.journal.display()
        ),
    };
    Error::new(ErrorCode::WriteFailed, message).in_file(ahead.journal)
}

/// Finalizes an original-DWG commit failure after `put_in_place` already
/// rolled back only the files THIS commit touched: cuts back the journal if
/// every touched file was restored, otherwise leaves the entry pending. Never
/// re-reads or overwrites an untouched/rejected file.
fn cut_back_original_unplaced(
    ahead: &Ahead,
    length: u64,
    unplaced: Unplaced,
    guard: OriginalGuard<'_>,
) -> Error {
    let Unplaced {
        mut error,
        not_back,
    } = unplaced;
    if !not_back.is_empty() {
        error.message.push_str(&format!(
            ", and could not put back {} at once: the journal holds entry {} pending so the next \
             command can finish rolling it back",
            not_back.join(", "),
            ahead.entry
        ));
        return error;
    }
    match cut_back_checked(ahead.journal, length, guard.source_id) {
        Ok(()) => {
            error
                .message
                .push_str("; every file this operation touched was put back as it was, so nothing was changed");
        }
        Err(e) => {
            error.message.push_str(&format!(
                "; every file this operation touched was put back as it was, but the journal \
                 could not be cut back ({e}), so it holds entry {} pending, and the next command \
                 marks it rolled back",
                ahead.entry
            ));
        }
    }
    error
}

/// Restores only files whose post-placement identity still matches what this
/// commit just left when marking `done` failed with `Marked::Pending`.
fn not_finished_original_done_failed(
    ahead: &Ahead,
    length: u64,
    error: Error,
    inspected: &[InspectedBefore],
    guard: OriginalGuard<'_>,
) -> Error {
    let base = error.message.clone();
    let message = match restore_original_inspected(
        ahead.entry,
        inspected,
        ahead.states,
        ahead.folders_made,
        guard,
    ) {
        Err(failed) => format!(
            "{base}, and could not put every file back as it was ({}): the journal holds entry {} \
             pending, so the next command puts every file back from its undo state",
            failed.message, ahead.entry
        ),
        Ok(_) => match cut_back_checked(ahead.journal, length, guard.source_id) {
            Ok(()) => format!("{base}; every file was put back as it was, so nothing was changed"),
            Err(e) => format!(
                "{base}; every file was put back as it was, but the journal could not be cut back \
                 ({e}), so it holds entry {} pending, and the next command marks it rolled back",
                ahead.entry
            ),
        },
    };
    Error { message, ..error }
}

/// Puts every file back as it was before the commit, through the same
/// [`restore`] a rollback uses, and cuts the journal back to `length`, its
/// length before the entry, so that nothing was changed. When either cannot
/// be done, the entry stays pending and the error says so: the next command
/// rolls it back.
fn not_finished(
    ahead: &Ahead,
    length: u64,
    unplaced: Unplaced,
    guard: Option<OriginalGuard<'_>>,
    remove_empty_parents: bool,
) -> Error {
    let Unplaced { error, not_back } = unplaced;
    let base = error.message.clone();
    let source_id = guard.and_then(|g| g.source_id);
    let message = match restore_impl(
        ahead.entry,
        ahead.before,
        ahead.states,
        ahead.folders_made,
        guard,
        remove_empty_parents,
    ) {
        Err(failed) => format!(
            "{base}, and could not put every file back as it was ({}{}): the set's journal \
             holds entry {} pending, so the next command puts every file back from its undo \
             state",
            failed.message,
            if not_back.is_empty() {
                String::new()
            } else {
                format!("; {} not put back at once", not_back.join(", "))
            },
            ahead.entry
        ),
        Ok(_) => match cut_back_checked(ahead.journal, length, source_id) {
            Ok(()) => format!("{base}; every file was put back as it was, so nothing was changed"),
            Err(e) => format!(
                "{base}; every file was put back as it was, but the set's journal could not be \
                 cut back ({e}), so it holds entry {} pending, and the next command marks it \
                 rolled back",
                ahead.entry
            ),
        },
    };
    Error { message, ..error }
}

/// Puts each file of `before` back as it was, byte for byte, from the undo
/// states in `states`: a file that was not there is taken away, and each
/// folder of `folders_made` once it is empty. A file already as it was is
/// left. What the commit of `entry` left beside the files, new files not
/// yet in place and old ones moved aside, is taken away once all are back.
///
/// It is all or nothing: every undo state it needs is read and checked
/// first, and if a file cannot be put back, every file is as the restore
/// found it, and the error says why. A commit that fails puts its files
/// back through it, and so does the rollback of one a killed process left.
pub(super) fn restore(
    entry: u64,
    before: &[Before],
    states: &Path,
    folders_made: &[PathBuf],
) -> Result<Vec<Restored>, Error> {
    restore_impl(entry, before, states, folders_made, None, true)
}

/// Restores original-DWG files from a single-read [`InspectedBefore`] list
/// without re-reading the files on disk, verifying real directory chains,
/// pre-mutation `FileId`, and `temp_scope`-isolated cleanup.
pub(super) fn restore_original_inspected(
    entry: u64,
    inspected: &[InspectedBefore],
    states: &Path,
    folders_made: &[PathBuf],
    guard: OriginalGuard<'_>,
) -> Result<Vec<Restored>, Error> {
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut expected: Vec<ExpectedTarget> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut restored: Vec<Restored> = Vec::new();
    for (j, file) in inspected.iter().enumerate() {
        if file.found == file.print || replacements.iter().any(|r| r.path == file.path) {
            continue;
        }
        let bytes = match &file.print {
            None => None,
            Some(print) => {
                let state = states.join(print);
                let bytes = super::journal::undo_state_original_at(
                    guard.base_dir,
                    guard.key,
                    guard.source,
                    guard.source_id,
                    print,
                );
                Some(bytes.ok_or_else(|| {
                    Error::new(
                        ErrorCode::ReadFailed,
                        format!(
                            "the undo state of {}, which holds it as it was before, is missing or \
                             damaged",
                            file.path.display()
                        ),
                    )
                    .in_file(&state)
                })?)
            }
        };
        restored.push(Restored {
            path: file.path.clone(),
            found: file.found.clone(),
            back: file.print.clone(),
        });
        replacements.push(Replacement {
            path: file.path.clone(),
            bytes,
            what: "a file the rollback puts back",
        });
        expected.push(file.target);
        keys.push(format!("back-f{j}"));
    }
    let names = Names::new(entry, Some(guard.temp_scope), keys);
    let placed = put_in_place(
        &replacements,
        Some(expected.as_slice()),
        Some(guard),
        &names,
        &mut Seam::restore(),
    )
    .map_err(|unplaced| {
        let Unplaced { error, not_back } = *unplaced;
        if not_back.is_empty() {
            error
        } else {
            Error {
                message: format!(
                    "{}, and could not put back {} as the rollback found it",
                    error.message,
                    not_back.join(", ")
                ),
                ..error
            }
        }
    })?;
    finish(&replacements, &placed, Some(guard), false);
    let paths: Vec<&Path> = inspected.iter().map(|f| f.path.as_path()).collect();
    cleanup_after_restore(&paths, states, folders_made, &names, Some(guard))?;
    Ok(restored)
}

fn restore_impl(
    entry: u64,
    before: &[Before],
    states: &Path,
    folders_made: &[PathBuf],
    guard: Option<OriginalGuard<'_>>,
    remove_empty_parents: bool,
) -> Result<Vec<Restored>, Error> {
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut restored: Vec<Restored> = Vec::new();
    for (j, file) in before.iter().enumerate() {
        let found = match fs::read(&file.path) {
            Ok(bytes) => Some(fingerprint(&bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(Error::new(
                    ErrorCode::ReadFailed,
                    format!("could not read the file to put it back: {e}"),
                )
                .in_file(&file.path))
            }
        };
        if found == file.print || replacements.iter().any(|r| r.path == file.path) {
            continue;
        }
        let bytes = match &file.print {
            None => None,
            Some(print) => {
                let state = states.join(print);
                let bytes = fs::read(&state)
                    .ok()
                    .filter(|bytes| fingerprint(bytes) == *print);
                Some(bytes.ok_or_else(|| {
                    Error::new(
                        ErrorCode::ReadFailed,
                        format!(
                            "the undo state of {}, which holds it as it was before, is missing or \
                             damaged",
                            file.path.display()
                        ),
                    )
                    .in_file(&state)
                })?)
            }
        };
        restored.push(Restored {
            path: file.path.clone(),
            found,
            back: file.print.clone(),
        });
        replacements.push(Replacement {
            path: file.path.clone(),
            bytes,
            what: "a file the rollback puts back",
        });
        keys.push(format!("back-f{j}"));
    }
    let names = Names::new(entry, guard.map(|g| g.temp_scope), keys);
    let placed = put_in_place(&replacements, None, guard, &names, &mut Seam::restore()).map_err(
        |unplaced| {
            let Unplaced { error, not_back } = *unplaced;
            if not_back.is_empty() {
                error
            } else {
                Error {
                    message: format!(
                        "{}, and could not put back {} as the rollback found it",
                        error.message,
                        not_back.join(", ")
                    ),
                    ..error
                }
            }
        },
    )?;
    finish(&replacements, &placed, guard, remove_empty_parents);
    let paths: Vec<&Path> = before.iter().map(|f| f.path.as_path()).collect();
    cleanup_after_restore(&paths, states, folders_made, &names, guard)?;
    Ok(restored)
}

fn cleanup_after_restore(
    before_paths: &[&Path],
    states: &Path,
    folders_made: &[PathBuf],
    names: &Names,
    guard: Option<OriginalGuard<'_>>,
) -> Result<(), Error> {
    let mut folders = folders_made.to_vec();
    folders.sort_by_key(|f| Reverse(f.components().count()));
    let mut beside: Vec<&Path> = before_paths.iter().filter_map(|f| f.parent()).collect();
    if let Some(g) = guard {
        // Verify `.agentdrafter/originals/<key>/undo-states` is a real
        // non-symlink directory chain before scanning it for leftover temps.
        let states_ok = verify_real_dir_chain(
            g.base_dir,
            &[".agentdrafter", "originals", g.key, "undo-states"],
        )
        .map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("cannot inspect the original's undo-states folder: {e}"),
            )
            .in_file(states)
        })?;
        if states_ok {
            beside.push(states);
        }
    } else {
        beside.push(states);
    }
    beside.extend(folders.iter().map(PathBuf::as_path));
    let tmp_end = format!(".{}", names.tmp_suffix);
    let old_end = format!(".{}", names.old_suffix);
    for folder in beside {
        let folder_path = if folder.as_os_str().is_empty() {
            Path::new(".")
        } else {
            folder
        };
        if let Some(g) = guard {
            let Ok(meta) = fs::symlink_metadata(folder_path) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
                continue;
            }
            if let Ok(rel) = folder_path.strip_prefix(g.orig_dir_real) {
                if !rel.as_os_str().is_empty()
                    && verify_no_symlink_intermediate_dirs(g.orig_dir_real, &rel.join("x")).is_err()
                {
                    continue;
                }
            }
        }
        let Ok(listed) = fs::read_dir(folder_path) else {
            continue;
        };
        for item in listed.flatten() {
            let name = item.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            if !name_str.starts_with(&names.prefix)
                || !(name_str.ends_with(&tmp_end) || name_str.ends_with(&old_end))
            {
                continue;
            }
            let item_path = item.path();
            if let Some(g) = guard {
                if !is_safe_regular_non_original(&item_path, g.source, g.source_id) {
                    continue;
                }
            }
            let _ = fs::remove_file(&item_path);
        }
    }
    for folder in folders {
        let _ = fs::remove_dir(folder);
    }
    Ok(())
}

/// Whether `path` is an existing regular non-symlink file distinct from the
/// original DWG (ADR-0004).
fn is_safe_regular_non_original(path: &Path, source: &Path, source_id: Option<FileId>) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return false;
    }
    #[cfg(any(unix, windows))]
    {
        let _ = source;
        let cur = crate::original::regular_file_id_no_follow(path);
        cur.is_some() && !(source_id.is_some() && cur == source_id)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = source_id;
        !crate::build::is_same_file(source, path)
    }
}

/// Whether the commit of `entry`, or a rollback of it, moved the file at
/// `path`, its file `j`, aside, holding bytes whose SHA-256 is one of
/// `prints`, and stopped before it put another in its place: so a missing
/// file is one a commit left so, not one taken away by hand. Only that
/// file's own, named after `j`, counts, never another file's.
pub(super) fn moved_aside(entry: u64, path: &Path, j: usize, prints: &[&str]) -> bool {
    [format!("f{j}"), format!("back-f{j}")].iter().any(|key| {
        let aside = path.with_file_name(format!("{}{key}.old", prefix(entry, None)));
        fs::read(aside).is_ok_and(|bytes| prints.contains(&fingerprint(&bytes).as_str()))
    })
}

/// Scoped, non-following [`moved_aside`] for original-DWG crash recovery,
/// matching randomized backup filenames produced by
/// [`move_to_unique_backup_beside`].
pub(super) fn moved_aside_original(
    entry: u64,
    guard: OriginalGuard<'_>,
    path: &Path,
    j: usize,
    prints: &[&str],
) -> bool {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let Ok(meta) = fs::symlink_metadata(parent) else {
        return false;
    };
    if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(parent) else {
        return false;
    };
    let base = prefix(entry, Some(guard.temp_scope));
    let key_prefixes = [format!("{base}f{j}-"), format!("{base}back-f{j}-")];
    let exact_legacy = [
        format!("{base}f{j}.agentdrafter-undo.old"),
        format!("{base}back-f{j}.agentdrafter-undo.old"),
    ];
    for item in entries.flatten() {
        let name = item.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !name_str.ends_with(".agentdrafter-undo.old") {
            continue;
        }
        let matches_key = key_prefixes.iter().any(|p| name_str.starts_with(p))
            || exact_legacy.iter().any(|e| name_str == e);
        if !matches_key {
            continue;
        }
        if matches!(
            read_regular_no_follow(guard.source, guard.source_id, &item.path()),
            Ok(NoFollowRead::File(bytes)) if prints.contains(&fingerprint(&bytes).as_str())
        ) {
            return true;
        }
    }
    false
}

/// Adds `line` to the end of the journal at `path`, making it if it is not
/// there, and syncs it, in a step of its own, such as a failed attempt's
/// entry or the line that marks an entry rolled back. When it cannot all be
/// written, the journal is cut back to its length before.
pub(super) fn add_line(path: &Path, line: &[u8], what: &str) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            Error::new(
                ErrorCode::WriteFailed,
                format!("could not make the folder the set's journal goes in: {e}"),
            )
            .in_file(parent)
        })?;
    }
    add_line_checked(path, line, what, None)
}

/// Like [`add_line`], for original-DWG journals: verifies the real directory
/// chain (never traversing symlinks), opens `journal.jsonl` without following
/// symlinks, and refuses any file matching `guard.source_id`.
pub(super) fn add_line_original(
    path: &Path,
    line: &[u8],
    what: &str,
    guard: OriginalGuard<'_>,
) -> Result<(), Error> {
    ensure_real_dir_chain(guard.base_dir, &[".agentdrafter", "originals", guard.key]).map_err(
        |e| {
            Error::new(
                ErrorCode::WriteFailed,
                format!("could not make the folder the original's journal goes in: {e}"),
            )
            .in_file(path)
        },
    )?;
    add_line_checked(path, line, what, guard.source_id)
}

fn add_line_checked(
    path: &Path,
    line: &[u8],
    what: &str,
    source_id: Option<FileId>,
) -> Result<(), Error> {
    add_to_end(path, line, Sync::Plain, source_id)
        .map(|_| ())
        .map_err(|failed| {
            let left = failed
                .left
                .map(|left| {
                    format!(
                    ", and it could not be cut back ({left}), so it may hold part of {what}; take \
                     it out"
                )
                })
                .unwrap_or_default();
            Error::new(
                ErrorCode::WriteFailed,
                format!(
                    "could not add {what} to the set's journal ({}){left}",
                    failed.error
                ),
            )
            .in_file(path)
        })
}

/// What every file a commit of `entry` writes beside a file begins with.
/// When `scope` is `Some(key)` (an original DWG's `<safe-stem>` key), temp and
/// backup files are isolated per original so two originals in the same folder
/// never collide on entry numbers.
fn prefix(entry: u64, scope: Option<&str>) -> String {
    match scope {
        Some(key) => format!(".agentdrafter-orig-{key}-entry-{entry}-"),
        None => format!(".agentdrafter-entry-{entry}-"),
    }
}

/// The names a commit's files take beside their places: by its entry (and
/// original `<safe-stem>` scope when revising an original), and by each
/// replacement's key, which names the file it stands for.
struct Names {
    prefix: String,
    tmp_suffix: &'static str,
    old_suffix: &'static str,
    keys: Vec<String>,
}

impl Names {
    fn new(entry: u64, scope: Option<&str>, keys: Vec<String>) -> Self {
        let (tmp_suffix, old_suffix) = if scope.is_some() {
            ("rev.tmp", "agentdrafter-undo.old")
        } else {
            ("tmp", "old")
        };
        Self {
            prefix: prefix(entry, scope),
            tmp_suffix,
            old_suffix,
            keys,
        }
    }

    /// The scoped prefix for replacement `n` before its random suffix.
    fn key_prefix(&self, n: usize) -> String {
        format!("{}{}", self.prefix, self.keys[n])
    }

    /// The moved-aside backup name beside `path` for replacement `n`.
    fn aside(&self, path: &Path, n: usize) -> PathBuf {
        path.with_file_name(format!(
            "{}{}.{}",
            self.prefix, self.keys[n], self.old_suffix
        ))
    }
}

/// Puts every file of `replacements` in place, or none: each new file is
/// written whole beside its place, then each is moved into it, the file
/// there moved aside. When a step fails, every file already moved is put
/// back and every new file taken away, with the folders made for them.
fn put_in_place(
    replacements: &[Replacement],
    expected: Option<&[ExpectedTarget]>,
    guard: Option<OriginalGuard<'_>>,
    names: &Names,
    seam: &mut Seam,
) -> Result<Vec<Placed>, Box<Unplaced>> {
    let mut made: Vec<PathBuf> = Vec::new();
    let mut staged: Vec<Option<(PathBuf, Option<FileId>)>> = Vec::new();
    if let Err(error) = stage(replacements, names, &mut made, &mut staged, seam) {
        clean(&staged, &made);
        return Err(Box::new(Unplaced {
            error,
            not_back: Vec::new(),
        }));
    }
    let mut placed: Vec<Placed> = Vec::with_capacity(replacements.len());
    for (i, (replacement, temporary)) in replacements.iter().zip(&staged).enumerate() {
        let target = expected.map(|list| list[i]);
        let temp_ref = temporary.as_ref().map(|(p, id)| (p.as_path(), *id));
        match place(i, replacement, target, guard, temp_ref, names, seam) {
            Ok(done) => placed.push(done),
            Err(e) => {
                let not_back = roll_back(&replacements[..placed.len()], &placed, guard);
                clean(&staged[i..], &made);
                let error = Error::new(
                    ErrorCode::WriteFailed,
                    format!("could not put {} in place ({e})", replacement.what),
                )
                .in_file(&replacement.path);
                return Err(Box::new(Unplaced { error, not_back }));
            }
        }
    }
    Ok(placed)
}

/// Takes away the files moved aside, now that every file is in place, and
/// the folder a file taken away leaves empty.
fn finish(
    replacements: &[Replacement],
    placed: &[Placed],
    guard: Option<OriginalGuard<'_>>,
    remove_empty_parents: bool,
) {
    for (replacement, done) in replacements.iter().zip(placed) {
        if let Some(aside) = &done.aside {
            let safe =
                guard.is_none_or(|g| is_safe_regular_non_original(aside, g.source, g.source_id));
            if safe {
                let _ = fs::remove_file(aside);
            }
        }
        if remove_empty_parents && replacement.bytes.is_none() {
            if let Some(parent) = replacement.path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    }
}

/// Makes the folders the new files go in, and writes each beside its place
/// into a brand-new unpredictable temporary file (`create_new(true)`).
fn stage(
    replacements: &[Replacement],
    names: &Names,
    made: &mut Vec<PathBuf>,
    staged: &mut Vec<Option<(PathBuf, Option<FileId>)>>,
    seam: &mut Seam,
) -> Result<(), Error> {
    for (i, replacement) in replacements.iter().enumerate() {
        let path = &replacement.path;
        let Some(bytes) = &replacement.bytes else {
            staged.push(None);
            continue;
        };
        if path.file_name().is_none() {
            return Err(Error::usage("the path does not name a file").in_file(path));
        }
        make_folder(path, replacement.what, made)?;
        let (temporary, mut file) =
            create_scoped_temp_beside(path, &names.key_prefix(i), names.tmp_suffix).map_err(
                |e| {
                    Error::new(
                        ErrorCode::WriteFailed,
                        format!("could not write {}: {e}", replacement.what),
                    )
                    .in_file(path)
                },
            )?;
        #[cfg(any(unix, windows))]
        let tmp_id = crate::build::file_id_of_file(&file);
        #[cfg(not(any(unix, windows)))]
        let tmp_id: Option<FileId> = None;
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(e) = written {
            let _ = fs::remove_file(&temporary);
            return Err(Error::new(
                ErrorCode::WriteFailed,
                format!("could not write {}: {e}", replacement.what),
            )
            .in_file(path));
        }
        staged.push(Some((temporary, tmp_id)));
        seam.passed(|| format!("{} was written beside its place", path.display()));
    }
    Ok(())
}

/// Makes the folder the file at `path` goes in, and every folder above it
/// that is missing, adding each it made to `made`.
fn make_folder(path: &Path, what: &str, made: &mut Vec<PathBuf>) -> Result<(), Error> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    let missing: Vec<&Path> = parent.ancestors().take_while(|a| !a.exists()).collect();
    fs::create_dir_all(parent).map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not make the folder {what} goes in: {e}"),
        )
        .in_file(parent)
    })?;
    made.extend(missing.into_iter().map(Path::to_owned));
    Ok(())
}

/// Which line of a commit an append writes, so a test can fail its sync.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sync {
    /// The entry, pending.
    Pending,
    /// The line that marks it done.
    Done,
    /// A line of its own, such as a failed attempt's.
    Plain,
}

/// Adds `bytes` to the end of the file at `path`, making the file if it is
/// not there, and syncs it, and gives the file's length before. Opens `path`
/// without following symlinks and refusing `source_id`. When they cannot all
/// be written, the file is cut back to its length before, or taken away if it
/// was not there; when that fails too, the failure says why.
fn add_to_end(
    path: &Path,
    bytes: &[u8],
    seam: Sync,
    source_id: Option<FileId>,
) -> Result<u64, AddFailed> {
    let existed = fs::symlink_metadata(path).is_ok();
    let mut file = open_no_follow_write(path, true, source_id)?;
    let length = file.seek(SeekFrom::End(0))?;
    let written = file
        .write_all(bytes)
        .and_then(|()| fail_sync(seam))
        .and_then(|()| file.sync_all());
    if let Err(error) = written {
        let back = fail_cut_back(seam)
            .and_then(|()| file.set_len(length))
            .and_then(|()| file.sync_all());
        drop(file);
        let back = back.and_then(|()| {
            if existed {
                Ok(())
            } else {
                fs::remove_file(path)
            }
        });
        return Err(AddFailed {
            error,
            left: back.err(),
        });
    }
    Ok(length)
}

/// Cuts the journal at `path` back to `length`, its length before an entry
/// that is taken out again, or before a line written in part.
pub(super) fn cut_back(path: &Path, length: u64) -> io::Result<()> {
    cut_back_checked(path, length, None)
}

/// Like [`cut_back`], refusing symlinks and any file matching `source_id`.
pub(super) fn cut_back_checked(
    path: &Path,
    length: u64,
    source_id: Option<FileId>,
) -> io::Result<()> {
    let file = open_no_follow_write(path, false, source_id)?;
    file.set_len(length)?;
    file.sync_all()
}

/// Moves the file at the replacement's path, if there is one, aside, and
/// the new one from `temporary`, if there is one, into its place. When the
/// new one cannot be moved, the old one is put back before the error is
/// given.
///
/// When `guard` is `Some`, re-verifies that `path` still matches `expected`
/// (regular non-symlink file, distinct from the original, same `FileId` or
/// still absent) immediately before mutating anything.
fn place(
    index: usize,
    replacement: &Replacement,
    expected: Option<ExpectedTarget>,
    guard: Option<OriginalGuard<'_>>,
    temporary: Option<(&Path, Option<FileId>)>,
    names: &Names,
    seam: &mut Seam,
) -> io::Result<Placed> {
    seam.fail(index)?;
    let path = &replacement.path;
    if let (Some(g), Some(target)) = (guard, expected) {
        if let Ok(rel) = path.strip_prefix(g.orig_dir_real) {
            verify_no_symlink_intermediate_dirs(g.orig_dir_real, rel)?;
        } else if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let pmeta = fs::symlink_metadata(parent)?;
            if pmeta.file_type().is_symlink() || !pmeta.file_type().is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "`{}` parent `{}` is a symbolic link or not a directory",
                        path.display(),
                        parent.display()
                    ),
                ));
            }
        }
        let aside = match target {
            ExpectedTarget::Absent => match fs::symlink_metadata(path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "`{}` appeared while preparing the operation; refusing to overwrite",
                            path.display()
                        ),
                    ));
                }
                Err(e) => return Err(e),
            },
            ExpectedTarget::Existing(expected_id) => {
                let meta = fs::symlink_metadata(path)?;
                if meta.file_type().is_symlink() || !meta.file_type().is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "`{}` changed to a symbolic link or non-regular file before replacing",
                            path.display()
                        ),
                    ));
                }
                #[cfg(any(unix, windows))]
                {
                    let cur = crate::original::regular_file_id_no_follow(path);
                    if expected_id.is_none() || cur != expected_id {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!(
                                "`{}` changed identity before it could be replaced",
                                path.display()
                            ),
                        ));
                    }
                    if g.source_id.is_some() && cur == g.source_id {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!(
                                "`{}` is the original DWG itself, which is never modified or \
                                 removed (ADR-0004)",
                                path.display()
                            ),
                        ));
                    }
                }
                #[cfg(not(any(unix, windows)))]
                {
                    let _ = expected_id;
                    if crate::build::is_same_file(g.source, path) {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!(
                                "`{}` is the original DWG itself, which is never modified or \
                                 removed (ADR-0004)",
                                path.display()
                            ),
                        ));
                    }
                }
                // Move aside atomically to an unpredictable randomized backup
                // name without ever clobbering an existing file or symlink.
                let aside =
                    move_to_unique_backup_beside(path, &names.key_prefix(index), names.old_suffix)?;
                seam.passed(|| format!("{} was moved aside", path.display()));
                Some(aside)
            }
        };
        let placed_id = if let Some((temporary, tmp_id)) = temporary {
            if let Err(err) =
                place_new_file(temporary, path, replacement.what, ErrorCode::WriteFailed)
            {
                if let Some(aside) = &aside {
                    let _ = rename_no_replace_keep_src(aside, path);
                }
                return Err(io::Error::other(err.message));
            }
            seam.passed(|| format!("{} was put in its place", path.display()));
            tmp_id
        } else {
            None
        };
        return Ok(Placed { aside, placed_id });
    }

    let aside = if fs::symlink_metadata(path).is_ok() {
        let aside = names.aside(path, index);
        fs::rename(path, &aside)?;
        seam.passed(|| format!("{} was moved aside", path.display()));
        Some(aside)
    } else {
        None
    };
    let placed_id = if let Some((temporary, tmp_id)) = temporary {
        if let Err(e) = fs::rename(temporary, path) {
            if let Some(aside) = &aside {
                let _ = fs::rename(aside, path);
            }
            return Err(e);
        }
        seam.passed(|| format!("{} was put in its place", path.display()));
        tmp_id
    } else {
        None
    };
    Ok(Placed { aside, placed_id })
}

/// Puts back, last first, every file `placed` moved, and gives the files it
/// could not put back.
fn roll_back(
    replacements: &[Replacement],
    placed: &[Placed],
    guard: Option<OriginalGuard<'_>>,
) -> Vec<String> {
    let mut not_back = Vec::new();
    for (replacement, done) in replacements.iter().zip(placed).rev() {
        let path = &replacement.path;
        let back = match (guard, &done.aside, &replacement.bytes) {
            (Some(_), Some(aside), None) => {
                // `path` was moved aside and should be absent: restore without
                // ever clobbering an unrelated file that appeared meanwhile.
                rename_no_replace_keep_src(aside, path)
            }
            (Some(g), Some(aside), Some(_)) => {
                // `path` holds the file THIS commit placed (`done.placed_id`):
                // verify identity before removing it, then move `aside` back
                // non-clobberingly.
                if matches_placed_non_original(path, done.placed_id, g) {
                    fs::remove_file(path).and_then(|()| rename_no_replace_keep_src(aside, path))
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "refusing to overwrite file whose identity changed during rollback",
                    ))
                }
            }
            (Some(g), None, Some(_)) => {
                if matches_placed_non_original(path, done.placed_id, g) {
                    fs::remove_file(path)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "refusing to remove file whose identity changed during rollback",
                    ))
                }
            }
            (Some(_), None, None) => Ok(()),
            (None, Some(aside), _) => fs::rename(aside, path),
            (None, None, Some(_)) => fs::remove_file(path),
            (None, None, None) => Ok(()),
        };
        if back.is_err() {
            not_back.push(path.display().to_string());
        }
    }
    not_back
}

fn matches_placed_non_original(
    path: &Path,
    placed_id: Option<FileId>,
    guard: OriginalGuard<'_>,
) -> bool {
    if !is_safe_regular_non_original(path, guard.source, guard.source_id) {
        return false;
    }
    #[cfg(any(unix, windows))]
    {
        placed_id.is_some() && crate::original::regular_file_id_no_follow(path) == placed_id
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = placed_id;
        true
    }
}

/// Takes away the new files not moved into place and the folders made for
/// them, deepest first, each only when it is empty.
fn clean(staged: &[Option<(PathBuf, Option<FileId>)>], made: &[PathBuf]) {
    for (temporary, _) in staged.iter().flatten() {
        let _ = fs::remove_file(temporary);
    }
    let mut folders = made.to_vec();
    folders.sort_by_key(|f| Reverse(f.components().count()));
    for folder in folders {
        let _ = fs::remove_dir(folder);
    }
}

/// Where a test can fail a commit, or stop it as a killed process stops:
/// each step a commit passes, after its entry is written pending and
/// before it is marked done, is counted from 0, and so is each step of a
/// restore, apart.
struct Seam {
    next: usize,
    /// Whether these are a restore's steps, not a commit's.
    restoring: bool,
}

impl Seam {
    fn commit() -> Self {
        Self {
            next: 0,
            restoring: false,
        }
    }

    fn restore() -> Self {
        Self {
            next: 0,
            restoring: true,
        }
    }

    /// Fails replacement `index` of a commit, or the line that marks the
    /// entry done after the last, once, when a test asks for it, so a test
    /// can show a failed commit puts every file back.
    fn fail(&self, index: usize) -> io::Result<()> {
        #[cfg(test)]
        if !self.restoring && FAIL_AT.with(|f| f.get() == Some(index) && f.replace(None).is_some())
        {
            return Err(io::Error::other("a failure the test asked for"));
        }
        let _ = index;
        Ok(())
    }

    /// Counts a step of the commit, `what` saying what was just done, and
    /// stops the commit there when a test asks: a unit test's stop unwinds
    /// with [`Stopped`], and leaves everything as a killed process would;
    /// with the `test-seams` feature, a process asked to stop by
    /// `AGENTDRAFTER_TEST_STOP_COMMIT_AT` says so on its standard error and
    /// waits there to be killed.
    fn passed(&mut self, what: impl Fn() -> String) {
        let step = self.next;
        self.next += 1;
        #[cfg(test)]
        {
            let stop = if self.restoring {
                &STOP_RESTORE_AT
            } else {
                &STOP_AT
            };
            if stop.with(|s| s.get() == Some(step)) {
                stop.with(|s| s.set(None));
                std::panic::resume_unwind(Box::new(Stopped(step)));
            }
        }
        #[cfg(feature = "test-seams")]
        if !self.restoring
            && std::env::var(STOP_VARIABLE)
                .ok()
                .and_then(|v| v.parse().ok())
                == Some(step)
        {
            eprintln!(
                "agentdrafter: stopped the commit at step {step}, once {}",
                what()
            );
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        let _ = (step, what, self.restoring);
    }
}

/// The variable that stops a commit at a step, with the `test-seams`
/// feature only, so a test can kill the process there.
#[cfg(feature = "test-seams")]
const STOP_VARIABLE: &str = "AGENTDRAFTER_TEST_STOP_COMMIT_AT";

/// What a unit test's stopped commit unwinds with: the step it stopped at.
#[cfg(test)]
#[derive(Debug)]
pub(super) struct Stopped(pub usize);

#[cfg(test)]
thread_local! {
    /// The replacement a test asks to fail, by its place in the commit.
    pub(super) static FAIL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Whether a test asks the pending entry's append to fail once its
    /// bytes are written, as a failed sync would, and, when `true`,
    /// cutting it back to fail too.
    pub(super) static FAIL_SYNC: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// The step a test asks a commit to stop at, as a killed process stops.
    pub(crate) static STOP_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// As [`FAIL_SYNC`], for the line that marks the entry done.
    pub(super) static FAIL_DONE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// The step a test asks a restore to stop at, as a killed process stops.
    pub(super) static STOP_RESTORE_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// The seam that fails the sync of `line`, and cutting it back, for a test.
#[cfg(test)]
fn sync_seam(line: Sync) -> Option<&'static std::thread::LocalKey<std::cell::Cell<Option<bool>>>> {
    match line {
        Sync::Pending => Some(&FAIL_SYNC),
        Sync::Done => Some(&FAIL_DONE),
        Sync::Plain => None,
    }
}

/// Fails the append's sync once a test asks for it.
fn fail_sync(line: Sync) -> io::Result<()> {
    #[cfg(test)]
    if sync_seam(line).is_some_and(|seam| seam.with(std::cell::Cell::get).is_some()) {
        return Err(io::Error::other("a failed sync the test asked for"));
    }
    let _ = line;
    Ok(())
}

/// Fails cutting the append back once, when a test asks for it.
fn fail_cut_back(line: Sync) -> io::Result<()> {
    #[cfg(test)]
    if sync_seam(line).is_some_and(|seam| seam.with(|f| f.replace(None)) == Some(true)) {
        return Err(io::Error::other("a failed cut the test asked for"));
    }
    let _ = line;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::original::regular_file_id_no_follow;

    #[test]
    fn original_commit_failure_restores_touched_file_and_never_overwrites_replaced_file() {
        let dir = std::env::temp_dir().join(format!(
            "agentdrafter-commit-orig-rollback-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dir = fs::canonicalize(&dir).unwrap();

        let source = dir.join("ORIG.dwg");
        fs::write(&source, b"original-dwg").unwrap();
        let source_id = regular_file_id_no_follow(&source);

        let file1 = dir.join("ORIG.rev1.dwg");
        let file2 = dir.join("ORIG.rev1.changes.json");
        fs::write(&file1, b"rev1-dwg-bytes").unwrap();
        fs::write(&file2, b"rev1-changes-bytes").unwrap();
        let id1 = regular_file_id_no_follow(&file1);
        let stale_id2 = regular_file_id_no_follow(&file2);

        // Save undo states for both files before committing (just as `undo` does).
        let key = "orig-000000000000";
        let scope = "orig-000000000000-1111111111111111";
        let jdir = dir.join(".agentdrafter").join("originals").join(key);
        let states_dir = jdir.join("undo-states");
        fs::create_dir_all(&states_dir).unwrap();
        let p1 = fingerprint(b"rev1-dwg-bytes");
        let p2 = fingerprint(b"rev1-changes-bytes");
        fs::write(states_dir.join(&p1), b"rev1-dwg-bytes").unwrap();
        fs::write(states_dir.join(&p2), b"rev1-changes-bytes").unwrap();
        let journal = jdir.join("journal.jsonl");
        fs::write(&journal, b"{\"entry\":1,\"kind\":\"change_set\"}\n").unwrap();

        // Externally replace file2 with a brand-new file (new FileId & content)
        // BEFORE `commit_original_replacements` runs!
        let replacement_tmp = dir.join("external-new.json");
        fs::write(&replacement_tmp, b"external-brand-new-content").unwrap();
        fs::rename(&replacement_tmp, &file2).unwrap();
        let new_id2 = regular_file_id_no_follow(&file2);
        #[cfg(any(unix, windows))]
        assert_ne!(
            new_id2, stale_id2,
            "replacement must have a distinct FileId"
        );

        let before = vec![
            Before {
                path: file1.clone(),
                print: Some(p1),
            },
            Before {
                path: file2.clone(),
                print: Some(p2),
            },
        ];
        let replacements = vec![
            Replacement {
                path: file1.clone(),
                bytes: None,
                what: "rev1 DWG",
            },
            Replacement {
                path: file2.clone(),
                bytes: None,
                what: "rev1 changes",
            },
        ];
        let expected = vec![
            ExpectedTarget::Existing(id1),
            ExpectedTarget::Existing(stale_id2),
        ];
        let guard = OriginalGuard {
            key,
            temp_scope: scope,
            base_dir: &dir,
            orig_dir_real: &dir,
            source: &source,
            source_id,
        };
        let ahead = Ahead {
            journal: &journal,
            entry: 2,
            pending: b"{\"entry\":2,\"kind\":\"undo\",\"state\":\"pending\"}\n",
            done: b"{\"entry\":2,\"state\":\"done\"}\n",
            before: &before,
            states: &states_dir,
            folders_made: &[],
        };

        let err = commit_original_replacements(&ahead, &replacements, &expected, guard)
            .expect_err("must refuse file2 whose FileId changed");
        assert_eq!(err.code, ErrorCode::WriteFailed);

        // File 1 (moved aside by this commit before failing on file 2) MUST be
        // restored byte-for-byte!
        assert_eq!(fs::read(&file1).unwrap(), b"rev1-dwg-bytes");
        // File 2 (externally replaced, rejected by `place`) MUST NOT be
        // overwritten with saved undo-state bytes — its new bytes & FileId
        // must remain completely intact!
        assert_eq!(fs::read(&file2).unwrap(), b"external-brand-new-content");
        #[cfg(any(unix, windows))]
        assert_eq!(regular_file_id_no_follow(&file2), new_id2);
        // Journal was cleanly cut back to entry 1.
        assert_eq!(
            fs::read(&journal).unwrap(),
            b"{\"entry\":1,\"kind\":\"change_set\"}\n"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
