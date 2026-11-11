//! Puts a change set's files in place together: the records it edited, the
//! DWGs of the sheets it landed on, and the set's inconsistent mark, written
//! or taken away, with the undo state it saves and its entry added to the end
//! of the set's journal. An undo puts its files back through the same step.
//!
//! Every new file is first written whole beside its place. Then, one file
//! at a time, the file already there is moved aside and the new one moved
//! into its place. If any step fails, every file already moved is put back
//! as it was and every new file taken away, so on disk too a change set
//! lands whole or not at all: no sheet is left with new records beside its
//! old DWG, and no record two sheets share changes for one of them only
//! (ADR-0005), and the mark never lags the sheets: a change set that lands
//! on some sheets and fails on others either lands with its mark or not at
//! all. The journal's entry is added last, once every file is in place, and
//! a journal that cannot take it is cut back to its length before, and every
//! file put back, so a change set lands only with its entry. The files moved
//! aside are taken away once all are in place.

use std::cmp::Reverse;
use std::fs::{self, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::build::{beside, write_beside};
use crate::error::{Error, ErrorCode};

/// One file a change set puts in place, or takes away.
pub(super) struct Replacement {
    pub path: PathBuf,
    /// The file's new bytes, or `None` to take the file away.
    pub bytes: Option<Vec<u8>>,
    /// What the file is, for an error, such as "the DWG".
    pub what: &'static str,
}

/// Text added to the end of a file, such as a journal's entry, in the same
/// step as the replacements.
pub(super) struct Append {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    /// What the file is, for an error, such as "the journal".
    pub what: &'static str,
}

/// Where one replacement got to: the file that was there, moved aside, and
/// whether the new one is in place.
struct Placed {
    aside: Option<PathBuf>,
}

/// Puts every file of `replacements` in place and adds `append` to the end
/// of its file, or does none of it. On failure the error says so, and every
/// file is as it was before.
pub(super) fn commit(replacements: &[Replacement], append: Option<&Append>) -> Result<(), Error> {
    let mut made: Vec<PathBuf> = Vec::new();
    let mut staged: Vec<Option<PathBuf>> = Vec::new();
    let staging = stage(replacements, &mut made, &mut staged).and_then(|()| match append {
        Some(append) => make_folder(&append.path, append.what, &mut made),
        None => Ok(()),
    });
    if let Err(e) = staging {
        clean(&staged, &made);
        return Err(Error {
            message: format!("{}; nothing was changed", e.message),
            ..e
        });
    }
    let mut placed: Vec<Placed> = Vec::with_capacity(replacements.len());
    for (i, (replacement, temporary)) in replacements.iter().zip(&staged).enumerate() {
        match place(i, &replacement.path, temporary.as_deref()) {
            Ok(done) => placed.push(done),
            Err(e) => {
                let not_back = roll_back(&replacements[..placed.len()], &placed);
                clean(&staged[i..], &made);
                let message = if not_back.is_empty() {
                    format!(
                        "could not put {} in place ({e}); every file the change set had put in \
                         place was put back as it was, so nothing was changed",
                        replacement.what
                    )
                } else {
                    format!(
                        "could not put {} in place ({e}), and could not put back {}: build the \
                         set again with --overwrite to bring its DWGs in step with its records",
                        replacement.what,
                        not_back.join(", ")
                    )
                };
                return Err(Error::new(ErrorCode::WriteFailed, message).in_file(&replacement.path));
            }
        }
    }
    if let Some(append) = append {
        if let Err(e) = add_to_end(replacements.len(), append) {
            let not_back = roll_back(replacements, &placed);
            clean(&[], &made);
            let message = if not_back.is_empty() {
                format!(
                    "could not add to {} ({e}); every file put in place was put back as it was, \
                     so nothing was changed",
                    append.what
                )
            } else {
                format!(
                    "could not add to {} ({e}), and could not put back {}: build the set again \
                     with --overwrite to bring its DWGs in step with its records",
                    append.what,
                    not_back.join(", ")
                )
            };
            return Err(Error::new(ErrorCode::WriteFailed, message).in_file(&append.path));
        }
    }
    for (replacement, done) in replacements.iter().zip(&placed) {
        if let Some(aside) = &done.aside {
            let _ = fs::remove_file(aside);
        }
        // A folder a file taken away leaves empty goes too.
        if replacement.bytes.is_none() {
            if let Some(parent) = replacement.path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    }
    Ok(())
}

/// Makes the folders the new files go in, and writes each beside its place.
fn stage(
    replacements: &[Replacement],
    made: &mut Vec<PathBuf>,
    staged: &mut Vec<Option<PathBuf>>,
) -> Result<(), Error> {
    for replacement in replacements {
        let path = &replacement.path;
        let Some(bytes) = &replacement.bytes else {
            staged.push(None);
            continue;
        };
        make_folder(path, replacement.what, made)?;
        staged.push(Some(write_beside(path, bytes, replacement.what)?));
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

/// Adds `append`'s bytes to the end of its file, making the file if it is
/// not there. When they cannot all be written, the file is cut back to its
/// length before, or taken away if it was not there.
fn add_to_end(index: usize, append: &Append) -> io::Result<()> {
    fail_here(index)?;
    let existed = fs::symlink_metadata(&append.path).is_ok();
    // Opened to write, not only to append, so that Windows lets it be cut
    // back.
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&append.path)?;
    let length = file.seek(SeekFrom::End(0))?;
    let written = file.write_all(&append.bytes).and_then(|()| file.sync_all());
    if let Err(e) = written {
        let _ = file.set_len(length);
        drop(file);
        if !existed {
            let _ = fs::remove_file(&append.path);
        }
        return Err(e);
    }
    Ok(())
}

/// Moves the file at `path`, if there is one, aside, and the new one from
/// `temporary`, if there is one, into its place. When the new one cannot be
/// moved, the old one is put back before the error is given.
fn place(index: usize, path: &Path, temporary: Option<&Path>) -> io::Result<Placed> {
    fail_here(index)?;
    let aside = if fs::symlink_metadata(path).is_ok() {
        let aside = beside(path, "old");
        fs::rename(path, &aside)?;
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

#[cfg(test)]
thread_local! {
    /// The replacement a test asks to fail, by its place in the commit.
    pub(super) static FAIL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Fails replacement `index`, or the append after the last, once, when a
/// test asks for it, so a test can show a failed commit puts every file
/// back.
fn fail_here(index: usize) -> io::Result<()> {
    #[cfg(test)]
    if FAIL_AT.with(|f| f.get() == Some(index) && f.replace(None).is_some()) {
        return Err(io::Error::other("a failure the test asked for"));
    }
    let _ = index;
    Ok(())
}
