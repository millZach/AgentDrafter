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
//! The files a commit writes beside their places are named by its entry,
//! `.agentdrafter-entry-<entry>-<n>.tmp` and `.old`, so a rollback knows
//! what a killed commit left and takes it away.

use std::cmp::Reverse;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, ErrorCode};
use crate::stamp::fingerprint;

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

/// Where one replacement got to: the file that was there, moved aside.
struct Placed {
    aside: Option<PathBuf>,
}

/// Why files could not all be put in place: the error, and the files that
/// could not be put back as they were. Boxed where it is given, since an
/// error is large.
struct Unplaced {
    error: Error,
    not_back: Vec<String>,
}

/// Writes `ahead`'s entry, pending, then puts every file of `replacements`
/// in place, the undo states first, then marks the entry done; or, when a
/// step fails, puts every file back and cuts the journal back, so nothing is
/// changed. On failure the error says what is left.
pub(super) fn commit(ahead: &Ahead, replacements: &[Replacement]) -> Result<(), Error> {
    let mut seam = Seam::commit();
    let length = match add_to_end(ahead.journal, ahead.pending, true) {
        Ok(length) => length,
        Err(failed) => return Err(not_added(ahead, failed)),
    };
    seam.passed(|| "its entry was written, pending".to_owned());
    let names = Names::new(ahead.entry, "");
    let placed = match put_in_place(replacements, &names, &mut seam) {
        Ok(placed) => placed,
        Err(unplaced) => return Err(not_finished(ahead, length, *unplaced)),
    };
    finish(replacements, &placed);
    seam.passed(|| "every file was in place".to_owned());
    let done = seam
        .fail(replacements.len())
        .map_err(AddFailed::from)
        .and_then(|()| add_to_end(ahead.journal, ahead.done, false).map(|_| ()));
    if let Err(failed) = done {
        let error = Error::new(
            ErrorCode::WriteFailed,
            format!(
                "could not mark entry {} done in the set's journal ({})",
                ahead.entry, failed.error
            ),
        )
        .in_file(ahead.journal);
        return Err(not_finished(
            ahead,
            length,
            Unplaced {
                error,
                not_back: Vec::new(),
            },
        ));
    }
    Ok(())
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

/// Puts every file back as it was before the commit, through the same
/// [`restore`] a rollback uses, and cuts the journal back to `length`, its
/// length before the entry, so that nothing was changed. When either cannot
/// be done, the entry stays pending and the error says so: the next command
/// rolls it back.
fn not_finished(ahead: &Ahead, length: u64, unplaced: Unplaced) -> Error {
    let Unplaced { error, not_back } = unplaced;
    let base = error.message.clone();
    let message = match restore(ahead.entry, ahead.before, ahead.states, ahead.folders_made) {
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
        Ok(_) => match cut_back(ahead.journal, length) {
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
    let mut replacements: Vec<Replacement> = Vec::new();
    let mut restored: Vec<Restored> = Vec::new();
    for file in before {
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
    }
    let placed = put_in_place(
        &replacements,
        &Names::new(entry, "back-"),
        &mut Seam::none(),
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
    finish(&replacements, &placed);
    let mut folders = folders_made.to_vec();
    folders.sort_by_key(|f| Reverse(f.components().count()));
    // What the commit left beside its files goes, wherever it is.
    let mut beside: Vec<&Path> = before.iter().filter_map(|f| f.path.parent()).collect();
    beside.push(states);
    beside.extend(folders.iter().map(PathBuf::as_path));
    let prefix = Names::new(entry, "").prefix;
    for folder in beside {
        let Ok(listed) = fs::read_dir(folder) else {
            continue;
        };
        for item in listed.flatten() {
            if item.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_file(item.path());
            }
        }
    }
    for folder in folders {
        let _ = fs::remove_dir(folder);
    }
    Ok(restored)
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
    add_to_end(path, line, false).map(|_| ()).map_err(|failed| {
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

/// The names a commit's files take beside their places: by its entry, and
/// for a rollback's own after `back-`.
struct Names {
    prefix: String,
}

impl Names {
    fn new(entry: u64, part: &str) -> Self {
        Self {
            prefix: format!(".agentdrafter-entry-{entry}-{part}"),
        }
    }

    /// The name beside `path` of the file `n` of the commit, ending in
    /// `ending`.
    fn beside(&self, path: &Path, n: usize, ending: &str) -> PathBuf {
        path.with_file_name(format!("{}{n}.{ending}", self.prefix))
    }
}

/// Puts every file of `replacements` in place, or none: each new file is
/// written whole beside its place, then each is moved into it, the file
/// there moved aside. When a step fails, every file already moved is put
/// back and every new file taken away, with the folders made for them.
fn put_in_place(
    replacements: &[Replacement],
    names: &Names,
    seam: &mut Seam,
) -> Result<Vec<Placed>, Box<Unplaced>> {
    let mut made: Vec<PathBuf> = Vec::new();
    let mut staged: Vec<Option<PathBuf>> = Vec::new();
    if let Err(error) = stage(replacements, names, &mut made, &mut staged, seam) {
        clean(&staged, &made);
        return Err(Box::new(Unplaced {
            error,
            not_back: Vec::new(),
        }));
    }
    let mut placed: Vec<Placed> = Vec::with_capacity(replacements.len());
    for (i, (replacement, temporary)) in replacements.iter().zip(&staged).enumerate() {
        match place(i, replacement, temporary.as_deref(), names, seam) {
            Ok(done) => placed.push(done),
            Err(e) => {
                let not_back = roll_back(&replacements[..placed.len()], &placed);
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
fn finish(replacements: &[Replacement], placed: &[Placed]) {
    for (replacement, done) in replacements.iter().zip(placed) {
        if let Some(aside) = &done.aside {
            let _ = fs::remove_file(aside);
        }
        if replacement.bytes.is_none() {
            if let Some(parent) = replacement.path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    }
}

/// Makes the folders the new files go in, and writes each beside its place.
fn stage(
    replacements: &[Replacement],
    names: &Names,
    made: &mut Vec<PathBuf>,
    staged: &mut Vec<Option<PathBuf>>,
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
        let temporary = names.beside(path, i, "tmp");
        let written = File::create(&temporary).and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
        if let Err(e) = written {
            let _ = fs::remove_file(&temporary);
            return Err(Error::new(
                ErrorCode::WriteFailed,
                format!("could not write {}: {e}", replacement.what),
            )
            .in_file(path));
        }
        staged.push(Some(temporary));
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

/// Adds `bytes` to the end of the file at `path`, making the file if it is
/// not there, and syncs it, and gives the file's length before. When they
/// cannot all be written, the file is cut back to its length before, or
/// taken away if it was not there; when that fails too, the failure says
/// why. `seam` lets a test fail the sync.
fn add_to_end(path: &Path, bytes: &[u8], seam: bool) -> Result<u64, AddFailed> {
    let existed = fs::symlink_metadata(path).is_ok();
    // Opened to write, not only to append, so that Windows lets it be cut
    // back.
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    let length = file.seek(SeekFrom::End(0))?;
    let written = file
        .write_all(bytes)
        .and_then(|()| if seam { fail_sync() } else { Ok(()) })
        .and_then(|()| file.sync_all());
    if let Err(error) = written {
        let back = (if seam { fail_cut_back() } else { Ok(()) })
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
/// that is taken out again.
fn cut_back(path: &Path, length: u64) -> io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(length)?;
    file.sync_all()
}

/// Moves the file at the replacement's path, if there is one, aside, and
/// the new one from `temporary`, if there is one, into its place. When the
/// new one cannot be moved, the old one is put back before the error is
/// given.
fn place(
    index: usize,
    replacement: &Replacement,
    temporary: Option<&Path>,
    names: &Names,
    seam: &mut Seam,
) -> io::Result<Placed> {
    seam.fail(index)?;
    let path = &replacement.path;
    let aside = if fs::symlink_metadata(path).is_ok() {
        let aside = names.beside(path, index, "old");
        fs::rename(path, &aside)?;
        seam.passed(|| format!("{} was moved aside", path.display()));
        Some(aside)
    } else {
        None
    };
    if let Some(temporary) = temporary {
        if let Err(e) = fs::rename(temporary, path) {
            if let Some(aside) = &aside {
                let _ = fs::rename(aside, path);
            }
            return Err(e);
        }
        seam.passed(|| format!("{} was put in its place", path.display()));
    }
    Ok(Placed { aside })
}

/// Puts back, last first, every file `placed` moved, and gives the files it
/// could not put back.
fn roll_back(replacements: &[Replacement], placed: &[Placed]) -> Vec<String> {
    let mut not_back = Vec::new();
    for (replacement, done) in replacements.iter().zip(placed).rev() {
        let path = &replacement.path;
        let back = match (&done.aside, &replacement.bytes) {
            (Some(aside), _) => fs::rename(aside, path),
            (None, Some(_)) => fs::remove_file(path),
            (None, None) => Ok(()),
        };
        if back.is_err() {
            not_back.push(path.display().to_string());
        }
    }
    not_back
}

/// Takes away the new files not moved into place and the folders made for
/// them, deepest first, each only when it is empty.
fn clean(staged: &[Option<PathBuf>], made: &[PathBuf]) {
    for temporary in staged.iter().flatten() {
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
/// before it is marked done, is counted from 0. A rollback's own steps are
/// not counted.
struct Seam {
    next: usize,
    counted: bool,
}

impl Seam {
    fn commit() -> Self {
        Self {
            next: 0,
            counted: true,
        }
    }

    fn none() -> Self {
        Self {
            next: 0,
            counted: false,
        }
    }

    /// Fails replacement `index`, or the line that marks the entry done
    /// after the last, once, when a test asks for it, so a test can show a
    /// failed commit puts every file back.
    fn fail(&self, index: usize) -> io::Result<()> {
        #[cfg(test)]
        if self.counted && FAIL_AT.with(|f| f.get() == Some(index) && f.replace(None).is_some()) {
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
        if !self.counted {
            return;
        }
        #[cfg(test)]
        if STOP_AT.with(|s| s.get() == Some(step)) {
            STOP_AT.with(|s| s.set(None));
            std::panic::resume_unwind(Box::new(Stopped(step)));
        }
        #[cfg(feature = "test-seams")]
        if std::env::var(STOP_VARIABLE)
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
        let _ = (step, what);
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
    pub(super) static STOP_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Fails the append's sync once a test asks for it.
fn fail_sync() -> io::Result<()> {
    #[cfg(test)]
    if FAIL_SYNC.with(std::cell::Cell::get).is_some() {
        return Err(io::Error::other("a failed sync the test asked for"));
    }
    Ok(())
}

/// Fails cutting the append back once, when a test asks for it.
fn fail_cut_back() -> io::Result<()> {
    #[cfg(test)]
    if FAIL_SYNC.with(|f| f.replace(None)) == Some(true) {
        return Err(io::Error::other("a failed cut the test asked for"));
    }
    Ok(())
}
