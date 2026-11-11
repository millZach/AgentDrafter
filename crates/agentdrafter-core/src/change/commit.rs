//! Puts a change set's files in place together: the records it edited and
//! the DWGs of the sheets it landed on.
//!
//! Every new file is first written whole beside its place. Then, one file
//! at a time, the file already there is moved aside and the new one moved
//! into its place. If any step fails, every file already moved is put back
//! as it was and every new file taken away, so on disk too a change set
//! lands whole or not at all: no sheet is left with new records beside its
//! old DWG, and no record two sheets share changes for one of them only
//! (ADR-0005). The files moved aside are taken away once all are in place.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::build::{beside, write_beside};
use crate::error::{Error, ErrorCode};

/// One file a change set puts in place.
pub(super) struct Replacement {
    pub path: PathBuf,
    /// The file's new bytes.
    pub bytes: Vec<u8>,
    /// What the file is, for an error, such as "the DWG".
    pub what: &'static str,
}

/// Where one replacement got to: the file that was there, moved aside, and
/// whether the new one is in place.
struct Placed {
    aside: Option<PathBuf>,
}

/// Puts every file of `replacements` in place, or none of them. On failure
/// the error says so, and every file is as it was before.
pub(super) fn commit(replacements: &[Replacement]) -> Result<(), Error> {
    let mut made: Vec<PathBuf> = Vec::new();
    let mut staged: Vec<PathBuf> = Vec::new();
    if let Err(e) = stage(replacements, &mut made, &mut staged) {
        clean(&staged, &made);
        return Err(Error {
            message: format!("{}; nothing was changed", e.message),
            ..e
        });
    }
    let mut placed: Vec<Placed> = Vec::with_capacity(replacements.len());
    for (i, (replacement, temporary)) in replacements.iter().zip(&staged).enumerate() {
        match place(i, &replacement.path, temporary) {
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
    for done in &placed {
        if let Some(aside) = &done.aside {
            let _ = fs::remove_file(aside);
        }
    }
    Ok(())
}

/// Makes the folders the new files go in, and writes each beside its place.
fn stage(
    replacements: &[Replacement],
    made: &mut Vec<PathBuf>,
    staged: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    for replacement in replacements {
        let path = &replacement.path;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let missing: Vec<&Path> = parent.ancestors().take_while(|a| !a.exists()).collect();
            fs::create_dir_all(parent).map_err(|e| {
                Error::new(
                    ErrorCode::WriteFailed,
                    format!(
                        "could not make the folder {} goes in: {e}",
                        replacement.what
                    ),
                )
                .in_file(parent)
            })?;
            made.extend(missing.into_iter().map(Path::to_owned));
        }
        staged.push(write_beside(path, &replacement.bytes, replacement.what)?);
    }
    Ok(())
}

/// Moves the file at `path`, if there is one, aside, and the new one from
/// `temporary` into its place. When the new one cannot be moved, the old
/// one is put back before the error is given.
fn place(index: usize, path: &Path, temporary: &Path) -> io::Result<Placed> {
    fail_here(index)?;
    let aside = if fs::symlink_metadata(path).is_ok() {
        let aside = beside(path, "old");
        fs::rename(path, &aside)?;
        Some(aside)
    } else {
        None
    };
    if let Err(e) = fs::rename(temporary, path) {
        if let Some(aside) = &aside {
            let _ = fs::rename(aside, path);
        }
        return Err(e);
    }
    Ok(Placed { aside })
}

/// Puts back, last first, every file `placed` moved, and gives the files it
/// could not put back.
fn roll_back(replacements: &[Replacement], placed: &[Placed]) -> Vec<String> {
    let mut not_back = Vec::new();
    for (replacement, done) in replacements.iter().zip(placed).rev() {
        let path = &replacement.path;
        let back = match &done.aside {
            Some(aside) => fs::rename(aside, path),
            None => fs::remove_file(path),
        };
        if back.is_err() {
            not_back.push(path.display().to_string());
        }
    }
    not_back
}

/// Takes away the new files not moved into place and the folders made for
/// them, deepest first, each only when it is empty.
fn clean(staged: &[PathBuf], made: &[PathBuf]) {
    for temporary in staged {
        let _ = fs::remove_file(temporary);
    }
    for folder in made {
        let _ = fs::remove_dir(folder);
    }
}

#[cfg(test)]
thread_local! {
    /// The replacement a test asks to fail, by its place in the commit.
    pub(super) static FAIL_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Fails replacement `index` when a test asks for it, so a test can show a
/// failed commit puts every file back.
fn fail_here(index: usize) -> io::Result<()> {
    #[cfg(test)]
    if FAIL_AT.with(std::cell::Cell::get) == Some(index) {
        return Err(io::Error::other("a failure the test asked for"));
    }
    let _ = index;
    Ok(())
}
