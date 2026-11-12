//! A drawing set as it was at an entry of its journal, rebuilt in memory
//! from the undo states, for the viewer to step back through: nothing is
//! written.
//!
//! The set's files just after an entry are the files as the next entry to
//! write each of them found it: the bytes `before` it, which the undo
//! states keep for a change set until its undo state is pruned or it is
//! undone. A file no later entry wrote is as it is now. A file is found by
//! its SHA-256 in the undo states, or on disk when it is still as it was.
//! When neither holds it, the entry cannot be shown, and the error says
//! why: the undo state that held it was pruned, or the change set that
//! left it was undone, and the journal keeps no copy of a file as an undone
//! change set left it.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::journal::{fates, read_entries, relative, undo_state, EntryKind, KEPT_UNDO_STATES};
use crate::description::ReadFiles;
use crate::error::{Error, ErrorCode};
use crate::stamp::fingerprint;

/// A drawing set's files as they were just after one entry of its journal:
/// those that differ from the files on disk, by their place in the set's
/// folder. A file not held here is read from disk; a file held here never
/// is.
///
/// Each file is named as the journal names it: the path the set's reader
/// and a change set's commit both join onto the set's folder, such as
/// `templates/a.yaml` or `../shared/rows.yaml`, taken back off the folder
/// by the journal's own [`relative`]. A file the reader asks for is named
/// the same way, so a record outside the set's folder, or written with
/// `..`, is found as the journal kept it.
pub(crate) struct FilesAsOf {
    /// The set's folder, as the reader is given it.
    folder: PathBuf,
    /// Each file a later entry wrote, by its path relative to the set's
    /// folder: its bytes then, or none when there was no file.
    files: BTreeMap<String, Option<Vec<u8>>>,
}

impl FilesAsOf {
    /// The set in `folder` as it was just after its journal's entry
    /// `entry`.
    pub(crate) fn read(folder: &Path, entry: u64) -> Result<FilesAsOf, Error> {
        let entries = read_entries(folder)?;
        let newest = entries.last().map_or(0, |e| e.entry);
        if !entries.iter().any(|e| e.entry == entry) {
            let range = match newest {
                0 => "the journal is empty".to_owned(),
                1 => "it holds entry 1".to_owned(),
                n => format!("it holds entries 1 to {n}"),
            };
            return Err(Error::new(
                ErrorCode::UnknownName,
                format!("the set's journal has no entry {entry}: {range}"),
            )
            .at_field("entry"));
        }
        let fates = fates(&entries);
        // Each file a later entry wrote, as the first of them found it.
        let mut wanted: BTreeMap<&str, (Option<&str>, u64, EntryKind)> = BTreeMap::new();
        for later in entries.iter().filter(|e| e.entry > entry) {
            for file in &later.files {
                wanted.entry(file.path.as_str()).or_insert((
                    file.before.as_deref(),
                    later.entry,
                    later.kind,
                ));
            }
        }
        let mut files = BTreeMap::new();
        for (path, (print, by, kind)) in wanted {
            let Some(print) = print else {
                files.insert(path.to_owned(), None);
                continue;
            };
            let bytes = undo_state(folder, print).or_else(|| {
                fs::read(folder.join(path))
                    .ok()
                    .filter(|bytes| fingerprint(bytes) == print)
            });
            let Some(bytes) = bytes else {
                let why = match kind {
                    EntryKind::ChangeSet => match fates.get(&by) {
                        Some(fate) if fate.undone_by.is_some() => format!(
                            "change set {by} was undone, by entry {}, and its undo state, which \
                             held `{path}` as it was before it, went with it",
                            fate.undone_by.unwrap_or_default()
                        ),
                        Some(fate) if fate.pruned_by.is_some() => format!(
                            "the undo state of change set {by}, which held `{path}` as it was \
                             before it, was pruned when change set {} landed, since only the \
                             {KEPT_UNDO_STATES} newest change sets keep theirs",
                            fate.pruned_by.unwrap_or_default()
                        ),
                        _ => format!(
                            "the undo state of change set {by}, which holds `{path}` as it was \
                             before it, is missing or damaged"
                        ),
                    },
                    _ => format!(
                        "entry {by}, an undo, put `{path}` back, and the journal keeps no copy \
                         of a file as an undone change set left it"
                    ),
                };
                return Err(Error::new(
                    ErrorCode::Pruned,
                    format!("the set as it was at entry {entry} cannot be shown: {why}"),
                )
                .at_field("entry"));
            };
            files.insert(path.to_owned(), Some(bytes));
        }
        Ok(FilesAsOf {
            folder: folder.to_owned(),
            files,
        })
    }

    /// The bytes of the file at `path` as they were, if a later entry wrote
    /// it: `Some(None)` when there was no file then.
    fn held(&self, path: &Path) -> Option<Option<&[u8]>> {
        self.files
            .get(&relative(&self.folder, path))
            .map(Option::as_deref)
    }
}

impl ReadFiles for FilesAsOf {
    fn text(&self, path: &Path) -> io::Result<String> {
        match self.held(path) {
            None => fs::read_to_string(path),
            Some(None) => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "there was no such file at that entry of the journal",
            )),
            Some(Some(bytes)) => String::from_utf8(bytes.to_vec())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        }
    }
}
