//! Leftover outputs: the DWGs and PDFs in a drawing set's output folder
//! that AgentDrafter made for the set and that no row writes any more,
//! since their sheet was renamed, merged or taken out of the rows.
//!
//! Every build and plot of a set lists them under `leftovers`, so that an
//! old sheet does not look current and get issued by mistake. Zach decided
//! on 2026-11-20 (#109) that they are only listed unless cleaning is asked
//! for, by `clean` for one run or by `clean: true` in the set's settings,
//! and that cleaning moves them into the folder [`SUPERSEDED_FOLDER`] in
//! the output folder, keeping their names, and never deletes them: a move
//! never replaces a file already there.
//!
//! Only a file AgentDrafter made for this very set is ever a leftover: a
//! DWG whose build mark names the set, as the set's own build writes it
//! from its output folder, and a PDF whose stamp names one of the set's
//! rows files. Any other file, an original among them, is never listed and
//! never moved (ADR-0004). A file a row writes is told by the file itself,
//! not by how its name compares: it is the file at a row's output path.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Condvar, Mutex, OnceLock};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::build::{file_id_of_path, is_same_file, FileId};
use crate::description::RowsFile;
use crate::dwg;
use crate::error::{Error, ErrorCode};
use crate::original::rename_no_replace_keep_src;
use crate::set::SET_SETTINGS_FILE;
use crate::stamp::{self, SourceKind};

/// How long a build, plot or change waits for another process writing or
/// cleaning the same output folder to finish (#238).
#[cfg(not(target_arch = "wasm32"))]
const OUTPUT_LOCK_WAIT: Duration = Duration::from_secs(30);

/// Holds an exclusive lock on a set's output folder across writing and
/// leftover cleanup so concurrent builds, plots or changes on the same output
/// folder never race each other (#238). Stored in the system temp folder so no
/// lock file ever appears inside the set or its output folder.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct OutputLock {
    _file: fs::File,
    _in_process: InProcessOutputLock,
}

#[cfg(target_arch = "wasm32")]
pub(crate) struct OutputLock;

#[cfg(not(target_arch = "wasm32"))]
struct InProcessLocks {
    held: Mutex<HashSet<String>>,
    changed: Condvar,
}

#[cfg(not(target_arch = "wasm32"))]
static IN_PROCESS_OUTPUT_LOCKS: OnceLock<InProcessLocks> = OnceLock::new();

#[cfg(not(target_arch = "wasm32"))]
struct InProcessOutputLock {
    key: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for InProcessOutputLock {
    fn drop(&mut self) {
        if let Some(locks) = IN_PROCESS_OUTPUT_LOCKS.get() {
            let mut held = locks.held.lock().unwrap_or_else(|e| e.into_inner());
            held.remove(&self.key);
            locks.changed.notify_all();
        }
    }
}

/// Acquires an exclusive in-process and cross-process lock for `output`,
/// waiting up to [`OUTPUT_LOCK_WAIT`] for another build, plot or change to
/// release it (#238).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn lock_output(output: &Path) -> Result<OutputLock, Error> {
    let canonical = fs::canonicalize(output)
        .or_else(|_| std::path::absolute(output))
        .unwrap_or_else(|_| output.to_path_buf());
    let key = stamp::fingerprint(canonical.as_os_str().as_encoded_bytes());
    let started = Instant::now();
    let failed = |why: String| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not lock the set's output folder: {why}"),
        )
        .in_file(output)
    };

    // 1. In-process mutual exclusion (since advisory OS locks may be shared
    //    within one process on some platforms).
    let locks = IN_PROCESS_OUTPUT_LOCKS.get_or_init(|| InProcessLocks {
        held: Mutex::new(HashSet::new()),
        changed: Condvar::new(),
    });
    let in_process = {
        let mut held = locks.held.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if held.insert(key.clone()) {
                break InProcessOutputLock { key: key.clone() };
            }
            let Some(left) = OUTPUT_LOCK_WAIT.checked_sub(started.elapsed()) else {
                return Err(failed(format!(
                    "another build or plot has held the output folder for {} seconds",
                    OUTPUT_LOCK_WAIT.as_secs()
                )));
            };
            let (guard, _) = locks
                .changed
                .wait_timeout(held, left)
                .unwrap_or_else(|e| e.into_inner());
            held = guard;
        }
    };

    // 2. Cross-process OS lock on a per-output-folder lock file in temp dir.
    let lock_dir = output_lock_dir();
    create_lock_dir(&lock_dir).map_err(|e| failed(e.to_string()))?;
    let lock_path = lock_dir.join(format!("output-{key}.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| failed(e.to_string()))?;
    loop {
        match file.try_lock() {
            Ok(()) => {
                return Ok(OutputLock {
                    _file: file,
                    _in_process: in_process,
                })
            }
            Err(fs::TryLockError::WouldBlock) if started.elapsed() < OUTPUT_LOCK_WAIT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(fs::TryLockError::WouldBlock) => {
                return Err(failed(format!(
                    "another process has held the output folder for {} seconds",
                    OUTPUT_LOCK_WAIT.as_secs()
                )))
            }
            Err(fs::TryLockError::Error(e)) => return Err(failed(e.to_string())),
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn lock_output(_output: &Path) -> Result<OutputLock, Error> {
    Ok(OutputLock)
}

#[cfg(not(target_arch = "wasm32"))]
fn output_lock_dir() -> PathBuf {
    #[cfg(unix)]
    {
        extern "C" {
            fn geteuid() -> u32;
        }
        // SAFETY: `geteuid(2)` takes no arguments and never fails.
        let uid = unsafe { geteuid() };
        std::env::temp_dir().join(format!("agentdrafter-locks-{uid}"))
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("agentdrafter-locks")
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn create_lock_dir(dir: &Path) -> io::Result<()> {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// The folder in a set's output folder that cleaning moves leftover
/// outputs into.
pub const SUPERSEDED_FOLDER: &str = "superseded";

/// The most numbered names tried for a leftover whose name is taken in
/// [`SUPERSEDED_FOLDER`], before it is left where it is.
const MOST_NUMBERED: u32 = 9_999;

/// A DWG or PDF in a set's output folder that AgentDrafter made for the
/// set and that no row writes any more; or one no row writes that could
/// not be read to tell, marked `unreadable`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Leftover {
    /// The file, in the set's output folder.
    pub file: String,
    /// What it is: `dwg` or `pdf`.
    pub kind: LeftoverKind,
    /// The sheet it was made for: its name without `.dwg` or `.pdf`, which
    /// no row of the set names any more.
    pub sheet: String,
    /// True when the file could not be read to tell whether AgentDrafter
    /// made it for the set: it may or may not be a leftover, and it is
    /// never moved. Its `error` says why. Left out when false.
    #[serde(skip_serializing_if = "is_false")]
    pub unreadable: bool,
    /// Where cleaning moved it, in the output folder's
    /// [`SUPERSEDED_FOLDER`]. None when it was only listed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
    /// Why it could not be read, when it is `unreadable`; or why cleaning
    /// could not move it, when it could not: it is still where it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// What kind of output a leftover is. Written in `snake_case` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeftoverKind {
    /// A DWG a build wrote.
    Dwg,
    /// A DXF a build wrote.
    Dxf,
    /// A PDF a plot wrote.
    Pdf,
}

/// What a set's outputs are, for finding its leftovers.
pub(crate) struct SetOutputs<'a> {
    /// The set's folder.
    pub folder: &'a Path,
    /// The set's output folder, joined onto its folder.
    pub output: &'a Path,
    /// The set as its DWGs' build marks name it, from the output folder
    /// ([`crate::set::set_in_mark`]); none when they name no set, and then
    /// no DWG can be told to be this set's.
    pub marked: Option<&'a str>,
    /// The set's rows files, as they were read.
    pub rows_files: &'a [RowsFile],
    /// Each sheet the set's rows name.
    pub sheets: Vec<&'a str>,
}

/// What looking for a set's leftovers found.
pub(crate) struct Scan {
    /// The leftovers, in the byte order of their names; none when the
    /// output folder could not be listed, so that no one takes an empty
    /// list for a folder looked through.
    pub leftovers: Option<Vec<Leftover>>,
    /// Why the output folder could not be listed, when it could not.
    pub error: Option<Error>,
}

/// One leftover candidate recorded during [`find`] with the file identity it
/// had when examined, so [`clean_up`] refuses to move it if another process
/// replaced it in the meantime (#238).
struct Candidate {
    leftover: Leftover,
    id: Option<FileId>,
}

/// Looks for the leftover outputs of a set in its output folder, and moves
/// them into [`SUPERSEDED_FOLDER`] when `clean` asks.
pub(crate) fn leftovers(set: &SetOutputs, clean: bool) -> Scan {
    match find(set) {
        Ok(found) => Scan {
            leftovers: Some(if clean {
                clean_up(set, found)
            } else {
                found.into_iter().map(|c| c.leftover).collect()
            }),
            error: None,
        },
        Err(error) => Scan {
            leftovers: None,
            error: Some(error),
        },
    }
}

/// The error a set's reply carries when looking for its leftovers did not
/// finish, or cleaning did not, if either did not: the output folder could
/// not be listed (`read_failed`), a file in it could not be read to tell
/// (`read_failed`), or a leftover could not be moved (`write_failed`).
pub(crate) fn scan_error(
    scan_failed: Option<&Error>,
    leftovers: Option<&[Leftover]>,
) -> Option<Error> {
    if let Some(error) = scan_failed {
        return Some(error.clone());
    }
    let leftovers = leftovers?;
    let unreadable = leftovers.iter().filter(|l| l.unreadable).count();
    if unreadable > 0 {
        return Some(Error::new(
            ErrorCode::ReadFailed,
            format!(
                "{unreadable} files in the set's output folder could not be read to tell whether \
                 they are the set's leftover outputs; they were not moved, and each one is under \
                 `leftovers`, marked `unreadable`, with its error"
            ),
        ));
    }
    let stuck = leftovers.iter().filter(|l| l.error.is_some()).count();
    (stuck > 0).then(|| {
        Error::new(
            ErrorCode::WriteFailed,
            format!(
                "{stuck} of the set's {} leftover outputs could not be moved into \
                 `{SUPERSEDED_FOLDER}` and are still where they were; each one's error is under \
                 `leftovers`",
                leftovers.len()
            ),
        )
    })
}

/// The files of the output folder no row writes that AgentDrafter made for
/// the set, or that could not be read to tell, in the byte order of their
/// names. An error when the folder cannot be listed to the end.
fn find(set: &SetOutputs) -> Result<Vec<Candidate>, Error> {
    let not_listed = |e: io::Error| {
        Error::new(
            ErrorCode::ReadFailed,
            format!(
                "could not list the set's output folder to look for leftover outputs, so none \
                 was looked for or moved: {e}"
            ),
        )
        .in_file(set.output)
    };
    let written = Written::new(set);
    let mut found = Vec::new();
    for entry in fs::read_dir(set.output).map_err(not_listed)? {
        let entry = entry.map_err(not_listed)?;
        // A name that is not text is no sheet's: every sheet's name is.
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some((sheet, kind)) = output_kind(&name).map(|(s, k)| (s.to_owned(), k)) else {
            continue;
        };
        // A row's own output path, as it writes it: the file is the row's.
        if written.names.contains(name.as_str()) {
            continue;
        }
        let path = entry.path();
        let unreadable = |e: io::Error| Candidate {
            leftover: Leftover {
                file: path.display().to_string(),
                kind,
                sheet: sheet.to_owned(),
                unreadable: true,
                moved_to: None,
                error: Some(
                    Error::new(
                        ErrorCode::ReadFailed,
                        format!(
                            "could not read the file to tell whether AgentDrafter made it for the \
                             set: {e}; it was not moved"
                        ),
                    )
                    .in_file(&path),
                ),
            },
            id: None,
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) => {
                found.push((name, unreadable(e)));
                continue;
            }
        };
        // A link is no file AgentDrafter made, whatever it leads to; nor is
        // a folder.
        if !is_plain_file(&metadata) {
            continue;
        }
        // A row writes it under a name that compares otherwise, such as in
        // other letter case where the file system ignores case.
        if written.writes(&path, &sheet) {
            continue;
        }
        let id = file_id_of_path(&path);
        match is_sets(set, kind, &path) {
            Ok(true) => found.push((
                name,
                Candidate {
                    leftover: Leftover {
                        file: path.display().to_string(),
                        kind,
                        sheet: sheet.to_owned(),
                        unreadable: false,
                        moved_to: None,
                        error: None,
                    },
                    id,
                },
            )),
            Ok(false) => {}
            Err(e) => found.push((name, unreadable(e))),
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found.into_iter().map(|(_, candidate)| candidate).collect())
}

/// What the set's rows write in its output folder: each output's file
/// name, checked live against current row outputs so a file replaced under
/// its own name mid-run is never mistaken for a leftover (#238).
struct Written<'a> {
    output: &'a Path,
    sheets: &'a [&'a str],
    /// `<sheet>.dwg`, `<sheet>.dxf` and `<sheet>.pdf` for each row.
    names: HashSet<String>,
}

impl<'a> Written<'a> {
    fn new(set: &'a SetOutputs<'a>) -> Self {
        let names = set
            .sheets
            .iter()
            .flat_map(|sheet| {
                [
                    format!("{sheet}.dwg"),
                    format!("{sheet}.dxf"),
                    format!("{sheet}.pdf"),
                ]
            })
            .collect();
        Written {
            output: set.output,
            sheets: &set.sheets,
            names,
        }
    }

    /// Whether the file at `path`, named for `sheet`, is one a row writes:
    /// the very file at a row's output path right now, as the file system
    /// tells it. Where that cannot be told, by its name in any letter case,
    /// the safe way round: the file is then taken to be the row's, and left
    /// alone.
    fn writes(&self, path: &Path, sheet: &str) -> bool {
        is_current_output(self.output, self.sheets, path, sheet)
    }
}

/// Live check whether `path` (named for `sheet`) is currently one of the
/// set's row outputs (#238). Never relies on a stale snapshot of `FileId`s.
fn is_current_output(output: &Path, sheets: &[&str], path: &Path, sheet: &str) -> bool {
    if let Some(id) = file_id_of_path(path) {
        return sheets.iter().any(|row_sheet| {
            ["dwg", "dxf", "pdf"]
                .iter()
                .any(|ext| file_id_of_path(&output.join(format!("{row_sheet}.{ext}"))) == Some(id))
        });
    }
    let folded = sheet.to_lowercase();
    sheets.iter().any(|s| s.to_lowercase() == folded)
}

/// Whether `metadata`, read without following links, is of a plain file:
/// not a folder and not a link.
fn is_plain_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file() && !is_link(metadata)
}

/// Whether `metadata`, read without following links, is of a link: a
/// symbolic link, or on Windows any reparse point that names another
/// place, a junction among them, as Rust's standard library tells them. A
/// file OneDrive keeps in the cloud is a reparse point too, but names no
/// other place, and is not a link.
fn is_link(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// The sheet a file of the output folder named `name` is for, and what it
/// is: a DWG, a DXF or a PDF, by its extension in any case. None for any other
/// file.
fn output_kind(name: &str) -> Option<(&str, LeftoverKind)> {
    let (sheet, extension) = name.rsplit_once('.')?;
    let kind = if extension.eq_ignore_ascii_case("dwg") {
        LeftoverKind::Dwg
    } else if extension.eq_ignore_ascii_case("dxf") {
        LeftoverKind::Dxf
    } else if extension.eq_ignore_ascii_case("pdf") {
        LeftoverKind::Pdf
    } else {
        return None;
    };
    (!sheet.is_empty()).then_some((sheet, kind))
}

/// Whether the file at `path` of the output folder, a `kind`, is one
/// AgentDrafter made for the set. An error when it cannot be read.
fn is_sets(set: &SetOutputs, kind: LeftoverKind, path: &Path) -> io::Result<bool> {
    match kind {
        LeftoverKind::Dwg | LeftoverKind::Dxf => is_sets_dwg(set, path),
        LeftoverKind::Pdf => is_sets_pdf(set, path),
    }
}

/// Whether the DWG at `path`, in the set's output folder, is one the set
/// built: its build mark names the set as the set's own build names it
/// from there. A DWG with no mark, an original among them, or whose mark
/// names no set or another set, is not. An error when the file cannot be
/// read to tell, as a DWG cut short cannot ([`dwg::read_build_mark`]).
fn is_sets_dwg(set: &SetOutputs, path: &Path) -> io::Result<bool> {
    let Some(marked) = set.marked else {
        return Ok(false);
    };
    let mark = dwg::read_build_mark(path).map_err(io::Error::other)?;
    Ok(mark.is_some_and(|mark| mark.set.as_deref() == Some(marked)))
}

/// Whether the PDF at `path`, in the set's output folder, is one plotted
/// from the set: its stamp, which AgentDrafter alone writes, names a rows
/// file of the set. That is one of the set's rows files now, or a file in
/// the set's folder, gone or not, that no other set inside it holds, so
/// that a sheet whose whole rows file was taken out of the settings is
/// still known. An error when the file cannot be read.
fn is_sets_pdf(set: &SetOutputs, path: &Path) -> io::Result<bool> {
    let bytes = fs::read(path)?;
    if !bytes.starts_with(b"%PDF-") {
        return Ok(false);
    }
    let Ok(stamp) = stamp::stamp_of(&bytes) else {
        return Ok(false);
    };
    Ok(stamp
        .sources
        .iter()
        .filter(|source| source.kind == SourceKind::Rows)
        .any(|source| {
            let rows = stamp::resolve(path, &source.path);
            set.rows_files.iter().any(|f| is_same_file(&f.path, &rows))
                || set_holding(&rows).is_some_and(|folder| is_same_file(&folder, set.folder))
        }))
}

/// The folder of the set that holds the file at `path`: the nearest folder
/// above it with a set's settings in it.
fn set_holding(path: &Path) -> Option<PathBuf> {
    let whole = std::path::absolute(path).ok()?;
    whole
        .ancestors()
        .skip(1)
        .find(|folder| folder.join(SET_SETTINGS_FILE).is_file())
        .map(Path::to_path_buf)
}

/// Moves each of `found` that is a leftover into [`SUPERSEDED_FOLDER`],
/// made only when there is one to move. A sheet's leftover outputs (such as
/// its DWG and PDF) move together: if one cannot be moved, any sibling already
/// moved for that sheet is moved back so the pair is never split (#236). When
/// `superseded/` is a link or a junction, or cannot be made, nothing is moved
/// and each leftover carries the error.
fn clean_up(set: &SetOutputs, found: Vec<Candidate>) -> Vec<Leftover> {
    clean_up_with(
        found,
        &set.output.join(SUPERSEDED_FOLDER),
        |kind, path, sheet, expected_id| still_a_leftover(set, kind, path, sheet, expected_id),
        supersede,
    )
}

fn clean_up_with<Check, Move>(
    mut found: Vec<Candidate>,
    folder: &Path,
    mut check: Check,
    mut mover: Move,
) -> Vec<Leftover>
where
    Check: FnMut(LeftoverKind, &Path, &str, Option<FileId>) -> Result<(), Error>,
    Move: FnMut(&Path, &str, &Path) -> Result<PathBuf, Error>,
{
    if found.iter().all(|c| c.leftover.unreadable) {
        return found.into_iter().map(|c| c.leftover).collect();
    }
    let ready = superseded_folder(folder);
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (idx, candidate) in found.iter().enumerate() {
        if candidate.leftover.unreadable {
            continue;
        }
        if let Some((_, indices)) = groups
            .iter_mut()
            .find(|(s, _)| s == &candidate.leftover.sheet)
        {
            indices.push(idx);
        } else {
            groups.push((candidate.leftover.sheet.clone(), vec![idx]));
        }
    }
    for (sheet, indices) in groups {
        if let Err(err) = &ready {
            for idx in indices {
                found[idx].leftover.error = Some(err.clone());
            }
            continue;
        }
        // Check every movable output of this sheet before moving any of them,
        // so a changed, replaced, current, or missing sibling leaves the whole
        // sheet untouched (#236, #238).
        let mut first_failed: Option<String> = None;
        for &idx in &indices {
            let path = PathBuf::from(&found[idx].leftover.file);
            if let Err(err) = check(
                found[idx].leftover.kind,
                &path,
                &found[idx].leftover.sheet,
                found[idx].id,
            ) {
                if first_failed.is_none() {
                    first_failed = Some(file_name_text(&path));
                }
                found[idx].leftover.error = Some(err);
            }
        }
        if let Some(failed_name) = first_failed {
            for &idx in &indices {
                if found[idx].leftover.error.is_none() {
                    let path = PathBuf::from(&found[idx].leftover.file);
                    found[idx].leftover.error =
                        Some(held_together_error(&path, &sheet, &failed_name));
                }
            }
            continue;
        }
        // Move each output of the sheet; if any fails, roll back earlier moves
        // for this sheet so its DWG/DXF and PDF never end up split (#236).
        let mut moved: Vec<(usize, PathBuf, PathBuf)> = Vec::new();
        let mut failed: Option<(usize, String)> = None;
        for (pos, &idx) in indices.iter().enumerate() {
            let path = PathBuf::from(&found[idx].leftover.file);
            let name = file_name_text(&path);
            match mover(folder, &name, &path) {
                Ok(dest) => {
                    found[idx].leftover.moved_to = Some(dest.display().to_string());
                    moved.push((idx, path, dest));
                }
                Err(err) => {
                    found[idx].leftover.error = Some(err);
                    failed = Some((pos, name));
                    break;
                }
            }
        }
        if let Some((failed_pos, failed_name)) = failed {
            for (prev_idx, orig_path, dest_path) in moved.into_iter().rev() {
                match move_without_replacing(&dest_path, &orig_path) {
                    Ok(()) => {
                        found[prev_idx].leftover.moved_to = None;
                        found[prev_idx].leftover.error =
                            Some(held_together_error(&orig_path, &sheet, &failed_name));
                    }
                    Err(e) => {
                        found[prev_idx].leftover.error = Some(
                            Error::new(
                                ErrorCode::WriteFailed,
                                format!(
                                    "could not be moved back after `{failed_name}` failed to move: {e}"
                                ),
                            )
                            .in_file(&orig_path),
                        );
                    }
                }
            }
            for &later_idx in &indices[failed_pos + 1..] {
                let later_path = PathBuf::from(&found[later_idx].leftover.file);
                found[later_idx].leftover.error =
                    Some(held_together_error(&later_path, &sheet, &failed_name));
            }
        }
    }
    found.into_iter().map(|c| c.leftover).collect()
}

fn file_name_text(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn held_together_error(path: &Path, sheet: &str, failed_name: &str) -> Error {
    Error::new(
        ErrorCode::WriteFailed,
        format!(
            "not moved into `{SUPERSEDED_FOLDER}` so `{sheet}`'s outputs stay together after \
             `{failed_name}` could not be moved"
        ),
    )
    .in_file(path)
}

/// Makes `folder`, the output folder's [`SUPERSEDED_FOLDER`], when it is
/// not there, and refuses it when it is a link or a junction, which would
/// move the leftovers somewhere else, or anything but a folder.
fn superseded_folder(folder: &Path) -> Result<(), Error> {
    let refused = |why: &str| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("`{SUPERSEDED_FOLDER}` in the output folder {why}, so no leftover was moved"),
        )
        .in_file(folder)
    };
    if fs::symlink_metadata(folder).is_err() {
        fs::create_dir(folder).or_else(|e| match e.kind() {
            ErrorKind::AlreadyExists => Ok(()),
            _ => Err(refused(&format!("could not be made: {e}"))),
        })?;
    }
    let metadata =
        fs::symlink_metadata(folder).map_err(|e| refused(&format!("could not be read: {e}")))?;
    if is_link(&metadata) {
        return Err(refused(
            "is a link or a junction, which would move the leftovers to another folder",
        ));
    }
    if !metadata.is_dir() {
        return Err(refused("is not a folder"));
    }
    Ok(())
}

/// Refuses to move the file at `path` unless it is still the same plain file
/// examined during [`find`], is not a current output of the set, and still
/// carries the set's build mark or stamp (#238).
fn still_a_leftover(
    set: &SetOutputs,
    kind: LeftoverKind,
    path: &Path,
    sheet: &str,
    expected_id: Option<FileId>,
) -> Result<(), Error> {
    let changed = |why: String| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("the leftover {why} since it was found, so it was not moved"),
        )
        .in_file(path)
    };
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| changed(format!("could not be read again ({e})")))?;
    if !is_plain_file(&metadata) {
        return Err(changed("is no longer a plain file".to_owned()));
    }
    if let Some(expected) = expected_id {
        if file_id_of_path(path) != Some(expected) {
            return Err(changed("was replaced".to_owned()));
        }
    }
    if is_current_output(set.output, &set.sheets, path, sheet) {
        return Err(changed("became a current output of the set".to_owned()));
    }
    match is_sets(set, kind, path) {
        Ok(true) => Ok(()),
        Ok(false) => Err(changed(
            "no longer carries the set's build mark or stamp".to_owned(),
        )),
        Err(e) => Err(changed(format!("could not be read again ({e})"))),
    }
}

/// Moves the leftover `name` at `path` into `folder`, the output folder's
/// [`SUPERSEDED_FOLDER`], keeping its name, and never over a file there:
/// when a file of that name is there, from an earlier clean, or appears
/// while the move is made, it is kept and the leftover takes the first free
/// name `<sheet> (2).<ext>`, `<sheet> (3).<ext>` and so on. Returns where
/// it went.
fn supersede(folder: &Path, name: &str, path: &Path) -> Result<PathBuf, Error> {
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    let failed = |e: io::Error| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not move the leftover into `{SUPERSEDED_FOLDER}`: {e}"),
        )
        .in_file(path)
    };
    let names = std::iter::once(name.to_owned())
        .chain((2..=MOST_NUMBERED).map(|n| format!("{stem} ({n}).{extension}")));
    for candidate in names {
        let to = folder.join(candidate);
        match move_without_replacing(path, &to) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(failed(e)),
        }
    }
    Err(Error::new(
        ErrorCode::WriteFailed,
        format!(
            "`{SUPERSEDED_FOLDER}` already holds `{name}` and every numbered name for it up to \
             ({MOST_NUMBERED})"
        ),
    )
    .in_file(path))
}

/// Moves the file at `from` to `to`, failing with
/// [`ErrorKind::AlreadyExists`] when anything is at `to`, and never
/// replacing it, even should it appear while the move is made: uses an atomic
/// no-replace rename (`MoveFileExW` on Windows, `renameat2 RENAME_NOREPLACE`
/// on Linux, `renamex_np RENAME_EXCL` on macOS) so there is no link-then-unlink
/// window where a concurrent replacement at `from` can be removed (#238).
/// Falls back to copying only when the volume does not support same-device
/// rename/link, re-checking `from`'s identity before removing it.
fn move_without_replacing(from: &Path, to: &Path) -> io::Result<()> {
    let before_id = file_id_of_path(from);
    match rename_no_replace_keep_src(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(e),
        Err(e) if is_cross_device_or_unsupported(&e) => {
            copy_new(from, to)?;
            if before_id.is_some() && file_id_of_path(from) != before_id {
                let _ = fs::remove_file(to);
                return Err(io::Error::other(
                    "the leftover was replaced while copying it",
                ));
            }
            fs::remove_file(from).inspect_err(|_| {
                let _ = fs::remove_file(to);
            })
        }
        Err(e) => Err(e),
    }
}

fn is_cross_device_or_unsupported(err: &io::Error) -> bool {
    if matches!(
        err.kind(),
        ErrorKind::CrossesDevices | ErrorKind::Unsupported
    ) {
        return true;
    }
    #[cfg(unix)]
    {
        // EXDEV = 18 on Linux and macOS.
        matches!(err.raw_os_error(), Some(18))
    }
    #[cfg(windows)]
    {
        // ERROR_INVALID_FUNCTION = 1, ERROR_NOT_SAME_DEVICE = 17.
        matches!(err.raw_os_error(), Some(1 | 17))
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Copies the file at `from` to a file made new at `to`, failing with
/// [`ErrorKind::AlreadyExists`] when anything is at `to`. The copy has the
/// source's permissions, never wider: on Unix it is made with no more than
/// the source's mode, whatever the umask, and given exactly that mode
/// before any byte is written; on Windows it is made read-only once its
/// bytes are written when the source is. A copy that does not finish is
/// taken away.
fn copy_new(from: &Path, to: &Path) -> io::Result<()> {
    let mut source = fs::File::open(from)?;
    let permissions = source.metadata()?.permissions();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(permissions.mode() & 0o7777);
    }
    let mut target = options.open(to)?;
    fill(&mut source, &mut target, permissions).inspect_err(|_| {
        let _ = fs::remove_file(to);
    })
}

/// Writes the bytes of `source` into `target`, a file just made, which
/// takes `permissions`: on Unix before the bytes are written, and elsewhere,
/// where the one permission is read-only, after them.
fn fill(
    source: &mut fs::File,
    target: &mut fs::File,
    permissions: fs::Permissions,
) -> io::Result<()> {
    #[cfg(unix)]
    target.set_permissions(permissions)?;
    io::copy(source, target)?;
    target.sync_all()?;
    #[cfg(not(unix))]
    if permissions.readonly() {
        target.set_permissions(permissions)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_dwg_or_a_pdf_is_an_output() {
        assert_eq!(
            output_kind("DEMO-004.dwg"),
            Some(("DEMO-004", LeftoverKind::Dwg))
        );
        assert_eq!(
            output_kind("DEMO-004.PDF"),
            Some(("DEMO-004", LeftoverKind::Pdf))
        );
        assert_eq!(output_kind("A.B.dwg"), Some(("A.B", LeftoverKind::Dwg)));
        assert_eq!(output_kind("DEMO-004.png"), None);
        assert_eq!(output_kind("DEMO-004"), None);
        assert_eq!(output_kind(".dwg"), None);
    }

    /// A copy keeps the source's mode exactly, neither wider, as a file made
    /// under the usual umask of 022 would be, nor narrower, as one made
    /// under a umask that takes away a group's writing would be.
    #[cfg(unix)]
    #[test]
    fn a_copy_keeps_the_source_s_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-copy-mode-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for mode in [0o640, 0o660, 0o600, 0o444] {
            let (from, to) = (
                dir.join(format!("{mode:o}.dwg")),
                dir.join(format!("{mode:o}-copy.dwg")),
            );
            fs::write(&from, b"leftover").unwrap();
            fs::set_permissions(&from, fs::Permissions::from_mode(mode)).unwrap();
            copy_new(&from, &to).unwrap();
            let copied = fs::metadata(&to).unwrap().permissions().mode() & 0o7777;
            assert_eq!(copied, mode, "{mode:o} came out {copied:o}");
            assert_eq!(fs::read(&to).unwrap(), b"leftover");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// On Windows a read-only source makes a read-only copy.
    #[cfg(windows)]
    #[test]
    fn a_copy_of_a_read_only_file_is_read_only() {
        let dir = std::env::temp_dir().join(format!(
            "agentdrafter-copy-read-only-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("from.dwg"), dir.join("to.dwg"));
        fs::write(&from, b"leftover").unwrap();
        let mut read_only = fs::metadata(&from).unwrap().permissions();
        read_only.set_readonly(true);
        fs::set_permissions(&from, read_only).unwrap();
        copy_new(&from, &to).unwrap();
        assert!(fs::metadata(&to).unwrap().permissions().readonly());
        assert_eq!(fs::read(&to).unwrap(), b"leftover");
        for path in [&from, &to] {
            let mut writable = fs::metadata(path).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            writable.set_readonly(false);
            fs::set_permissions(path, writable).unwrap();
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A move never replaces what is at the new name: it fails, and both
    /// files stay as they were; and a copy made new refuses the same.
    #[test]
    fn a_move_never_replaces_a_file() {
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-no-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("from.dwg"), dir.join("to.dwg"));
        fs::write(&from, b"leftover").unwrap();
        fs::write(&to, b"kept").unwrap();
        let refused = move_without_replacing(&from, &to).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            copy_new(&from, &to).unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&from).unwrap(), b"leftover");
        assert_eq!(fs::read(&to).unwrap(), b"kept");

        let free = dir.join("free.dwg");
        move_without_replacing(&from, &free).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(&free).unwrap(), b"leftover");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_sibling_move_rolls_back_the_sheets_first_file_without_blocking_other_sheets() {
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-pair-rollback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sup = dir.join(SUPERSEDED_FOLDER);

        let dwg2 = dir.join("DEMO-002.dwg");
        let pdf2 = dir.join("DEMO-002.pdf");
        let dwg3 = dir.join("DEMO-003.dwg");
        let pdf3 = dir.join("DEMO-003.pdf");
        fs::write(&dwg2, b"dwg-2").unwrap();
        fs::write(&pdf2, b"pdf-2").unwrap();
        fs::write(&dwg3, b"dwg-3").unwrap();
        fs::write(&pdf3, b"pdf-3").unwrap();

        let item = |path: &Path, kind: LeftoverKind, sheet: &str| Candidate {
            leftover: Leftover {
                file: path.display().to_string(),
                kind,
                sheet: sheet.to_owned(),
                unreadable: false,
                moved_to: None,
                error: None,
            },
            id: file_id_of_path(path),
        };
        let found = vec![
            item(&dwg2, LeftoverKind::Dwg, "DEMO-002"),
            item(&pdf2, LeftoverKind::Pdf, "DEMO-002"),
            item(&dwg3, LeftoverKind::Dwg, "DEMO-003"),
            item(&pdf3, LeftoverKind::Pdf, "DEMO-003"),
        ];

        // Moving DEMO-002.pdf fails after DEMO-002.dwg was already moved:
        // DEMO-002.dwg must be rolled back so DEMO-002 is not split, while
        // DEMO-003 still moves cleanly.
        let out = clean_up_with(
            found,
            &sup,
            |_, _, _, _| Ok(()),
            |folder, name, path| {
                if name == "DEMO-002.pdf" {
                    return Err(
                        Error::new(ErrorCode::WriteFailed, "file is held open").in_file(path)
                    );
                }
                supersede(folder, name, path)
            },
        );

        assert!(out[0].moved_to.is_none(), "{:#?}", out[0]);
        assert!(
            out[0]
                .error
                .as_ref()
                .unwrap()
                .message
                .contains("DEMO-002.pdf"),
            "{:#?}",
            out[0]
        );
        assert!(out[1].moved_to.is_none(), "{:#?}", out[1]);
        assert!(
            out[1].error.as_ref().unwrap().message.contains("held open"),
            "{:#?}",
            out[1]
        );
        assert_eq!(fs::read(&dwg2).unwrap(), b"dwg-2");
        assert_eq!(fs::read(&pdf2).unwrap(), b"pdf-2");
        assert!(!sup.join("DEMO-002.dwg").exists());
        assert!(!sup.join("DEMO-002.pdf").exists());

        // DEMO-003 moved intact.
        assert!(out[2].moved_to.is_some() && out[2].error.is_none());
        assert!(out[3].moved_to.is_some() && out[3].error.is_none());
        assert!(!dwg3.exists() && !pdf3.exists());
        assert_eq!(fs::read(sup.join("DEMO-003.dwg")).unwrap(), b"dwg-3");
        assert_eq!(fs::read(sup.join("DEMO-003.pdf")).unwrap(), b"pdf-3");

        // Pre-check failure on one sibling holds back the other without moving either.
        let out2 = clean_up_with(
            vec![
                item(&dwg2, LeftoverKind::Dwg, "DEMO-002"),
                item(&pdf2, LeftoverKind::Pdf, "DEMO-002"),
            ],
            &sup,
            |kind, path, _, _| {
                if kind == LeftoverKind::Pdf {
                    Err(Error::new(ErrorCode::WriteFailed, "changed").in_file(path))
                } else {
                    Ok(())
                }
            },
            supersede,
        );
        assert!(out2[0].moved_to.is_none() && out2[0].error.is_some());
        assert!(out2[1].moved_to.is_none() && out2[1].error.is_some());
        assert_eq!(fs::read(&dwg2).unwrap(), b"dwg-2");
        assert_eq!(fs::read(&pdf2).unwrap(), b"pdf-2");
        assert!(!sup.join("DEMO-002.dwg").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn still_a_leftover_refuses_a_replaced_file_or_current_output() {
        let dir = std::env::temp_dir().join(format!(
            "agentdrafter-leftover-replaced-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("OLD.dwg");
        fs::write(&file, b"old").unwrap();
        let old_id = file_id_of_path(&file);

        // Replace OLD.dwg atomically via rename (giving it a new file identity).
        let replacement = dir.join("OLD.dwg.new");
        fs::write(&replacement, b"new").unwrap();
        fs::rename(&replacement, &file).unwrap();

        let set = SetOutputs {
            folder: &dir,
            output: &dir,
            marked: None,
            rows_files: &[],
            sheets: vec!["CURRENT"],
        };
        let err = still_a_leftover(&set, LeftoverKind::Dwg, &file, "OLD", old_id).unwrap_err();
        assert!(
            err.message.contains("was replaced"),
            "expected replaced error, got: {}",
            err.message
        );

        // A file whose identity now matches a current sheet output is refused too.
        let current = dir.join("CURRENT.pdf");
        fs::write(&current, b"current").unwrap();
        let current_id = file_id_of_path(&current);
        let err =
            still_a_leftover(&set, LeftoverKind::Pdf, &current, "CURRENT", current_id).unwrap_err();
        assert!(
            err.message.contains("current output"),
            "expected current output error, got: {}",
            err.message
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
