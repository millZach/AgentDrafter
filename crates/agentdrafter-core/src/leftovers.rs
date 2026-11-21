//! Leftover outputs: the DWGs and PDFs in a drawing set's output folder
//! that AgentDrafter made for the set and that no row writes any more,
//! since their sheet was renamed, merged or taken out of the rows.
//!
//! Every build and plot of a set lists them under `leftovers`, so that an
//! old sheet does not look current and get issued by mistake. Zach decided
//! on 2026-11-20 (#109) that they are only listed unless cleaning is asked
//! for, by `clean` for one run or by `clean: true` in the set's settings,
//! and that cleaning moves them into the folder [`SUPERSEDED_FOLDER`] in
//! the output folder, keeping their names, and never deletes them.
//!
//! Only a file AgentDrafter made for this very set is ever a leftover: a
//! DWG whose build mark names the set, as the set's own build writes it
//! from its output folder, and a PDF whose stamp names one of the set's
//! rows files. Any other file, an original among them, is never listed and
//! never moved (ADR-0004).

use std::fs;
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
/// set and that no row writes any more.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Leftover {
    /// The file, in the set's output folder.
    pub file: String,
    /// What it is: `dwg` or `pdf`.
    pub kind: LeftoverKind,
    /// The sheet it was made for: its name without `.dwg` or `.pdf`, which
    /// no row of the set names any more.
    pub sheet: String,
    /// Where cleaning moved it, in the output folder's
    /// [`SUPERSEDED_FOLDER`]. None when it was only listed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
    /// Why cleaning could not move it, when it could not: it is still where
    /// it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
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

/// The leftover outputs of a set in its output folder, in the byte order
/// of their names, moved into [`SUPERSEDED_FOLDER`] when `clean` asks.
/// A folder that cannot be listed has none.
pub(crate) fn leftovers(set: &SetOutputs, clean: bool) -> Vec<Leftover> {
    let written: Vec<String> = set.sheets.iter().map(|s| s.to_lowercase()).collect();
    let Ok(entries) = fs::read_dir(set.output) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf, LeftoverKind, String)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let path = entry.path();
            // A link is no file AgentDrafter made, whatever it leads to.
            if !fs::symlink_metadata(&path).ok()?.is_file() {
                return None;
            }
            let (sheet, kind) = output_kind(&name)?;
            // A row writes it: its name is a sheet's, in any case, as a
            // set's sheet names are told apart.
            if written.contains(&sheet.to_lowercase()) {
                return None;
            }
            let ours = match kind {
                LeftoverKind::Dwg => is_sets_dwg(set, &path),
                LeftoverKind::Pdf => is_sets_pdf(set, &path),
            };
            ours.then(|| (name.clone(), path, kind, sheet.to_owned()))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
        .into_iter()
        .map(|(name, path, kind, sheet)| {
            let (moved_to, error) = if clean {
                match supersede(set.output, &name, &path) {
                    Ok(to) => (Some(to.display().to_string()), None),
                    Err(error) => (None, Some(error)),
                }
            } else {
                (None, None)
            };
            Leftover {
                file: path.display().to_string(),
                kind,
                sheet,
                moved_to,
                error,
            }
        })
        .collect()
}

/// The error a set's reply carries when cleaning could not move some of
/// its leftovers, if it could not.
pub(crate) fn not_moved(leftovers: &[Leftover]) -> Option<Error> {
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

/// Whether the DWG at `path`, in the set's output folder, is one the set
/// built: its build mark names the set as the set's own build names it
/// from there. A DWG with no mark, an original among them, or whose mark
/// names no set or another set, is not.
fn is_sets_dwg(set: &SetOutputs, path: &Path) -> bool {
    let Some(marked) = set.marked else {
        return false;
    };
    dwg::build_mark(path).is_some_and(|mark| mark.set.as_deref() == Some(marked))
}

/// Whether the PDF at `path`, in the set's output folder, is one plotted
/// from the set: its stamp, which AgentDrafter alone writes, names a rows
/// file of the set. That is one of the set's rows files now, or a file in
/// the set's folder, gone or not, that no other set inside it holds, so
/// that a sheet whose whole rows file was taken out of the settings is
/// still known.
fn is_sets_pdf(set: &SetOutputs, path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    if !bytes.starts_with(b"%PDF-") {
        return false;
    }
    let Ok(stamp) = stamp::stamp_of(&bytes) else {
        return false;
    };
    stamp
        .sources
        .iter()
        .filter(|source| source.kind == SourceKind::Rows)
        .any(|source| {
            let rows = stamp::resolve(path, &source.path);
            set.rows_files.iter().any(|f| is_same_file(&f.path, &rows))
                || set_holding(&rows).is_some_and(|folder| is_same_file(&folder, set.folder))
        })
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

/// Moves the leftover `name` at `path` into the output folder's
/// [`SUPERSEDED_FOLDER`], keeping its name. When a file of that name is
/// there already, from an earlier clean, it is kept and the leftover takes
/// the first free name `<sheet> (2).<ext>`, `<sheet> (3).<ext>` and so on,
/// so nothing is ever replaced. Returns where it went.
fn supersede(output: &Path, name: &str, path: &Path) -> Result<PathBuf, Error> {
    let folder = output.join(SUPERSEDED_FOLDER);
    fs::create_dir_all(&folder).map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!(
                "could not make the folder `{SUPERSEDED_FOLDER}` to move the leftover into: {e}"
            ),
        )
        .in_file(&folder)
    })?;
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    let to = std::iter::once(name.to_owned())
        .chain((2..=MOST_NUMBERED).map(|n| format!("{stem} ({n}).{extension}")))
        .map(|candidate| folder.join(candidate))
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
        .ok_or_else(|| {
            Error::new(
                ErrorCode::WriteFailed,
                format!(
                    "`{SUPERSEDED_FOLDER}` already holds `{name}` and every numbered name for it \
                     up to ({MOST_NUMBERED})"
                ),
            )
            .in_file(path)
        })?;
    fs::rename(path, &to).map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not move the leftover into `{SUPERSEDED_FOLDER}`: {e}"),
        )
        .in_file(path)
    })?;
    Ok(to)
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
}
