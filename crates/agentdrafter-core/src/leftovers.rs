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
use std::io::{self, ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Condvar, Mutex, OnceLock};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

use serde::Serialize;

#[cfg(any(unix, windows))]
use crate::build::file_id_of_file;
use crate::build::{file_id_of_path, is_same_file, FileId};
use crate::description::RowsFile;
use crate::dwg;
use crate::error::{Error, ErrorCode};
#[cfg(not(unix))]
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

/// One leftover candidate recorded during [`find`] with its exact OS path,
/// file name, and file identity so [`clean_up`] never rebuilds paths from
/// lossy display text and refuses to move a file replaced in the meantime
/// (#238, #239).
struct Candidate {
    leftover: Leftover,
    path: PathBuf,
    name: String,
    id: Option<FileId>,
}

/// Looks for the leftover outputs of a set in its output folder, and moves
/// them into [`SUPERSEDED_FOLDER`] when `clean` asks.
pub(crate) fn leftovers(set: &SetOutputs, clean: bool) -> Scan {
    match find(set) {
        Ok((output_dir, found)) => Scan {
            leftovers: Some(if clean {
                clean_up(set, &output_dir, found)
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
fn find(set: &SetOutputs) -> Result<(DirHandle, Vec<Candidate>), Error> {
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
    let output_dir = DirHandle::open_output(set.output).map_err(not_listed)?;
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
            path: path.clone(),
            name: name.clone(),
            id: None,
        };
        let (mut file, id) = match open_plain_file_no_follow(&output_dir, &name) {
            Ok(OpenCandidate::Plain { file, id }) => (file, id),
            Ok(OpenCandidate::NotPlain) => continue,
            Err(e) => {
                found.push((name.clone(), unreadable(e)));
                continue;
            }
        };
        // A row writes it under a name that compares otherwise, such as in
        // other letter case where the file system ignores case.
        if written.writes(&path, &sheet, id) {
            continue;
        }
        match is_sets_file(set, kind, &path, &mut file) {
            Ok(true) => found.push((
                name.clone(),
                Candidate {
                    leftover: Leftover {
                        file: path.display().to_string(),
                        kind,
                        sheet: sheet.to_owned(),
                        unreadable: false,
                        moved_to: None,
                        error: None,
                    },
                    path,
                    name,
                    id,
                },
            )),
            Ok(false) => {}
            Err(e) => found.push((name.clone(), unreadable(e))),
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((
        output_dir,
        found.into_iter().map(|(_, candidate)| candidate).collect(),
    ))
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
    fn writes(&self, path: &Path, sheet: &str, id: Option<FileId>) -> bool {
        is_current_output(self.output, self.sheets, path, sheet, id)
    }
}

/// Live check whether `path` (named for `sheet`) is currently one of the
/// set's row outputs (#238). Never relies on a stale snapshot of `FileId`s.
fn is_current_output(
    output: &Path,
    sheets: &[&str],
    path: &Path,
    sheet: &str,
    known_id: Option<FileId>,
) -> bool {
    if let Some(id) = known_id.or_else(|| file_id_of_path(path)) {
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
#[cfg(not(unix))]
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

/// Whether the already-opened file `file` at `path` of the output folder,
/// a `kind`, is one AgentDrafter made for the set. An error when it cannot
/// be read.
fn is_sets_file(
    set: &SetOutputs,
    kind: LeftoverKind,
    path: &Path,
    file: &mut fs::File,
) -> io::Result<bool> {
    match kind {
        LeftoverKind::Dwg | LeftoverKind::Dxf => is_sets_dwg_file(set, path, file),
        LeftoverKind::Pdf => is_sets_pdf_file(set, path, file),
    }
}

/// Whether the DWG at `path`, already opened as `file`, is one the set built:
/// its build mark names the set as the set's own build names it from there.
/// A DWG with no mark, an original among them, or whose mark names no set or
/// another set, is not. An error when the file cannot be read to tell, as a
/// DWG cut short cannot ([`dwg::read_build_mark_from_file`]).
fn is_sets_dwg_file(set: &SetOutputs, path: &Path, file: &mut fs::File) -> io::Result<bool> {
    let Some(marked) = set.marked else {
        return Ok(false);
    };
    let mark = dwg::read_build_mark_from_file(file, path).map_err(io::Error::other)?;
    Ok(mark.is_some_and(|mark| mark.set.as_deref() == Some(marked)))
}

/// Whether the PDF at `path`, already opened as `file`, is one plotted from
/// the set: its stamp, which AgentDrafter alone writes, names a rows file of
/// the set. That is one of the set's rows files now, or a file in the set's
/// folder, gone or not, that no other set inside it holds, so that a sheet
/// whose whole rows file was taken out of the settings is still known. An
/// error when the file cannot be read.
fn is_sets_pdf_file(set: &SetOutputs, path: &Path, file: &mut fs::File) -> io::Result<bool> {
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
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
fn clean_up(set: &SetOutputs, output_dir: &DirHandle, found: Vec<Candidate>) -> Vec<Leftover> {
    clean_up_with(
        found,
        output_dir,
        &set.output.join(SUPERSEDED_FOLDER),
        |kind, path, name, sheet, expected_id| {
            still_a_leftover(set, output_dir, kind, path, name, sheet, expected_id)
        },
        supersede,
    )
}

/// Outcome of moving one leftover into `superseded/`: its final file name
/// inside `superseded/`, its display path, and its `FileId` after placement.
struct MovedEntry {
    dest_name: String,
    dest_path: PathBuf,
    id: Option<FileId>,
}

fn clean_up_with<Check, Move>(
    mut found: Vec<Candidate>,
    output_dir: &DirHandle,
    folder: &Path,
    mut check: Check,
    mut mover: Move,
) -> Vec<Leftover>
where
    Check: FnMut(LeftoverKind, &Path, &str, &str, Option<FileId>) -> Result<(), Error>,
    Move: FnMut(&DirHandle, &DirHandle, &str, &Path, Option<FileId>) -> Result<MovedEntry, Error>,
{
    if found.iter().all(|c| c.leftover.unreadable) {
        return found.into_iter().map(|c| c.leftover).collect();
    }
    let ready = DirHandle::open_or_create_superseded(output_dir, folder);
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
        let superseded_dir = match &ready {
            Ok(dir) => dir,
            Err(err) => {
                for idx in indices {
                    found[idx].leftover.error = Some(err.clone());
                }
                continue;
            }
        };
        // Check every movable output of this sheet before moving any of them,
        // so a changed, replaced, current, or missing sibling leaves the whole
        // sheet untouched (#236, #238).
        let mut first_failed: Option<String> = None;
        for &idx in &indices {
            if let Err(err) = check(
                found[idx].leftover.kind,
                &found[idx].path,
                &found[idx].name,
                &found[idx].leftover.sheet,
                found[idx].id,
            ) {
                if first_failed.is_none() {
                    first_failed = Some(found[idx].name.clone());
                }
                found[idx].leftover.error = Some(err);
            }
        }
        if let Some(failed_name) = first_failed {
            for &idx in &indices {
                if found[idx].leftover.error.is_none() {
                    let path = found[idx].path.clone();
                    found[idx].leftover.error =
                        Some(held_together_error(&path, &sheet, &failed_name));
                }
            }
            continue;
        }
        // Move each output of the sheet; if any fails, roll back earlier moves
        // for this sheet through the open handles so its DWG/DXF and PDF never
        // end up split (#236, #239).
        let mut moved: Vec<(usize, MovedEntry)> = Vec::new();
        let mut failed: Option<(usize, String)> = None;
        for (pos, &idx) in indices.iter().enumerate() {
            let name = found[idx].name.clone();
            let path = found[idx].path.clone();
            let id = found[idx].id;
            match mover(output_dir, superseded_dir, &name, &path, id) {
                Ok(entry) => {
                    found[idx].leftover.moved_to = Some(entry.dest_path.display().to_string());
                    moved.push((idx, entry));
                }
                Err(err) => {
                    found[idx].leftover.error = Some(err);
                    failed = Some((pos, name));
                    break;
                }
            }
        }
        if let Some((failed_pos, failed_name)) = failed {
            for (prev_idx, entry) in moved.into_iter().rev() {
                let orig_path = found[prev_idx].path.clone();
                let orig_name = found[prev_idx].name.clone();
                // Refuse to move back an archive entry that another process
                // replaced since we placed it (#239).
                let still_ours = match entry.id {
                    Some(expected) => {
                        plain_file_id_in_dir(superseded_dir, &entry.dest_name) == Some(expected)
                    }
                    None => true,
                };
                let rollback_result = if !still_ours {
                    Err(io::Error::other(format!(
                        "`{}` in `{SUPERSEDED_FOLDER}` was replaced before rollback",
                        entry.dest_name
                    )))
                } else {
                    move_entry_no_replace(
                        superseded_dir,
                        &entry.dest_name,
                        output_dir,
                        &orig_name,
                        entry.id,
                    )
                    .map(|_| ())
                };
                match rollback_result {
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
                let later_path = found[later_idx].path.clone();
                found[later_idx].leftover.error =
                    Some(held_together_error(&later_path, &sheet, &failed_name));
            }
        }
    }
    found.into_iter().map(|c| c.leftover).collect()
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

/// Refuses to move the file named `name` inside `output_dir` unless it is
/// still the same plain file examined during [`find`], is not a current output
/// of the set, and still carries the set's build mark or stamp, inspecting it
/// through a single non-following handle (#238, #239).
fn still_a_leftover(
    set: &SetOutputs,
    output_dir: &DirHandle,
    kind: LeftoverKind,
    path: &Path,
    name: &str,
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
    let (mut file, id) = match open_plain_file_no_follow(output_dir, name) {
        Ok(OpenCandidate::Plain { file, id }) => (file, id),
        Ok(OpenCandidate::NotPlain) => {
            return Err(changed("is no longer a plain file".to_owned()));
        }
        Err(e) => return Err(changed(format!("could not be read again ({e})"))),
    };
    if let Some(expected) = expected_id {
        if id != Some(expected) {
            return Err(changed("was replaced".to_owned()));
        }
    }
    if is_current_output(set.output, &set.sheets, path, sheet, id) {
        return Err(changed("became a current output of the set".to_owned()));
    }
    match is_sets_file(set, kind, path, &mut file) {
        Ok(true) => Ok(()),
        Ok(false) => Err(changed(
            "no longer carries the set's build mark or stamp".to_owned(),
        )),
        Err(e) => Err(changed(format!("could not be read again ({e})"))),
    }
}

/// Moves the leftover `name` inside `output_dir` into `superseded_dir` through
/// their open directory handles, keeping its name, and never over a file
/// there: when a file of that name is there, from an earlier clean, or appears
/// while the move is made, it is kept and the leftover takes the first free
/// name `<sheet> (2).<ext>`, `<sheet> (3).<ext>` and so on. Returns where
/// it went and its file identity.
fn supersede(
    output_dir: &DirHandle,
    superseded_dir: &DirHandle,
    name: &str,
    path: &Path,
    expected_id: Option<FileId>,
) -> Result<MovedEntry, Error> {
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
        match move_entry_no_replace(output_dir, name, superseded_dir, &candidate, expected_id) {
            Ok(id) => {
                let dest_path = superseded_dir.path.join(&candidate);
                return Ok(MovedEntry {
                    dest_name: candidate,
                    dest_path,
                    id,
                });
            }
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

/// Result of opening a directory entry once without following links.
enum OpenCandidate {
    Plain { file: fs::File, id: Option<FileId> },
    NotPlain,
}

fn handle_file_id(file: &fs::File) -> Option<FileId> {
    #[cfg(any(unix, windows))]
    {
        file_id_of_file(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        None
    }
}

/// An open directory handle used so cleanup works against the verified real
/// `output/` and `superseded/` directories rather than re-resolving paths
/// that another process could swap for a link or junction mid-run (#239).
struct DirHandle {
    path: PathBuf,
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
    #[cfg(windows)]
    _pin: Option<std::os::windows::io::OwnedHandle>,
}

impl DirHandle {
    fn open_output(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let fd = unix_io::open_dir_at(unix_io::AT_FDCWD, path.as_os_str(), false)?;
            Ok(DirHandle {
                path: path.to_path_buf(),
                fd,
            })
        }
        #[cfg(windows)]
        {
            Ok(DirHandle {
                path: path.to_path_buf(),
                _pin: None,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(DirHandle {
                path: path.to_path_buf(),
            })
        }
    }

    /// Makes `folder`, the output folder's [`SUPERSEDED_FOLDER`], when it is
    /// not there, refuses it when it is a link or a junction or anything but a
    /// folder, and keeps an open handle to it for the rest of the run (#239).
    fn open_or_create_superseded(output: &DirHandle, folder: &Path) -> Result<Self, Error> {
        let refused = |why: &str| {
            Error::new(
                ErrorCode::WriteFailed,
                format!(
                    "`{SUPERSEDED_FOLDER}` in the output folder {why}, so no leftover was moved"
                ),
            )
            .in_file(folder)
        };
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let child = folder
                .file_name()
                .ok_or_else(|| refused("has no folder name"))?;
            if let Err(e) = unix_io::mkdir_at(output.fd.as_raw_fd(), child) {
                if e.kind() != ErrorKind::AlreadyExists {
                    return Err(refused(&format!("could not be made: {e}")));
                }
            }
            match unix_io::open_dir_at(output.fd.as_raw_fd(), child, true) {
                Ok(fd) => Ok(DirHandle {
                    path: folder.to_path_buf(),
                    fd,
                }),
                Err(e) if unix_io::is_eloop(&e) => Err(refused(
                    "is a link or a junction, which would move the leftovers to another folder",
                )),
                Err(e) if unix_io::is_enotdir(&e) => {
                    if fs::symlink_metadata(folder).is_ok_and(|m| is_link(&m)) {
                        Err(refused(
                            "is a link or a junction, which would move the leftovers to another folder",
                        ))
                    } else {
                        Err(refused("is not a folder"))
                    }
                }
                Err(e) => Err(refused(&format!("could not be read: {e}"))),
            }
        }
        #[cfg(windows)]
        {
            let _ = output;
            if fs::symlink_metadata(folder).is_err() {
                fs::create_dir(folder).or_else(|e| match e.kind() {
                    ErrorKind::AlreadyExists => Ok(()),
                    _ => Err(refused(&format!("could not be made: {e}"))),
                })?;
            }
            let handle = win_io::open_superseded_dir_pinned(folder)
                .map_err(|e| refused(&format!("could not be read: {e}")))?;
            let tag = win_io::handle_tag_info(&handle)
                .map_err(|e| refused(&format!("could not be read: {e}")))?;
            if win_io::is_link_tag(&tag) {
                return Err(refused(
                    "is a link or a junction, which would move the leftovers to another folder",
                ));
            }
            if (tag.file_attributes & win_io::FILE_ATTRIBUTE_DIRECTORY) == 0 {
                return Err(refused("is not a folder"));
            }
            Ok(DirHandle {
                path: folder.to_path_buf(),
                _pin: Some(handle),
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = output;
            if fs::symlink_metadata(folder).is_err() {
                fs::create_dir(folder).or_else(|e| match e.kind() {
                    ErrorKind::AlreadyExists => Ok(()),
                    _ => Err(refused(&format!("could not be made: {e}"))),
                })?;
            }
            let metadata = fs::symlink_metadata(folder)
                .map_err(|e| refused(&format!("could not be read: {e}")))?;
            if is_link(&metadata) {
                return Err(refused(
                    "is a link or a junction, which would move the leftovers to another folder",
                ));
            }
            if !metadata.is_dir() {
                return Err(refused("is not a folder"));
            }
            Ok(DirHandle {
                path: folder.to_path_buf(),
            })
        }
    }
}

/// Opens `name` inside `dir` once without following links, refusing folders,
/// links, FIFOs and other non-plain entries (#239).
fn open_plain_file_no_follow(dir: &DirHandle, name: &str) -> io::Result<OpenCandidate> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        match unix_io::open_read_no_follow_at(dir.fd.as_raw_fd(), name) {
            Ok(file) => {
                let meta = file.metadata()?;
                if !meta.file_type().is_file() {
                    return Ok(OpenCandidate::NotPlain);
                }
                let id = handle_file_id(&file);
                Ok(OpenCandidate::Plain { file, id })
            }
            Err(e) if unix_io::is_eloop(&e) || unix_io::is_eisdir(&e) => {
                Ok(OpenCandidate::NotPlain)
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        let path = dir.path.join(name);
        let meta = fs::symlink_metadata(&path)?;
        if !is_plain_file(&meta) {
            return Ok(OpenCandidate::NotPlain);
        }
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(crate::build::win_file::FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path);
        let file = match opened {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                if fs::symlink_metadata(&path).is_ok_and(|m| !is_plain_file(&m)) {
                    return Ok(OpenCandidate::NotPlain);
                }
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        let tag = win_io::raw_handle_tag_info(file.as_raw_handle())?;
        if win_io::is_link_tag(&tag)
            || (tag.file_attributes & win_io::FILE_ATTRIBUTE_DIRECTORY) != 0
        {
            return Ok(OpenCandidate::NotPlain);
        }
        // A non-link reparse point (such as a OneDrive cloud placeholder)
        // must be opened without `FILE_FLAG_OPEN_REPARSE_POINT` to read its
        // bytes; re-verify it is still a plain non-link file after opening.
        let file = if (tag.file_attributes & win_io::FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            let reopened = fs::File::open(&path)?;
            let retag = win_io::raw_handle_tag_info(reopened.as_raw_handle())?;
            if win_io::is_link_tag(&retag)
                || (retag.file_attributes & win_io::FILE_ATTRIBUTE_DIRECTORY) != 0
            {
                return Ok(OpenCandidate::NotPlain);
            }
            reopened
        } else {
            file
        };
        let id = handle_file_id(&file);
        Ok(OpenCandidate::Plain { file, id })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let path = dir.path.join(name);
        let meta = fs::symlink_metadata(&path)?;
        if !is_plain_file(&meta) {
            return Ok(OpenCandidate::NotPlain);
        }
        let file = fs::File::open(&path)?;
        Ok(OpenCandidate::Plain { file, id: None })
    }
}

/// The `FileId` of `name` inside `dir` without following links, or `None` if
/// missing, unreadable, or not a plain file.
fn plain_file_id_in_dir(dir: &DirHandle, name: &str) -> Option<FileId> {
    match open_plain_file_no_follow(dir, name) {
        Ok(OpenCandidate::Plain { id, .. }) => id,
        _ => None,
    }
}

/// Moves `src_name` in `src_dir` to `dst_name` in `dst_dir` through their open
/// directory handles, failing with [`ErrorKind::AlreadyExists`] when anything
/// is at `dst_name`, and never replacing it, even should it appear while the
/// move is made (#238, #239). Returns the `FileId` of the placed file.
fn move_entry_no_replace(
    src_dir: &DirHandle,
    src_name: &str,
    dst_dir: &DirHandle,
    dst_name: &str,
    expected_src_id: Option<FileId>,
) -> io::Result<Option<FileId>> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let src_fd = src_dir.fd.as_raw_fd();
        let dst_fd = dst_dir.fd.as_raw_fd();
        match unix_io::rename_no_replace_at(src_fd, src_name, dst_fd, dst_name) {
            Ok(()) => Ok(expected_src_id),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(e),
            Err(e) if is_cross_device_or_unsupported(&e) => {
                // On volumes without `RENAME_NOREPLACE`/`RENAME_EXCL`, try a
                // handle-relative `linkat` before falling back to a temp copy.
                let src_id = expected_src_id.or_else(|| plain_file_id_in_dir(src_dir, src_name));
                match unix_io::link_at(src_fd, src_name, dst_fd, dst_name) {
                    Ok(()) => {
                        if src_id.is_some() && plain_file_id_in_dir(src_dir, src_name) != src_id {
                            return Err(rollback_placed_entry(
                                dst_dir,
                                dst_name,
                                src_id,
                                io::Error::other("the leftover was replaced while moving it"),
                            ));
                        }
                        if let Err(rm_err) = unix_io::unlink_at(src_fd, src_name) {
                            return Err(rollback_placed_entry(dst_dir, dst_name, src_id, rm_err));
                        }
                        Ok(src_id)
                    }
                    Err(le) if le.kind() == ErrorKind::AlreadyExists => Err(le),
                    Err(le) if is_cross_device_or_unsupported(&le) || is_hard_link_refused(&le) => {
                        move_via_copy_fallback(
                            src_dir,
                            src_name,
                            dst_dir,
                            dst_name,
                            expected_src_id,
                        )
                    }
                    Err(le) => Err(le),
                }
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(not(unix))]
    {
        let from = src_dir.path.join(src_name);
        let to = dst_dir.path.join(dst_name);
        match rename_no_replace_keep_src(&from, &to) {
            Ok(()) => Ok(expected_src_id),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(e),
            Err(e) if is_cross_device_or_unsupported(&e) => {
                move_via_copy_fallback(src_dir, src_name, dst_dir, dst_name, expected_src_id)
            }
            Err(e) => Err(e),
        }
    }
}

/// Cross-device / no-hard-link fallback: copies `src_name` into a temporary
/// file inside `dst_dir` (preserving permissions, group on Unix, and DACL on
/// Windows), syncs and places it at `dst_name` without replacing anything,
/// verifies `src_name` was not replaced, and unlinks `src_name` — rolling back
/// `dst_name` only if it still matches the file we placed and never discarding
/// rollback errors (#239).
fn move_via_copy_fallback(
    src_dir: &DirHandle,
    src_name: &str,
    dst_dir: &DirHandle,
    dst_name: &str,
    expected_src_id: Option<FileId>,
) -> io::Result<Option<FileId>> {
    let (src_id, copied_id) =
        copy_new_via_temp(src_dir, src_name, dst_dir, dst_name, expected_src_id)?;
    if src_id.is_some() && plain_file_id_in_dir(src_dir, src_name) != src_id {
        return Err(rollback_placed_entry(
            dst_dir,
            dst_name,
            copied_id,
            io::Error::other("the leftover was replaced while copying it"),
        ));
    }
    if let Err(rm_err) = unlink_in_dir_maybe_readonly(src_dir, src_name, src_id) {
        return Err(rollback_placed_entry(dst_dir, dst_name, copied_id, rm_err));
    }
    Ok(copied_id)
}

/// Rolls back `dst_name` in `dst_dir` after a move failed, ONLY if `dst_name`
/// still has `placed_id` (refusing to delete a replacement another process put
/// there), and always surfaces any rollback error alongside `cause` (#239).
fn rollback_placed_entry(
    dst_dir: &DirHandle,
    dst_name: &str,
    placed_id: Option<FileId>,
    cause: io::Error,
) -> io::Error {
    if let Some(expected) = placed_id {
        if plain_file_id_in_dir(dst_dir, dst_name) != Some(expected) {
            return io::Error::new(
                cause.kind(),
                format!(
                    "{cause}; left `{dst_name}` in `{SUPERSEDED_FOLDER}` untouched because it was \
                     replaced before rollback"
                ),
            );
        }
    }
    match unlink_in_dir_maybe_readonly(dst_dir, dst_name, placed_id) {
        Ok(()) => cause,
        Err(rb_err) => io::Error::new(
            cause.kind(),
            format!(
                "{cause}; rollback could not remove `{dst_name}` from `{SUPERSEDED_FOLDER}`: \
                 {rb_err}"
            ),
        ),
    }
}

fn unlink_in_dir_maybe_readonly(
    dir: &DirHandle,
    name: &str,
    expected_id: Option<FileId>,
) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let _ = expected_id;
        unix_io::unlink_at(dir.fd.as_raw_fd(), name)
    }
    #[cfg(windows)]
    {
        let path = dir.path.join(name);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                // On Windows a read-only file cannot be unlinked until its
                // read-only attribute is cleared (#239). Re-check identity
                // before changing attributes, and restore read-only if the
                // removal still fails (e.g. file held open).
                if let Some(expected) = expected_id {
                    if plain_file_id_in_dir(dir, name) != Some(expected) {
                        return Err(io::Error::other(
                            "the file was replaced before it could be removed",
                        ));
                    }
                }
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    return Err(e);
                };
                if !is_plain_file(&meta) || !meta.permissions().readonly() {
                    return Err(e);
                }
                let mut writable = meta.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                writable.set_readonly(false);
                fs::set_permissions(&path, writable)?;
                match fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(rm_err) => {
                        let mut ro = meta.permissions();
                        ro.set_readonly(true);
                        let _ = fs::set_permissions(&path, ro);
                        Err(rm_err)
                    }
                }
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = expected_id;
        fs::remove_file(dir.path.join(name))
    }
}

fn is_cross_device_or_unsupported(err: &io::Error) -> bool {
    if matches!(
        err.kind(),
        ErrorKind::CrossesDevices | ErrorKind::Unsupported | ErrorKind::InvalidInput
    ) {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        // EXDEV = 18, EINVAL = 22, ENOSYS = 38, EOPNOTSUPP = 95.
        matches!(err.raw_os_error(), Some(18 | 22 | 38 | 95))
    }
    #[cfg(target_os = "macos")]
    {
        // EXDEV = 18, EINVAL = 22, ENOTSUP = 45, ENOSYS = 78.
        matches!(err.raw_os_error(), Some(18 | 22 | 45 | 78))
    }
    #[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
    {
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

#[cfg(unix)]
fn is_hard_link_refused(err: &io::Error) -> bool {
    // VFAT/exFAT and some network mounts refuse hard links with EPERM (1) or
    // EMLINK (31).
    err.kind() == ErrorKind::PermissionDenied || matches!(err.raw_os_error(), Some(1 | 31))
}

fn next_temp_name() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!(".agentdrafter-move-{}-{seq}.tmp", std::process::id())
}

/// Copies `src_name` from `src_dir` into a temporary file inside `dst_dir`,
/// preserves permissions (mode + group on Unix, DACL + read-only on Windows —
/// refusing the copy if they cannot be preserved), syncs it, and places it at
/// `dst_name` without replacing anything already there (#239). Returns
/// `(src_id, copied_id)`.
fn copy_new_via_temp(
    src_dir: &DirHandle,
    src_name: &str,
    dst_dir: &DirHandle,
    dst_name: &str,
    expected_src_id: Option<FileId>,
) -> io::Result<(Option<FileId>, Option<FileId>)> {
    let (mut source, src_id) = match open_plain_file_no_follow(src_dir, src_name)? {
        OpenCandidate::Plain { file, id } => (file, id),
        OpenCandidate::NotPlain => {
            return Err(io::Error::other("the leftover is no longer a plain file"));
        }
    };
    if let Some(expected) = expected_src_id {
        if src_id != Some(expected) {
            return Err(io::Error::other("the leftover was replaced"));
        }
    }

    let (tmp_name, mut target) = create_temp_in_dir(dst_dir)?;
    let cleanup_tmp = |err: io::Error| -> io::Error {
        let rm = unlink_in_dir_maybe_readonly(dst_dir, &tmp_name, None);
        if err.kind() == ErrorKind::AlreadyExists {
            return err;
        }
        match rm {
            Ok(()) => err,
            Err(rm_err) if rm_err.kind() == ErrorKind::NotFound => err,
            Err(rm_err) => io::Error::new(
                err.kind(),
                format!("{err}; could not remove temporary file `{tmp_name}`: {rm_err}"),
            ),
        }
    };

    let filled = fill_preserving_security(&mut source, &mut target);
    let copied_id = handle_file_id(&target);
    drop(target);
    if let Err(e) = filled {
        return Err(cleanup_tmp(e));
    }
    if let Err(e) = place_temp_in_dir_no_replace(dst_dir, &tmp_name, dst_name) {
        return Err(cleanup_tmp(e));
    }
    Ok((src_id, copied_id))
}

fn create_temp_in_dir(dir: &DirHandle) -> io::Result<(String, fs::File)> {
    for _ in 0..64 {
        let tmp_name = next_temp_name();
        #[cfg(unix)]
        let opened = {
            use std::os::fd::AsRawFd;
            unix_io::create_new_write_at(dir.fd.as_raw_fd(), &tmp_name, 0o600)
        };
        #[cfg(windows)]
        let opened = {
            use std::os::windows::fs::OpenOptionsExt;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .access_mode(
                    win_io::GENERIC_WRITE | win_io::WRITE_DAC | win_io::FILE_READ_ATTRIBUTES,
                )
                .open(dir.path.join(&tmp_name))
        };
        #[cfg(not(any(unix, windows)))]
        let opened = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.path.join(&tmp_name));
        match opened {
            Ok(file) => return Ok((tmp_name, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        "could not allocate a temporary file in `superseded`",
    ))
}

fn place_temp_in_dir_no_replace(dir: &DirHandle, tmp_name: &str, dst_name: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = dir.fd.as_raw_fd();
        match unix_io::rename_no_replace_at(fd, tmp_name, fd, dst_name) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(e),
            Err(e) if is_cross_device_or_unsupported(&e) => {
                unix_io::link_at(fd, tmp_name, fd, dst_name)?;
                unix_io::unlink_at(fd, tmp_name)
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(not(unix))]
    {
        rename_no_replace_keep_src(&dir.path.join(tmp_name), &dir.path.join(dst_name))
    }
}

/// Writes the bytes of `source` into `target`, preserving mode + group on Unix
/// and DACL + read-only on Windows before any byte is written (and setting the
/// Windows read-only attribute once synced) (#239).
fn fill_preserving_security(source: &mut fs::File, target: &mut fs::File) -> io::Result<()> {
    let meta = source.metadata()?;
    let permissions = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let src_gid = meta.gid();
        let src_mode = permissions.mode() & 0o7777;
        unix_io::preserve_group(target, src_gid)?;
        target.set_permissions(fs::Permissions::from_mode(src_mode))?;
        let after = target.metadata()?;
        if after.gid() != src_gid || (after.mode() & 0o7777) != src_mode {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "could not preserve the leftover's group and mode on the copy",
            ));
        }
    }
    #[cfg(windows)]
    {
        win_io::copy_dacl(source, target)?;
    }
    io::copy(source, target)?;
    target.sync_all()?;
    #[cfg(not(unix))]
    if permissions.readonly() {
        target.set_permissions(permissions)?;
    }
    Ok(())
}

#[cfg(unix)]
mod unix_io {
    use std::ffi::{c_char, c_int, CString, OsStr};
    use std::fs::File;
    use std::io;
    use std::os::fd::{FromRawFd, OwnedFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::io::AsRawFd;

    #[cfg(target_os = "macos")]
    type ModeT = u16;
    #[cfg(not(target_os = "macos"))]
    type ModeT = u32;

    pub(super) const AT_FDCWD: c_int = {
        #[cfg(target_os = "macos")]
        {
            -2
        }
        #[cfg(not(target_os = "macos"))]
        {
            -100
        }
    };

    const O_RDONLY: c_int = 0;
    const O_WRONLY: c_int = 1;
    const O_CREAT: c_int = {
        #[cfg(target_os = "macos")]
        {
            0x0200
        }
        #[cfg(not(target_os = "macos"))]
        {
            0o100
        }
    };
    const O_EXCL: c_int = {
        #[cfg(target_os = "macos")]
        {
            0x0800
        }
        #[cfg(not(target_os = "macos"))]
        {
            0o200
        }
    };
    const O_NONBLOCK: c_int = {
        #[cfg(target_os = "macos")]
        {
            0x0004
        }
        #[cfg(not(target_os = "macos"))]
        {
            0o4000
        }
    };
    const O_CLOEXEC: c_int = {
        #[cfg(target_os = "macos")]
        {
            0x0100_0000
        }
        #[cfg(not(target_os = "macos"))]
        {
            0o2000000
        }
    };
    const O_DIRECTORY: c_int = {
        #[cfg(target_os = "macos")]
        {
            0x0010_0000
        }
        #[cfg(all(
            any(target_os = "linux", target_os = "android"),
            any(target_arch = "aarch64", target_arch = "arm")
        ))]
        {
            0o40000
        }
        #[cfg(all(
            not(target_os = "macos"),
            not(all(
                any(target_os = "linux", target_os = "android"),
                any(target_arch = "aarch64", target_arch = "arm")
            ))
        ))]
        {
            0o200000
        }
    };
    const O_NOFOLLOW: c_int = {
        #[cfg(target_os = "macos")]
        {
            0x0100
        }
        #[cfg(all(
            any(target_os = "linux", target_os = "android"),
            any(target_arch = "aarch64", target_arch = "arm")
        ))]
        {
            0o100000
        }
        #[cfg(all(
            not(target_os = "macos"),
            not(all(
                any(target_os = "linux", target_os = "android"),
                any(target_arch = "aarch64", target_arch = "arm")
            ))
        ))]
        {
            0o400000
        }
    };

    extern "C" {
        fn openat(dirfd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
        fn mkdirat(dirfd: c_int, path: *const c_char, mode: ModeT) -> c_int;
        fn unlinkat(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int;
        fn linkat(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_int,
        ) -> c_int;
        fn fchown(fd: c_int, owner: u32, group: u32) -> c_int;
        #[cfg(target_os = "linux")]
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: std::ffi::c_uint,
        ) -> c_int;
        #[cfg(target_os = "macos")]
        fn renameatx_np(
            fromfd: c_int,
            from: *const c_char,
            tofd: c_int,
            to: *const c_char,
            flags: std::ffi::c_uint,
        ) -> c_int;
    }

    fn c_path(path: &OsStr) -> io::Result<CString> {
        CString::new(path.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a nul byte"))
    }

    pub(super) fn is_eloop(err: &io::Error) -> bool {
        #[cfg(target_os = "macos")]
        {
            err.raw_os_error() == Some(62)
        }
        #[cfg(not(target_os = "macos"))]
        {
            err.raw_os_error() == Some(40)
        }
    }

    pub(super) fn is_enotdir(err: &io::Error) -> bool {
        err.kind() == io::ErrorKind::NotADirectory || err.raw_os_error() == Some(20)
    }

    pub(super) fn is_eisdir(err: &io::Error) -> bool {
        err.kind() == io::ErrorKind::IsADirectory || err.raw_os_error() == Some(21)
    }

    pub(super) fn open_dir_at(dirfd: RawFd, path: &OsStr, no_follow: bool) -> io::Result<OwnedFd> {
        let cstr = c_path(path)?;
        let mut flags = O_RDONLY | O_DIRECTORY | O_CLOEXEC;
        if no_follow {
            flags |= O_NOFOLLOW;
        }
        // SAFETY: `cstr` is NUL-terminated; `O_CREAT` is not set so no mode
        // variadic argument is read.
        let fd = unsafe { openat(dirfd, cstr.as_ptr(), flags) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: `openat` returned a brand-new owned file descriptor.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    pub(super) fn mkdir_at(dirfd: RawFd, name: &OsStr) -> io::Result<()> {
        let cstr = c_path(name)?;
        // SAFETY: `cstr` is a valid NUL-terminated C string.
        let rc = unsafe { mkdirat(dirfd, cstr.as_ptr(), 0o777 as ModeT) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(super) fn open_read_no_follow_at(dirfd: RawFd, name: &str) -> io::Result<File> {
        let cstr = c_path(OsStr::new(name))?;
        let flags = O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK;
        // SAFETY: `cstr` is NUL-terminated; `O_CREAT` is not set.
        let fd = unsafe { openat(dirfd, cstr.as_ptr(), flags) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: `openat` returned a brand-new owned file descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    pub(super) fn create_new_write_at(dirfd: RawFd, name: &str, mode: u32) -> io::Result<File> {
        let cstr = c_path(OsStr::new(name))?;
        let flags = O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC;
        // SAFETY: `cstr` is NUL-terminated; `mode` is promoted to `c_uint`
        // matching C variadic promotion for `mode_t`.
        let fd = unsafe { openat(dirfd, cstr.as_ptr(), flags, mode as std::ffi::c_uint) };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: `openat` returned a brand-new owned file descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }

    pub(super) fn unlink_at(dirfd: RawFd, name: &str) -> io::Result<()> {
        let cstr = c_path(OsStr::new(name))?;
        // SAFETY: `cstr` is a valid NUL-terminated C string.
        let rc = unsafe { unlinkat(dirfd, cstr.as_ptr(), 0) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(super) fn link_at(
        olddirfd: RawFd,
        oldname: &str,
        newdirfd: RawFd,
        newname: &str,
    ) -> io::Result<()> {
        let src = c_path(OsStr::new(oldname))?;
        let dst = c_path(OsStr::new(newname))?;
        // SAFETY: `src` and `dst` are valid NUL-terminated C strings.
        let rc = unsafe { linkat(olddirfd, src.as_ptr(), newdirfd, dst.as_ptr(), 0) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(super) fn rename_no_replace_at(
        olddirfd: RawFd,
        oldname: &str,
        newdirfd: RawFd,
        newname: &str,
    ) -> io::Result<()> {
        let src = c_path(OsStr::new(oldname))?;
        let dst = c_path(OsStr::new(newname))?;
        #[cfg(target_os = "linux")]
        {
            const RENAME_NOREPLACE: std::ffi::c_uint = 1;
            // SAFETY: `src` and `dst` are valid NUL-terminated C strings.
            let rc = unsafe {
                renameat2(
                    olddirfd,
                    src.as_ptr(),
                    newdirfd,
                    dst.as_ptr(),
                    RENAME_NOREPLACE,
                )
            };
            if rc == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        #[cfg(target_os = "macos")]
        {
            const RENAME_EXCL: std::ffi::c_uint = 0x0000_0004;
            // SAFETY: `src` and `dst` are valid NUL-terminated C strings.
            let rc = unsafe {
                renameatx_np(olddirfd, src.as_ptr(), newdirfd, dst.as_ptr(), RENAME_EXCL)
            };
            if rc == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (olddirfd, src, newdirfd, dst);
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
    }

    pub(super) fn preserve_group(target: &File, gid: u32) -> io::Result<()> {
        if target.metadata()?.gid() == gid {
            return Ok(());
        }
        // SAFETY: `target.as_raw_fd()` is a valid open file descriptor;
        // `u32::MAX` (`(uid_t)-1`) keeps the file's owner unchanged.
        let rc = unsafe { fchown(target.as_raw_fd(), u32::MAX, gid) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(windows)]
mod win_io {
    use std::ffi::c_void;
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;
    use std::ptr::{null, null_mut};

    pub(super) const GENERIC_WRITE: u32 = 0x4000_0000;
    pub(super) const WRITE_DAC: u32 = 0x0004_0000;
    pub(super) const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    pub(super) const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    pub(super) const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    const FILE_LIST_DIRECTORY: u32 = 0x0000_0001;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_TAG_INFO_CLASS: u32 = 9;
    const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

    const SE_FILE_OBJECT: u32 = 1;
    const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
    const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(super) struct FileAttributeTagInfo {
        pub file_attributes: u32,
        pub reparse_tag: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *const c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: *mut c_void,
        ) -> *mut c_void;
        fn GetFileInformationByHandleEx(
            file: *mut c_void,
            file_information_class: u32,
            file_information: *mut c_void,
            buffer_size: u32,
        ) -> i32;
        fn LocalFree(hmem: *mut c_void) -> *mut c_void;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn GetSecurityInfo(
            handle: *mut c_void,
            object_type: u32,
            security_info: u32,
            ppsid_owner: *mut *mut c_void,
            ppsid_group: *mut *mut c_void,
            pp_dacl: *mut *mut c_void,
            pp_sacl: *mut *mut c_void,
            pp_security_descriptor: *mut *mut c_void,
        ) -> u32;
        fn SetSecurityInfo(
            handle: *mut c_void,
            object_type: u32,
            security_info: u32,
            psid_owner: *mut c_void,
            psid_group: *mut c_void,
            p_dacl: *mut c_void,
            p_sacl: *mut c_void,
        ) -> u32;
    }

    struct LocalMem(*mut c_void);

    impl Drop for LocalMem {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` was allocated by `GetSecurityInfo` via
                // LocalAlloc and is freed once here.
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }

    /// Opens `folder` without following reparse points and without granting
    /// `FILE_SHARE_DELETE`, requesting `FILE_LIST_DIRECTORY` so NTFS enforces
    /// share-mode checks and prevents another process from renaming or
    /// replacing `superseded/` with a junction while the handle is held.
    pub(super) fn open_superseded_dir_pinned(folder: &Path) -> io::Result<OwnedHandle> {
        let wide: Vec<u16> = folder
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` is a valid NUL-terminated UTF-16 path.
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if raw == (-1isize as *mut c_void) {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: `CreateFileW` succeeded and returned an owned handle.
            Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
        }
    }

    pub(super) fn handle_tag_info(handle: &OwnedHandle) -> io::Result<FileAttributeTagInfo> {
        raw_handle_tag_info(handle.as_raw_handle())
    }

    pub(super) fn raw_handle_tag_info(handle: *mut c_void) -> io::Result<FileAttributeTagInfo> {
        let mut info = FileAttributeTagInfo::default();
        // SAFETY: `handle` is a valid open handle and `info` is an initialized
        // `FILE_ATTRIBUTE_TAG_INFO` struct of matching byte size.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FILE_ATTRIBUTE_TAG_INFO_CLASS,
                (&mut info as *mut FileAttributeTagInfo).cast::<c_void>(),
                std::mem::size_of::<FileAttributeTagInfo>() as u32,
            )
        };
        if ok != 0 {
            Ok(info)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(super) fn is_link_tag(tag: &FileAttributeTagInfo) -> bool {
        (tag.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0
            && matches!(
                tag.reparse_tag,
                IO_REPARSE_TAG_SYMLINK | IO_REPARSE_TAG_MOUNT_POINT
            )
    }

    /// Copies the source file's DACL onto `target` with
    /// `PROTECTED_DACL_SECURITY_INFORMATION` so the copy keeps the source's
    /// ACL rather than inheriting `superseded/`'s (#239).
    pub(super) fn copy_dacl(source: &File, target: &File) -> io::Result<()> {
        let mut dacl: *mut c_void = null_mut();
        let mut sd: *mut c_void = null_mut();
        // SAFETY: `source.as_raw_handle()` is open with READ_CONTROL (part of
        // GENERIC_READ); `sd` is released by `LocalMem::drop`.
        let get_err = unsafe {
            GetSecurityInfo(
                source.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut sd,
            )
        };
        if get_err != 0 {
            return Err(io::Error::from_raw_os_error(get_err as i32));
        }
        let _sd_guard = LocalMem(sd);
        // SAFETY: `target.as_raw_handle()` is open with WRITE_DAC and `dacl`
        // points inside `sd` which remains alive for the duration of the call.
        let set_err = unsafe {
            SetSecurityInfo(
                target.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                dacl,
                null_mut(),
            )
        };
        if set_err != 0 {
            return Err(io::Error::from_raw_os_error(set_err as i32));
        }
        Ok(())
    }
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

    /// A copy keeps the source's mode and group exactly, neither wider, as a
    /// file made under the usual umask of 022 would be, nor narrower, as one
    /// made under a umask that takes away a group's writing would be (#239).
    #[cfg(unix)]
    #[test]
    fn a_copy_keeps_the_source_s_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-copy-mode-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let handle = DirHandle::open_output(&dir).unwrap();
        for mode in [0o640, 0o660, 0o600, 0o444] {
            let from_name = format!("{mode:o}.dwg");
            let to_name = format!("{mode:o}-copy.dwg");
            let (from, to) = (dir.join(&from_name), dir.join(&to_name));
            fs::write(&from, b"leftover").unwrap();
            fs::set_permissions(&from, fs::Permissions::from_mode(mode)).unwrap();
            let src_gid = fs::metadata(&from).unwrap().gid();
            copy_new_via_temp(&handle, &from_name, &handle, &to_name, None).unwrap();
            let copied_meta = fs::metadata(&to).unwrap();
            let copied = copied_meta.permissions().mode() & 0o7777;
            assert_eq!(copied, mode, "{mode:o} came out {copied:o}");
            assert_eq!(copied_meta.gid(), src_gid);
            assert_eq!(fs::read(&to).unwrap(), b"leftover");
        }
        // No temporary files left behind.
        assert!(fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .all(|n| !n.ends_with(".tmp")));
        let _ = fs::remove_dir_all(&dir);
    }

    /// On Unix, preserving a group the caller cannot `fchown` to refuses the
    /// copy rather than silently writing under another group (#239).
    #[cfg(unix)]
    #[test]
    fn preserve_group_refuses_an_unassignable_group_when_unprivileged() {
        extern "C" {
            fn geteuid() -> u32;
        }
        if unsafe { geteuid() } == 0 {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-copy-gid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = fs::File::create(dir.join("target.dwg")).unwrap();
        // A GID the unprivileged caller does not belong to cannot be assigned
        // via `fchown`.
        const UNASSIGNABLE_GID: u32 = 999_999;
        use std::os::unix::fs::MetadataExt;
        if file.metadata().unwrap().gid() != UNASSIGNABLE_GID {
            assert!(unix_io::preserve_group(&file, UNASSIGNABLE_GID).is_err());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// On Windows a read-only source makes a read-only copy, keeps the
    /// source's DACL rather than inheriting `superseded/`'s, and leaves no
    /// temporary file behind (#239).
    #[cfg(windows)]
    #[test]
    fn a_copy_of_a_read_only_file_is_read_only_and_keeps_dacl() {
        let dir = std::env::temp_dir().join(format!(
            "agentdrafter-copy-read-only-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sup = dir.join(SUPERSEDED_FOLDER);
        let src_dir = DirHandle::open_output(&dir).unwrap();
        let dst_dir = DirHandle::open_or_create_superseded(&src_dir, &sup).unwrap();

        let from = dir.join("from.dwg");
        let to = sup.join("to.dwg");
        fs::write(&from, b"leftover").unwrap();
        let mut read_only = fs::metadata(&from).unwrap().permissions();
        read_only.set_readonly(true);
        fs::set_permissions(&from, read_only).unwrap();

        let (_, copied_id) =
            copy_new_via_temp(&src_dir, "from.dwg", &dst_dir, "to.dwg", None).unwrap();
        assert!(copied_id.is_some());
        assert!(fs::metadata(&to).unwrap().permissions().readonly());
        assert_eq!(fs::read(&to).unwrap(), b"leftover");
        assert!(fs::read_dir(&sup)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .all(|n| !n.ends_with(".tmp")));

        // Verify the copied file has SE_DACL_PROTECTED set so it does not
        // inherit `superseded/`'s ACL.
        #[link(name = "advapi32")]
        extern "system" {
            fn GetSecurityInfo(
                handle: *mut std::ffi::c_void,
                object_type: u32,
                security_info: u32,
                ppsid_owner: *mut *mut std::ffi::c_void,
                ppsid_group: *mut *mut std::ffi::c_void,
                pp_dacl: *mut *mut std::ffi::c_void,
                pp_sacl: *mut *mut std::ffi::c_void,
                pp_security_descriptor: *mut *mut std::ffi::c_void,
            ) -> u32;
            fn GetSecurityDescriptorControl(
                sd: *mut std::ffi::c_void,
                control: *mut u16,
                revision: *mut u32,
            ) -> i32;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn LocalFree(hmem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        }
        use std::os::windows::io::AsRawHandle;
        let copied_file = fs::File::open(&to).unwrap();
        let mut sd = std::ptr::null_mut();
        let rc = unsafe {
            GetSecurityInfo(
                copied_file.as_raw_handle(),
                1,
                0x0000_0004,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut sd,
            )
        };
        assert_eq!(rc, 0);
        let mut control = 0u16;
        let mut rev = 0u32;
        assert_ne!(
            unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut rev) },
            0
        );
        unsafe {
            LocalFree(sd);
        }
        const SE_DACL_PROTECTED: u16 = 0x1000;
        assert_ne!(
            control & SE_DACL_PROTECTED,
            0,
            "copy should carry a protected copy of the source's DACL"
        );
        drop(copied_file);

        // Rolling back a read-only copy removes it cleanly rather than leaving
        // a duplicate read-only file behind in `superseded/`.
        let rb = rollback_placed_entry(&dst_dir, "to.dwg", copied_id, io::Error::other("boom"));
        assert_eq!(rb.to_string(), "boom");
        assert!(!to.exists(), "read-only copy should be removed on rollback");

        // Clean up read-only source.
        unlink_in_dir_maybe_readonly(&src_dir, "from.dwg", None).unwrap();
        drop(dst_dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A move never replaces what is at the new name: it fails, and both
    /// files stay as they were; and a copy made new via temp refuses the same
    /// without leaving any temp file behind.
    #[test]
    fn a_move_never_replaces_a_file() {
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-no-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let handle = DirHandle::open_output(&dir).unwrap();
        let (from, to) = (dir.join("from.dwg"), dir.join("to.dwg"));
        fs::write(&from, b"leftover").unwrap();
        fs::write(&to, b"kept").unwrap();
        let refused =
            move_entry_no_replace(&handle, "from.dwg", &handle, "to.dwg", None).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::AlreadyExists);
        assert_eq!(
            copy_new_via_temp(&handle, "from.dwg", &handle, "to.dwg", None)
                .unwrap_err()
                .kind(),
            ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&from).unwrap(), b"leftover");
        assert_eq!(fs::read(&to).unwrap(), b"kept");
        assert!(fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .all(|n| !n.ends_with(".tmp")));

        move_entry_no_replace(&handle, "from.dwg", &handle, "free.dwg", None).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(dir.join("free.dwg")).unwrap(), b"leftover");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Rollback refuses to delete an entry in `superseded/` that another
    /// process replaced, and surfaces the reason instead of discarding it (#239).
    #[test]
    fn rollback_refuses_to_remove_a_replaced_archive_entry() {
        let dir = std::env::temp_dir().join(format!(
            "agentdrafter-rollback-replaced-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sup = dir.join(SUPERSEDED_FOLDER);
        let out_dir = DirHandle::open_output(&dir).unwrap();
        let sup_dir = DirHandle::open_or_create_superseded(&out_dir, &sup).unwrap();

        let placed = sup.join("DEMO-003.dwg");
        fs::write(&placed, b"original-copy").unwrap();
        let placed_id = plain_file_id_in_dir(&sup_dir, "DEMO-003.dwg");

        // Another process atomically replaces DEMO-003.dwg in superseded/.
        let replacement = sup.join("DEMO-003.dwg.new");
        fs::write(&replacement, b"someone-elses-file").unwrap();
        fs::rename(&replacement, &placed).unwrap();

        let err = rollback_placed_entry(
            &sup_dir,
            "DEMO-003.dwg",
            placed_id,
            io::Error::other("unlink source failed"),
        );
        let msg = err.to_string();
        assert!(msg.contains("unlink source failed"), "{msg}");
        assert!(msg.contains("replaced before rollback"), "{msg}");
        assert_eq!(fs::read(&placed).unwrap(), b"someone-elses-file");

        drop(sup_dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// On Linux (where paths may hold arbitrary non-UTF-8 bytes; macOS APFS
    /// rejects them at the filesystem layer), a non-UTF-8 output folder name
    /// keeps its exact OS bytes throughout cleaning rather than rebuilding from
    /// display text and touching a U+FFFD sibling folder (#239).
    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_output_folder_moves_real_leftover_not_replacement_sibling() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let base =
            std::env::temp_dir().join(format!("agentdrafter-non-utf8-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let real_dir = base.join(OsString::from_vec(b"out-\xff".to_vec()));
        let decoy_dir = base.join("out-\u{FFFD}");
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&decoy_dir).unwrap();

        let real_file = real_dir.join("DEMO-003.dwg");
        let decoy_file = decoy_dir.join("DEMO-003.dwg");
        fs::write(&real_file, b"real-leftover").unwrap();
        fs::write(&decoy_file, b"decoy-file").unwrap();

        let out_handle = DirHandle::open_output(&real_dir).unwrap();
        let sup = real_dir.join(SUPERSEDED_FOLDER);
        let candidate = Candidate {
            leftover: Leftover {
                file: real_file.display().to_string(),
                kind: LeftoverKind::Dwg,
                sheet: "DEMO-003".to_owned(),
                unreadable: false,
                moved_to: None,
                error: None,
            },
            path: real_file.clone(),
            name: "DEMO-003.dwg".to_owned(),
            id: plain_file_id_in_dir(&out_handle, "DEMO-003.dwg"),
        };
        let out = clean_up_with(
            vec![candidate],
            &out_handle,
            &sup,
            |_, _, _, _, _| Ok(()),
            supersede,
        );
        assert!(
            out[0].moved_to.is_some() && out[0].error.is_none(),
            "{:#?}",
            out[0]
        );
        assert!(!real_file.exists());
        assert_eq!(
            fs::read(sup.join("DEMO-003.dwg")).unwrap(),
            b"real-leftover"
        );
        assert_eq!(fs::read(&decoy_file).unwrap(), b"decoy-file");
        assert!(!decoy_dir.join(SUPERSEDED_FOLDER).exists());
        let _ = fs::remove_dir_all(&base);
    }

    /// On Unix, swapping `superseded/` for a symlink after it is opened does
    /// not redirect moves made through the open directory handle (#239).
    #[cfg(unix)]
    #[test]
    fn swapping_superseded_to_symlink_mid_run_does_not_redirect_moves() {
        let base = std::env::temp_dir().join(format!(
            "agentdrafter-superseded-swap-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let out_dir = base.join("dwg");
        let elsewhere = base.join("elsewhere");
        fs::create_dir_all(&out_dir).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(out_dir.join("DEMO-003.dwg"), b"leftover").unwrap();

        let out_handle = DirHandle::open_output(&out_dir).unwrap();
        let sup = out_dir.join(SUPERSEDED_FOLDER);
        let sup_handle = DirHandle::open_or_create_superseded(&out_handle, &sup).unwrap();

        // Mid-run swap: move real `superseded/` aside and put a symlink in its place.
        let real_aside = base.join("real-superseded");
        fs::rename(&sup, &real_aside).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &sup).unwrap();

        move_entry_no_replace(
            &out_handle,
            "DEMO-003.dwg",
            &sup_handle,
            "DEMO-003.dwg",
            None,
        )
        .unwrap();
        assert_eq!(
            fs::read(real_aside.join("DEMO-003.dwg")).unwrap(),
            b"leftover"
        );
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&base);
    }

    /// On Windows, the open `superseded/` directory handle withholds
    /// `FILE_SHARE_DELETE`, preventing another process from renaming or
    /// replacing `superseded/` with a junction mid-run (#239).
    #[cfg(windows)]
    #[test]
    fn open_superseded_dir_blocks_replacing_it_with_a_junction_mid_run() {
        let base = std::env::temp_dir().join(format!(
            "agentdrafter-superseded-pin-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let out_dir = base.join("dwg");
        fs::create_dir_all(&out_dir).unwrap();
        let out_handle = DirHandle::open_output(&out_dir).unwrap();
        let sup = out_dir.join(SUPERSEDED_FOLDER);
        let sup_handle = DirHandle::open_or_create_superseded(&out_handle, &sup).unwrap();

        assert!(
            fs::rename(&sup, base.join("moved-aside")).is_err(),
            "pinned superseded/ must not be renameable while held"
        );
        assert!(
            fs::remove_dir(&sup).is_err(),
            "pinned superseded/ must not be removable while held"
        );
        // Moving a leftover file into the pinned folder still works normally.
        fs::write(out_dir.join("DEMO-003.dwg"), b"leftover").unwrap();
        move_entry_no_replace(
            &out_handle,
            "DEMO-003.dwg",
            &sup_handle,
            "DEMO-003.dwg",
            None,
        )
        .unwrap();
        assert_eq!(fs::read(sup.join("DEMO-003.dwg")).unwrap(), b"leftover");

        drop(sup_handle);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_failed_sibling_move_rolls_back_the_sheets_first_file_without_blocking_other_sheets() {
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-pair-rollback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sup = dir.join(SUPERSEDED_FOLDER);
        let out_handle = DirHandle::open_output(&dir).unwrap();

        let dwg2 = dir.join("DEMO-002.dwg");
        let pdf2 = dir.join("DEMO-002.pdf");
        let dwg3 = dir.join("DEMO-003.dwg");
        let pdf3 = dir.join("DEMO-003.pdf");
        fs::write(&dwg2, b"dwg-2").unwrap();
        fs::write(&pdf2, b"pdf-2").unwrap();
        fs::write(&dwg3, b"dwg-3").unwrap();
        fs::write(&pdf3, b"pdf-3").unwrap();

        let item = |path: &Path, name: &str, kind: LeftoverKind, sheet: &str| Candidate {
            leftover: Leftover {
                file: path.display().to_string(),
                kind,
                sheet: sheet.to_owned(),
                unreadable: false,
                moved_to: None,
                error: None,
            },
            path: path.to_path_buf(),
            name: name.to_owned(),
            id: plain_file_id_in_dir(&out_handle, name),
        };
        let found = vec![
            item(&dwg2, "DEMO-002.dwg", LeftoverKind::Dwg, "DEMO-002"),
            item(&pdf2, "DEMO-002.pdf", LeftoverKind::Pdf, "DEMO-002"),
            item(&dwg3, "DEMO-003.dwg", LeftoverKind::Dwg, "DEMO-003"),
            item(&pdf3, "DEMO-003.pdf", LeftoverKind::Pdf, "DEMO-003"),
        ];

        // Moving DEMO-002.pdf fails after DEMO-002.dwg was already moved:
        // DEMO-002.dwg must be rolled back so DEMO-002 is not split, while
        // DEMO-003 still moves cleanly.
        let out = clean_up_with(
            found,
            &out_handle,
            &sup,
            |_, _, _, _, _| Ok(()),
            |out_dir, sup_dir, name, path, id| {
                if name == "DEMO-002.pdf" {
                    return Err(
                        Error::new(ErrorCode::WriteFailed, "file is held open").in_file(path)
                    );
                }
                supersede(out_dir, sup_dir, name, path, id)
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
                item(&dwg2, "DEMO-002.dwg", LeftoverKind::Dwg, "DEMO-002"),
                item(&pdf2, "DEMO-002.pdf", LeftoverKind::Pdf, "DEMO-002"),
            ],
            &out_handle,
            &sup,
            |kind, path, _, _, _| {
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
        let out_handle = DirHandle::open_output(&dir).unwrap();
        let file = dir.join("OLD.dwg");
        fs::write(&file, b"old").unwrap();
        let old_id = plain_file_id_in_dir(&out_handle, "OLD.dwg");

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
        let err = still_a_leftover(
            &set,
            &out_handle,
            LeftoverKind::Dwg,
            &file,
            "OLD.dwg",
            "OLD",
            old_id,
        )
        .unwrap_err();
        assert!(
            err.message.contains("was replaced"),
            "expected replaced error, got: {}",
            err.message
        );

        // A file whose identity now matches a current sheet output is refused too.
        let current = dir.join("CURRENT.pdf");
        fs::write(&current, b"current").unwrap();
        let current_id = plain_file_id_in_dir(&out_handle, "CURRENT.pdf");
        let err = still_a_leftover(
            &set,
            &out_handle,
            LeftoverKind::Pdf,
            &current,
            "CURRENT.pdf",
            "CURRENT",
            current_id,
        )
        .unwrap_err();
        assert!(
            err.message.contains("current output"),
            "expected current output error, got: {}",
            err.message
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
