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

use serde::Serialize;

use crate::build::is_same_file;
use crate::description::RowsFile;
use crate::dwg;
use crate::error::{Error, ErrorCode};
use crate::set::SET_SETTINGS_FILE;
use crate::stamp::{self, SourceKind};

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

/// Looks for the leftover outputs of a set in its output folder, and moves
/// them into [`SUPERSEDED_FOLDER`] when `clean` asks.
pub(crate) fn leftovers(set: &SetOutputs, clean: bool) -> Scan {
    match find(set) {
        Ok(found) => Scan {
            leftovers: Some(if clean { clean_up(set, found) } else { found }),
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
fn find(set: &SetOutputs) -> Result<Vec<Leftover>, Error> {
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
    let mut written = Written::new(set);
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
        let unreadable = |e: io::Error| Leftover {
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
        match is_sets(set, kind, &path) {
            Ok(true) => found.push((
                name,
                Leftover {
                    file: path.display().to_string(),
                    kind,
                    sheet: sheet.to_owned(),
                    unreadable: false,
                    moved_to: None,
                    error: None,
                },
            )),
            Ok(false) => {}
            Err(e) => found.push((name, unreadable(e))),
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found.into_iter().map(|(_, leftover)| leftover).collect())
}

/// What the set's rows write in its output folder: each output's file
/// name, and, worked out only when a file of the folder is not named so,
/// which files they are.
struct Written<'a> {
    output: &'a Path,
    sheets: &'a [&'a str],
    /// `<sheet>.dwg` and `<sheet>.pdf` for each row.
    names: HashSet<String>,
    /// The files at the rows' output paths that are there, by what the
    /// file system tells them by; worked out the first time it is needed.
    files: Option<Vec<FileId>>,
}

impl<'a> Written<'a> {
    fn new(set: &'a SetOutputs<'a>) -> Self {
        let names = set
            .sheets
            .iter()
            .flat_map(|sheet| [format!("{sheet}.dwg"), format!("{sheet}.pdf")])
            .collect();
        Written {
            output: set.output,
            sheets: &set.sheets,
            names,
            files: None,
        }
    }

    /// Whether the file at `path`, named for `sheet`, is one a row writes:
    /// the very file at a row's output path, as the file system tells it.
    /// Where that cannot be told, by its name in any letter case, the safe
    /// way round: the file is then taken to be the row's, and left alone.
    fn writes(&mut self, path: &Path, sheet: &str) -> bool {
        let output = self.output;
        let sheets = self.sheets;
        let files = self.files.get_or_insert_with(|| {
            sheets
                .iter()
                .flat_map(|sheet| ["dwg", "pdf"].map(|e| output.join(format!("{sheet}.{e}"))))
                .filter_map(|path| file_id(&path))
                .collect()
        });
        match file_id(path) {
            Some(id) => files.contains(&id),
            None => {
                let folded = sheet.to_lowercase();
                sheets.iter().any(|s| s.to_lowercase() == folded)
            }
        }
    }
}

/// What the file system tells a file by: its device and number on Unix,
/// and elsewhere its full path as the system resolves it, in the letter
/// case the folder holds it in.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FileId {
    #[cfg(unix)]
    Node(u64, u64),
    #[cfg(not(unix))]
    Resolved(PathBuf),
}

/// What the file system tells the file at `path` by, if it is there and
/// can be told.
fn file_id(path: &Path) -> Option<FileId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).ok()?;
        Some(FileId::Node(metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        fs::canonicalize(path).ok().map(FileId::Resolved)
    }
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
/// is: a DWG or a PDF, by its extension in any case. None for any other
/// file.
fn output_kind(name: &str) -> Option<(&str, LeftoverKind)> {
    let (sheet, extension) = name.rsplit_once('.')?;
    let kind = if extension.eq_ignore_ascii_case("dwg") {
        LeftoverKind::Dwg
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
        LeftoverKind::Dwg => is_sets_dwg(set, path),
        LeftoverKind::Pdf => is_sets_pdf(set, path),
    }
}

/// Whether the DWG at `path`, in the set's output folder, is one the set
/// built: its build mark names the set as the set's own build names it
/// from there. A DWG with no mark, an original among them, or whose mark
/// names no set or another set, is not. An error when the file cannot be
/// opened to read its mark.
fn is_sets_dwg(set: &SetOutputs, path: &Path) -> io::Result<bool> {
    fs::File::open(path)?;
    let Some(marked) = set.marked else {
        return Ok(false);
    };
    Ok(dwg::build_mark(path).is_some_and(|mark| mark.set.as_deref() == Some(marked)))
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
/// made only when there is one to move. When that folder is a link or a
/// junction, or cannot be made, nothing is moved and each leftover carries
/// the error.
fn clean_up(set: &SetOutputs, found: Vec<Leftover>) -> Vec<Leftover> {
    if found.iter().all(|leftover| leftover.unreadable) {
        return found;
    }
    let folder = set.output.join(SUPERSEDED_FOLDER);
    let ready = superseded_folder(&folder);
    found
        .into_iter()
        .map(|mut leftover| {
            if leftover.unreadable {
                return leftover;
            }
            let path = PathBuf::from(&leftover.file);
            let moved = ready.clone().and_then(|()| {
                still_a_leftover(set, leftover.kind, &path)?;
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                supersede(&folder, &name, &path)
            });
            match moved {
                Ok(to) => leftover.moved_to = Some(to.display().to_string()),
                Err(error) => leftover.error = Some(error),
            }
            leftover
        })
        .collect()
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

/// Refuses to move the file at `path` unless it is still a plain file the
/// set made, as it was when it was found.
fn still_a_leftover(set: &SetOutputs, kind: LeftoverKind, path: &Path) -> Result<(), Error> {
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
/// replacing it, even should it appear while the move is made: the file is
/// given its new name as a hard link, which the system refuses to make
/// over another, then its old name is taken away. Where the file system
/// has no hard links, its bytes are copied to a file made new at `to`,
/// which the system refuses as well, and the old one is taken away once
/// the copy is whole. Should the old name not go, the new one is taken
/// away again, and the file stays where it was.
fn move_without_replacing(from: &Path, to: &Path) -> io::Result<()> {
    match fs::hard_link(from, to) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Err(e),
        Err(_) => copy_new(from, to)?,
    }
    fs::remove_file(from).inspect_err(|_| {
        let _ = fs::remove_file(to);
    })
}

/// Copies the file at `from` to a file made new at `to`, failing with
/// [`ErrorKind::AlreadyExists`] when anything is at `to`. A copy that does
/// not finish is taken away.
fn copy_new(from: &Path, to: &Path) -> io::Result<()> {
    let mut source = fs::File::open(from)?;
    let mut target = OpenOptions::new().write(true).create_new(true).open(to)?;
    io::copy(&mut source, &mut target)
        .and_then(|_| target.sync_all())
        .inspect_err(|_| {
            let _ = fs::remove_file(to);
        })
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
}
