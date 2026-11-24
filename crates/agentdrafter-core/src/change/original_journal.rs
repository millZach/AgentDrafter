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
//!   or removed (including `.agentdrafter`, `originals`, `<safe-stem>`,
//!   `undo-states`, `journal.jsonl`, `journal.lock`, and revision files).
//! - `--dry-run` on `change`, `undo`, and `accept` never acquires a write lock,
//!   never runs crash recovery, never creates `.agentdrafter/originals/`, and
//!   refuses if an unfinished pending entry or torn last line exists.
//! - `journal` on an original with no journal yet returns an empty report
//!   without creating any folder or file.
//! - Issued-stamp lookup for `render` and `plot` is strictly read-only.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

#[cfg(any(unix, windows))]
use crate::build::file_id_of_file;
#[cfg(not(any(unix, windows)))]
use crate::build::is_same_file;
use crate::build::FileId;
use crate::check::check_dwg_bytes;
use crate::error::{Error, ErrorCode};
use crate::interface::Interface;
use crate::original::{
    ensure_real_dir_chain, is_drawing, on_disk_path, read_regular_no_follow,
    read_regular_no_follow_with_id, stem_of, verify_no_symlink_ancestors,
    verify_no_symlink_intermediate_dirs, verify_real_dir_chain, NoFollowRead, NoFollowReadWithId,
};
use crate::reply::Reply;
use crate::revise::REVISION_CHANGES_EXTENSION;
use crate::settings::Settings;
use crate::stamp::fingerprint;

use super::accept::{
    failed_words, issued, moved_on, rule_text, AcceptReport, AcceptRequest, StampRevision,
};
use super::commit::{
    add_line_original, commit_new_files, commit_original_replacements, cut_back_checked,
    moved_aside_original, restore_original_inspected, Ahead, Before, ExpectedTarget,
    InspectedBefore, NewFile, OriginalGuard, Replacement,
};
use super::journal::{
    build_journal_report, code_text, fates, in_place, keeps_state, left_pending, line_of, listed,
    lock_original, next_number, parse_journal_bytes, pending, prune_original_at, state_line,
    undo_state_original_at, CheckFailed, CommitState, Entry, EntryKind, FileIssued, FileState,
    JournalRead, JournalReport, JournalRequest, ObjectChanged, OriginalRevisionMeta, Refusal,
    SheetFailed, KEPT_UNDO_STATES,
};
use super::recover::{kind_words, RolledBack};
use super::undo::{
    can_undo, message as undo_message, FileUndone, UndoAction, UndoReport, UndoRequest,
};

/// Folder, relative to an original DWG's parent folder, that holds per-original
/// revision journals (`<safe-stem>/journal.jsonl`, `journal.lock`,
/// `undo-states/`).
pub const ORIGINALS_JOURNAL_FOLDER: &str = ".agentdrafter/originals";

/// Returns the on-disk path of the per-original `journal.jsonl` for an
/// existing `.dwg` file, using the same canonical key resolution as `change`,
/// `journal`, `undo`, and `accept`. Never creates any folder or file.
///
/// Returns `None` when `source` is not an existing `.dwg` file.
pub fn original_journal_file(source: &Path) -> Option<PathBuf> {
    if !crate::original::is_drawing_file(source) {
        return None;
    }
    OriginalJournal::open(source).ok().map(|oj| oj.journal_file)
}

/// Metadata of the latest active (finished, non-undone, non-standalone) working
/// revision on disk for an original DWG, used so successive `change` operations
/// accumulate on top of the active working revision rather than dropping earlier
/// edits.
pub(crate) struct ActiveRevisionTip {
    pub entry: u64,
    pub path: PathBuf,
    pub file: String,
    pub original_sha256: String,
    pub revision_sha256: String,
}

/// Per-original journal paths and identity of the original DWG.
pub(crate) struct OriginalJournal {
    /// The original DWG path as requested by the caller (for display).
    pub source: PathBuf,
    /// Canonical on-disk path of the original DWG.
    pub resolved_source: PathBuf,
    /// Canonical parent folder of the original DWG.
    pub orig_dir_real: PathBuf,
    /// Canonical sheet name of the original (its on-disk stem).
    pub sheet: String,
    /// Collision-free per-folder journal key (`<readable>-<namehash12>`).
    pub key: String,
    /// Globally unique temp/backup scope (`<key>-<fullpathhash16>`), distinct
    /// even when two originals in different folders share a filename and write
    /// to the same output folder.
    pub temp_scope: String,
    /// Parent folder used for caller-relative display when `source` lives
    /// directly in `orig_dir_real`, or `orig_dir_real` when `source` is a
    /// symlink pointing into another directory.
    pub base_dir: PathBuf,
    /// Whether `source`'s parent folder canonicalizes to `orig_dir_real`.
    base_dir_matches_real: bool,
    /// `<dir>/journal.jsonl`
    pub journal_file: PathBuf,
    /// `<dir>/journal.lock`
    pub lock_file: PathBuf,
    /// `<dir>/undo-states`
    pub undo_states_dir: PathBuf,
    /// File identity of the original DWG, captured without modifying it.
    pub source_id: Option<FileId>,
}

/// Globally unique temp/backup scope derived from the full canonical original
/// path (`orig_dir_real` + on-disk filename), so `east/A.dwg` and `west/A.dwg`
/// writing to a shared output folder at entry 1 never collide.
pub(crate) fn original_temp_scope(resolved: &Path) -> String {
    let full_hash = &fingerprint(resolved.as_os_str().as_encoded_bytes())[..16];
    format!("{}-{full_hash}", original_key(resolved))
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

/// Resolves `path` to a real absolute path by canonicalizing its deepest
/// existing ancestor (usually its parent folder) and re-appending remaining
/// components verbatim — never lexically collapsing `..` across symlinks.
fn resolve_output_real(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name() else {
        return fs::canonicalize(path)
            .or_else(|_| std::path::absolute(path))
            .unwrap_or_else(|_| path.to_path_buf());
    };
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if let Ok(canon_parent) = fs::canonicalize(parent) {
        return canon_parent.join(file_name);
    }
    let mut tail = vec![file_name.to_os_string()];
    let mut cur = parent;
    loop {
        if let Ok(canon) = fs::canonicalize(cur) {
            let mut out = canon;
            for part in tail.into_iter().rev() {
                out.push(part);
            }
            return out;
        }
        let Some(name) = cur.file_name() else {
            break;
        };
        tail.push(name.to_os_string());
        match cur.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(p) => cur = p,
            None => {
                if let Ok(cwd) = std::env::current_dir().and_then(fs::canonicalize) {
                    let mut out = cwd;
                    for part in tail.into_iter().rev() {
                        out.push(part);
                    }
                    return out;
                }
                break;
            }
        }
    }
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl OriginalJournal {
    /// Constructs an [`OriginalJournal`] from an original DWG already opened
    /// and resolved via [`on_disk_path`]. Never creates any folder or file.
    pub(crate) fn from_opened(source: &Path, resolved: &Path, source_id: Option<FileId>) -> Self {
        let sheet = stem_of(resolved);
        let key = original_key(resolved);
        let temp_scope = original_temp_scope(resolved);
        let source_parent = source
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let orig_dir_real = resolved
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .and_then(|p| fs::canonicalize(p).ok().or_else(|| Some(p.to_path_buf())))
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .and_then(fs::canonicalize)
                    .unwrap_or_else(|_| PathBuf::from("."))
            });
        let source_parent_fs = if source_parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &source_parent
        };
        let base_dir_matches_real = fs::canonicalize(source_parent_fs)
            .ok()
            .is_some_and(|p| p == orig_dir_real);
        // When `source` is a symlink pointing into another directory (e.g.
        // `aliases/A.dwg -> drawings/A.dwg`), anchor the journal in
        // `orig_dir_real` (`drawings/`) so both the symlink and the real path
        // share one journal and resolve outputs identically.
        let base_dir = if base_dir_matches_real {
            source_parent
        } else {
            orig_dir_real.clone()
        };
        let dir = base_dir.join(ORIGINALS_JOURNAL_FOLDER).join(&key);
        let journal_file = dir.join("journal.jsonl");
        let lock_file = dir.join("journal.lock");
        let undo_states_dir = dir.join("undo-states");
        Self {
            source: source.to_path_buf(),
            resolved_source: resolved.to_path_buf(),
            orig_dir_real,
            sheet,
            key,
            temp_scope,
            base_dir,
            base_dir_matches_real,
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

    fn base_dir_for_fs(&self) -> &Path {
        if self.base_dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.base_dir
        }
    }

    fn guard(&self) -> OriginalGuard<'_> {
        OriginalGuard {
            key: &self.key,
            temp_scope: &self.temp_scope,
            base_dir: self.base_dir_for_fs(),
            orig_dir_real: &self.orig_dir_real,
            source: &self.source,
            source_id: self.source_id,
        }
    }

    /// Acquires this original's exclusive journal lock (`journal.lock`),
    /// verifying every directory component is a real directory (not a symlink)
    /// and creating `.agentdrafter/originals/<key>/` if needed.
    pub(crate) fn lock(&self) -> Result<File, Error> {
        lock_original(
            self.base_dir_for_fs(),
            &self.key,
            &self.lock_file,
            self.source_id,
        )
    }

    /// Whether `path`'s canonical parent directory is `self.orig_dir_real`
    /// itself (directly beside the original DWG, NOT in a child, sibling, or
    /// external folder).
    pub(crate) fn is_directly_in_orig_dir(&self, path: &Path) -> bool {
        let resolved = resolve_output_real(path);
        resolved.parent() == Some(self.orig_dir_real.as_path())
    }

    /// Computes the path to record in `journal.jsonl` for an output file:
    /// - if the resolved output path (canonical parent + filename, never
    ///   collapsing `..` across symlinks) lies inside `self.orig_dir_real`,
    ///   returns its clean relative path from `self.orig_dir_real` with `/`
    ///   separators;
    /// - otherwise returns the resolved canonical absolute path so `journal`,
    ///   `undo`, `accept`, and crash recovery resolve it accurately from any
    ///   working directory.
    pub(crate) fn rel_path(&self, path: &Path) -> String {
        let resolved = resolve_output_real(path);
        if let Ok(rel) = resolved.strip_prefix(&self.orig_dir_real) {
            let parts: Vec<String> = rel
                .components()
                .filter_map(|c| match c {
                    Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            if !parts.is_empty() {
                return parts.join("/");
            }
        }
        resolved.to_string_lossy().into_owned()
    }

    /// Resolves a path recorded in `journal.jsonl` against `self.orig_dir_real`
    /// (the exact same canonical base used by [`Self::rel_path`]), so symlinked
    /// originals such as `aliases/A.dwg -> drawings/A.dwg` resolve to the exact
    /// same file as `drawings/A.dwg`.
    pub(crate) fn resolve_file(&self, rel: &str) -> PathBuf {
        let p = Path::new(rel);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.orig_dir_real.join(p)
        }
    }

    /// Caller-facing display string for a recorded output path: preserves
    /// caller-relative display when `source` is directly in `orig_dir_real`
    /// (for deterministic cross-interface replies), otherwise shows the
    /// resolved path.
    fn display_file(&self, rel: &str) -> String {
        let p = Path::new(rel);
        if p.is_absolute() {
            rel.to_owned()
        } else if self.base_dir_matches_real {
            if self.base_dir.as_os_str().is_empty() {
                p.display().to_string()
            } else {
                self.base_dir.join(p).display().to_string()
            }
        } else {
            self.orig_dir_real.join(p).display().to_string()
        }
    }

    /// Verifies that every intermediate directory component between
    /// `orig_dir_real` and a recorded output file (or the file's immediate
    /// parent when absolute) is a real non-symlink directory.
    fn verify_output_parent_real(&self, rel: &str, full: &Path) -> Result<(), Error> {
        let p = Path::new(rel);
        if !p.is_absolute() {
            verify_no_symlink_intermediate_dirs(&self.orig_dir_real, p).map_err(|e| {
                Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "`{}` cannot be accessed safely: {e}; AgentDrafter never follows a \
                         symbolic link; nothing was changed",
                        full.display()
                    ),
                )
                .in_file(full)
            })?;
        } else if let Some(parent) = full.parent().filter(|d| !d.as_os_str().is_empty()) {
            verify_no_symlink_ancestors(parent).map_err(|e| {
                Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "`{}` cannot be accessed safely: {e}; AgentDrafter never follows a \
                         symbolic link; nothing was changed",
                        full.display()
                    ),
                )
                .in_file(full)
            })?;
        }
        Ok(())
    }

    /// Verifies the directory chain `.agentdrafter/originals/<key>` contains no
    /// symlinks and checks whether `journal.jsonl` exists as a safe regular
    /// file distinct from the original DWG.
    fn journal_exists_checked(&self) -> Result<bool, Error> {
        let dir_exists = verify_real_dir_chain(
            self.base_dir_for_fs(),
            &[".agentdrafter", "originals", &self.key],
        )
        .map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("cannot inspect the original's journal folder: {e}"),
            )
            .in_file(&self.journal_file)
        })?;
        if !dir_exists {
            return Ok(false);
        }
        match fs::symlink_metadata(&self.journal_file) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::new(
                ErrorCode::ReadFailed,
                format!("could not inspect the original's journal: {e}"),
            )
            .in_file(&self.journal_file)),
            Ok(meta) if meta.file_type().is_symlink() => Err(Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "`{}` is a symbolic link; AgentDrafter never follows a symbolic link as a \
                     journal",
                    self.journal_file.display()
                ),
            )
            .in_file(&self.journal_file)),
            Ok(meta) if !meta.file_type().is_file() => Err(Error::new(
                ErrorCode::ReadFailed,
                format!("`{}` is not a regular file", self.journal_file.display()),
            )
            .in_file(&self.journal_file)),
            Ok(_) => {
                #[cfg(any(unix, windows))]
                {
                    let jid = crate::original::regular_file_id_no_follow(&self.journal_file);
                    if self.source_id.is_some() && jid == self.source_id {
                        return Err(Error::new(
                            ErrorCode::BadValue,
                            format!(
                                "`{}` is the original DWG itself, which is never used as a journal \
                                 (ADR-0004)",
                                self.journal_file.display()
                            ),
                        )
                        .in_file(&self.journal_file));
                    }
                }
                Ok(true)
            }
        }
    }

    /// Reads `journal.jsonl` strictly without following symlinks on any folder
    /// component or on `journal.jsonl` itself, and refusing any file matching
    /// the original DWG.
    fn read_journal(&self) -> Result<Option<JournalRead>, Error> {
        let dir_exists = verify_real_dir_chain(
            self.base_dir_for_fs(),
            &[".agentdrafter", "originals", &self.key],
        )
        .map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("cannot read the original's journal folder: {e}"),
            )
            .in_file(&self.journal_file)
        })?;
        if !dir_exists {
            return Ok(None);
        }
        match read_regular_no_follow(&self.source, self.source_id, &self.journal_file) {
            Ok(NoFollowRead::Missing) => Ok(None),
            Ok(NoFollowRead::File(bytes)) => {
                parse_journal_bytes(&self.journal_file, &bytes).map(Some)
            }
            Ok(NoFollowRead::Symlink) => Err(Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "`{}` is a symbolic link; AgentDrafter never follows a symbolic link as a \
                     journal",
                    self.journal_file.display()
                ),
            )
            .in_file(&self.journal_file)),
            Ok(NoFollowRead::NotRegular) => Err(Error::new(
                ErrorCode::ReadFailed,
                format!("`{}` is not a regular file", self.journal_file.display()),
            )
            .in_file(&self.journal_file)),
            Ok(NoFollowRead::SameAsOriginal) => Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`{}` is the original DWG itself, which is never used as a journal (ADR-0004)",
                    self.journal_file.display()
                ),
            )
            .in_file(&self.journal_file)),
            Err(e) => Err(Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the original's journal: {e}"),
            )
            .in_file(&self.journal_file)),
        }
    }

    fn read_text(&self) -> Result<Option<(String, Vec<Entry>)>, Error> {
        let Some(read) = self.read_journal()? else {
            return Ok(None);
        };
        if let Some(torn) = read.torn {
            return Err(Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "line {} of the original's journal was written only in part, by a process \
                     that stopped while it wrote it",
                    torn.line
                ),
            )
            .at_position(torn.line, 1)
            .in_file(&self.journal_file));
        }
        Ok(Some((read.text, read.entries)))
    }

    fn read_entries(&self) -> Result<Vec<Entry>, Error> {
        Ok(self.read_text()?.map_or_else(Vec::new, |(_, e)| e))
    }

    fn prune(&self, entries: &[Entry]) {
        prune_original_at(self.base_dir_for_fs(), &self.key, entries, self.source_id);
    }

    /// Read-only check for `--dry-run`: refuses if the original's journal holds
    /// an unfinished pending entry or a torn last line, without acquiring any
    /// lock or creating any directory.
    pub(crate) fn refuse_unfinished(&self) -> Result<(), Error> {
        let Some(read) = self.read_journal()? else {
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
        let Some(read) = self.read_journal()? else {
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
        let Some(read) = self.read_journal()? else {
            return Ok(None);
        };
        let Some(entry) = pending(&read.entries) else {
            let Some(torn) = read.torn else {
                return Ok(None);
            };
            cut_back_checked(&self.journal_file, torn.at, self.source_id).map_err(|e| {
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

        // Single-read inspection of every file in the pending entry: verifies
        // intermediate directories, checks non-symlink/non-original safety, and
        // captures BOTH SHA-256 and `FileId` in one `read_regular_no_follow_with_id`
        // call with zero second reads.
        let (inspected, changed) = self.inspect_pending_files(entry)?;
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

        let made: Vec<PathBuf> = entry
            .folders_made
            .iter()
            .map(|f| self.resolve_file(f))
            .collect();
        let restored = restore_original_inspected(
            entry.entry,
            &inspected,
            &self.undo_states_dir,
            &made,
            self.guard(),
        )
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
            cut_back_checked(&self.journal_file, torn.at, self.source_id).map_err(|e| {
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
        add_line_original(
            &self.journal_file,
            &state_line(
                Some(&read.text),
                entry.entry,
                CommitState::RolledBack,
                files,
            ),
            &format!("the line that marks entry {} rolled back", entry.entry),
            self.guard(),
        )
        .map_err(|e| rest(e.message))?;
        if let Ok(entries) = self.read_entries() {
            self.prune(&entries);
        }

        let files: Vec<FileUndone> = restored
            .iter()
            .map(|r| {
                let rel = self.rel_path(&r.path);
                FileUndone {
                    file: self.display_file(&rel),
                    action: if r.back.is_some() {
                        UndoAction::Restored
                    } else {
                        UndoAction::Removed
                    },
                }
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

    /// Parses a sibling automatic working-revision filename
    /// (`<non-empty-stem>.rev<1+ ASCII digits>.dwg` with no directory
    /// components) and returns `<stem>`, or `None` otherwise.
    fn parse_sibling_working_revision_stem(revision_dwg: &str) -> Option<&str> {
        if revision_dwg.is_empty() || revision_dwg.contains('/') {
            return None;
        }
        let mut comps = Path::new(revision_dwg).components();
        let Component::Normal(file_name) = comps.next()? else {
            return None;
        };
        if comps.next().is_some() {
            return None;
        }
        let name = file_name.to_str()?;
        let without_ext = name.strip_suffix(".dwg").or_else(|| {
            let ext_start = name.len().checked_sub(4)?;
            name.is_char_boundary(ext_start)
                .then(|| &name[ext_start..])
                .filter(|ext| ext.eq_ignore_ascii_case(".dwg"))
                .map(|_| &name[..ext_start])
        })?;
        let (stem, num) = without_ext.rsplit_once(".rev")?;
        if stem.is_empty()
            || num.is_empty()
            || num.starts_with('0')
            || !num.bytes().all(|b| b.is_ascii_digit())
            || num.parse::<u64>().ok().is_none_or(|v| v == 0)
        {
            return None;
        }
        Some(stem)
    }

    /// Whether a sibling `<stem>.dwg` / `<stem>.DWG` in `self.orig_dir_real`
    /// resolves to the exact same underlying file identity as this original
    /// (e.g. a same-directory symlink `Alias.dwg -> Pump.dwg`).
    fn sibling_dwg_is_same_original(&self, stem: &str) -> bool {
        for ext in [".dwg", ".DWG"] {
            let candidate = self.orig_dir_real.join(format!("{stem}{ext}"));
            if let Ok(canon) = fs::canonicalize(&candidate) {
                if canon == self.resolved_source {
                    return true;
                }
            }
            #[cfg(any(unix, windows))]
            if let Some(orig_id) = self.source_id {
                if let Ok(file) = File::open(&candidate) {
                    if file_id_of_file(&file) == Some(orig_id) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Authoritatively classifies whether an `OriginalRevisionMeta` entry is a
    /// standalone `--output` revision (`true`) or part of the sequential working
    /// revision chain (`false`).
    ///
    /// - Entries written with an explicit `Some(flag)` use `flag` directly.
    /// - Legacy pre-PR entries (`None` on disk) are classified as working ONLY
    ///   when `revision_dwg` is a sibling `<stem>.rev<N>.dwg` whose `<stem>`
    ///   either matches this original's stem or names a sibling `<stem>.dwg`
    ///   pointing to the exact same underlying file identity (such as a
    ///   same-directory symlink alias). Legacy `--output IssuedCopy.rev1.dwg`
    ///   beside `Pump.dwg` has no `IssuedCopy.dwg -> Pump.dwg`, so it is
    ///   authoritatively classified as standalone (`true`).
    pub(crate) fn is_standalone_meta(&self, meta: &OriginalRevisionMeta) -> bool {
        if let Some(standalone) = meta.standalone_output {
            return standalone;
        }
        let Some(stem) = Self::parse_sibling_working_revision_stem(&meta.revision_dwg) else {
            return true;
        };
        let source_stem = stem_of(&self.source);
        #[cfg(any(windows, target_os = "macos"))]
        let matches_self =
            stem.eq_ignore_ascii_case(&self.sheet) || stem.eq_ignore_ascii_case(&source_stem);
        #[cfg(not(any(windows, target_os = "macos")))]
        let matches_self = stem == self.sheet || stem == source_stem;
        if matches_self {
            return false;
        }
        !self.sibling_dwg_is_same_original(stem)
    }

    /// Returns the newest active (finished, non-undone, non-standalone) working
    /// revision tip recorded in this original's journal, or `None` when none
    /// exists yet. Only inspects the newest active non-standalone entry so
    /// callers checking `original_sha256` never skip past a newer original's
    /// entry back to an older original's revision.
    /// Read-only: never creates any directory or file.
    pub(crate) fn latest_active_revision(&self) -> Result<Option<ActiveRevisionTip>, Error> {
        let entries = self.read_entries()?;
        if entries.is_empty() {
            return Ok(None);
        }
        let f = fates(&entries);
        for entry in entries.iter().rev() {
            if entry.kind != EntryKind::ChangeSet || !entry.finished() {
                continue;
            }
            if f.get(&entry.entry)
                .is_some_and(|fate| fate.undone_by.is_some())
            {
                continue;
            }
            let Some(meta) = entry.original_revision.as_ref() else {
                continue;
            };
            if self.is_standalone_meta(meta) {
                continue;
            }
            let path = self.resolve_file(&meta.revision_dwg);
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| meta.revision_dwg.clone());
            return Ok(Some(ActiveRevisionTip {
                entry: entry.entry,
                path,
                file,
                original_sha256: meta.original_sha256.clone(),
                revision_sha256: meta.revision_sha256.clone(),
            }));
        }
        Ok(None)
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
        let (text, entries_before) = self.read_text()?.unzip();
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
            self.guard(),
        );
        if committed.is_ok() {
            let mut entries = entries_before;
            entry.state = Some(CommitState::Done);
            entries.push(entry);
            self.prune(&entries);
        } else if let Ok(entries) = self.read_entries() {
            self.prune(&entries);
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
        let read = self.read_text().ok()?;
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
        add_line_original(&self.journal_file, &line, "a failed attempt", self.guard())
            .ok()
            .map(|()| number)
    }

    fn inspect_pending_files(
        &self,
        entry: &Entry,
    ) -> Result<(Vec<InspectedBefore>, Vec<String>), Error> {
        let mut inspected = Vec::with_capacity(entry.files.len());
        let mut changed = Vec::new();
        for (j, file) in entry.files.iter().enumerate() {
            let path = self.resolve_file(&file.path);
            self.verify_output_parent_real(&file.path, &path)
                .map_err(|e| self.cannot(Some(entry), &e.message))?;
            let (found, target) =
                match read_regular_no_follow_with_id(&self.source, self.source_id, &path) {
                    Ok(NoFollowReadWithId::File(bytes, id)) => {
                        (Some(fingerprint(&bytes)), ExpectedTarget::Existing(id))
                    }
                    Ok(NoFollowReadWithId::Missing) => (None, ExpectedTarget::Absent),
                    Ok(NoFollowReadWithId::SameAsOriginal) => {
                        return Err(self.cannot(
                            Some(entry),
                            &format!(
                                "`{}` is the original DWG itself, which is never modified or \
                                 removed (ADR-0004); nothing was changed",
                                path.display()
                            ),
                        ));
                    }
                    Ok(NoFollowReadWithId::Symlink) => {
                        return Err(self.cannot(
                            Some(entry),
                            &format!(
                                "`{}` is a symbolic link; AgentDrafter never follows, modifies or \
                                 removes a symbolic link; nothing was changed",
                                path.display()
                            ),
                        ));
                    }
                    Ok(NoFollowReadWithId::NotRegular) => {
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
                                "could not read {} to see whether it changed since: {e}; nothing \
                                 was changed",
                                path.display()
                            ),
                        ));
                    }
                };
            inspected.push(InspectedBefore {
                path: path.clone(),
                print: file.before.clone(),
                found: found.clone(),
                target,
            });
            if found == file.before || found == file.after {
                continue;
            }
            let prints: Vec<&str> = [&file.before, &file.after]
                .into_iter()
                .flatten()
                .map(String::as_str)
                .collect();
            if found.is_none() && moved_aside_original(entry.entry, self.guard(), &path, j, &prints)
            {
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
        Ok((inspected, changed))
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
    let mut entries = match oj.read_entries() {
        Ok(entries) => entries,
        Err(error) => return Reply::from(Err(error)).after(rolled_back),
    };
    for entry in &mut entries {
        if let Some(meta) = entry.original_revision.as_mut() {
            if meta.standalone_output.is_none() {
                meta.standalone_output = Some(oj.is_standalone_meta(meta));
            }
            meta.revision_path = Some(oj.display_file(&meta.revision_dwg));
        }
    }
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
    if request.dry_run {
        if let Err(error) = oj.refuse_unfinished() {
            return Reply::from(Err(error));
        }
        return Reply::from(undo_original_inner(&oj, request, None));
    }
    // Never create `.agentdrafter/originals/` or `journal.lock` when no
    // journal exists yet.
    match oj.journal_exists_checked() {
        Ok(false) => {
            let empty = BTreeMap::new();
            return Reply::from(Err(Error::new(
                ErrorCode::UnknownName,
                format!(
                    "the original's journal has no entry {}; {}",
                    request.change_set,
                    can_undo(&[], &empty)
                ),
            )
            .at_field("change_set")));
        }
        Ok(true) => {}
        Err(error) => return Reply::from(Err(error)),
    }
    // Hold `journal.lock` continuously across recovery -> reading journal ->
    // verifying files -> committing undo -> pruning so no concurrent revision
    // can interleave.
    let lock = match oj.lock() {
        Ok(l) => l,
        Err(error) => return Reply::from(Err(error)),
    };
    let rolled_back = match oj.recover_locked(&lock) {
        Ok(rb) => rb,
        Err(error) => return Reply::from(Err(error)),
    };
    Reply::from(undo_original_inner(&oj, request, Some(&lock))).after(rolled_back)
}

fn undo_original_inner(
    oj: &OriginalJournal,
    request: &UndoRequest,
    locked: Option<&File>,
) -> Result<UndoReport, Error> {
    let (text, entries) = oj.read_text()?.unzip();
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

    // Read and verify every file without following symlinks, refusing the
    // original DWG itself (ADR-0004), and capturing each file's opened `FileId`
    // for pre-unlink re-verification.
    let mut now_bytes: Vec<Option<Vec<u8>>> = Vec::with_capacity(plans.len());
    let mut now_prints: Vec<Option<String>> = Vec::with_capacity(plans.len());
    let mut now_targets: Vec<ExpectedTarget> = Vec::with_capacity(plans.len());
    for plan in &plans {
        let full = oj.resolve_file(&plan.path);
        oj.verify_output_parent_real(&plan.path, &full)
            .map_err(|e| e.at_field("change_set"))?;
        match read_regular_no_follow_with_id(&oj.source, oj.source_id, &full) {
            Ok(NoFollowReadWithId::SameAsOriginal) => {
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
            Ok(NoFollowReadWithId::Symlink) => {
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
            Ok(NoFollowReadWithId::NotRegular) => {
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
            Ok(NoFollowReadWithId::File(bytes, file_id)) => {
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
                now_targets.push(ExpectedTarget::Existing(file_id));
            }
            Ok(NoFollowReadWithId::Missing) => {
                if plan.left.is_some() {
                    changed.push(format!(
                        "`{}` (taken away since change set {} left it)",
                        full.display(),
                        plan.left_by
                    ));
                }
                now_prints.push(None);
                now_bytes.push(None);
                now_targets.push(ExpectedTarget::Absent);
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
    let mut replacement_targets: Vec<ExpectedTarget> = Vec::new();
    let mut states: Vec<FileState> = Vec::new();
    let mut files: Vec<FileUndone> = Vec::new();
    let mut undo_state_blobs: Vec<(String, Vec<u8>)> = Vec::new();
    for (((plan, print), current_bytes), target_id) in plans
        .iter()
        .zip(&now_prints)
        .zip(now_bytes)
        .zip(now_targets)
    {
        if *print == plan.target {
            continue;
        }
        let target_bytes = match &plan.target {
            Some(target) => Some(
                undo_state_original_at(
                    oj.base_dir_for_fs(),
                    &oj.key,
                    &oj.source,
                    oj.source_id,
                    target,
                )
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::ReadFailed,
                        format!(
                            "change set {wanted} cannot be undone: the undo state of `{}` is \
                             missing or damaged; nothing was changed",
                            oj.resolve_file(&plan.path).display()
                        ),
                    )
                    .in_file(&oj.undo_states_dir.join(target))
                })?,
            ),
            None => None,
        };
        if let (Some(p), Some(b)) = (print, current_bytes) {
            undo_state_blobs.push((p.clone(), b));
        }
        let full = oj.resolve_file(&plan.path);
        files.push(FileUndone {
            file: oj.display_file(&plan.path),
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
        replacement_targets.push(target_id);
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

    debug_assert!(
        locked.is_some(),
        "non-dry-run undo must hold journal.lock from before reading the journal"
    );
    let _ = locked;

    // Ensure `undo-states` is a real non-symlink directory before staging
    // backup undo states into it.
    if !undo_state_blobs.is_empty() {
        ensure_real_dir_chain(
            oj.base_dir_for_fs(),
            &[".agentdrafter", "originals", &oj.key, "undo-states"],
        )
        .map_err(|e| {
            Error::new(
                ErrorCode::WriteFailed,
                format!("could not verify the original's undo-states folder: {e}"),
            )
            .in_file(&oj.undo_states_dir)
        })?;
    }

    let number = next_number(&entries);
    let mut placing: Vec<Replacement> = Vec::new();
    let mut placing_targets: Vec<ExpectedTarget> = Vec::new();
    for (print, bytes) in undo_state_blobs {
        let state_path = oj.undo_states_dir.join(&print);
        if placing.iter().any(|r| r.path == state_path) {
            continue;
        }
        match read_regular_no_follow(&oj.source, oj.source_id, &state_path) {
            Ok(NoFollowRead::File(existing)) if fingerprint(&existing) == print => continue,
            Ok(NoFollowRead::Missing) => {
                placing.push(Replacement {
                    path: state_path,
                    bytes: Some(bytes),
                    what: "an undo state",
                });
                placing_targets.push(ExpectedTarget::Absent);
            }
            Ok(NoFollowRead::File(_)) => {
                // Corrupt undo state file: refuse rather than silently overwriting.
                return Err(Error::new(
                    ErrorCode::WriteFailed,
                    format!(
                        "existing undo state `{}` does not match its SHA-256; nothing was changed",
                        state_path.display()
                    ),
                )
                .in_file(&state_path));
            }
            Ok(NoFollowRead::Symlink | NoFollowRead::NotRegular | NoFollowRead::SameAsOriginal) => {
                return Err(Error::new(
                    ErrorCode::WriteFailed,
                    format!(
                        "unsafe entry at undo state `{}`; nothing was changed",
                        state_path.display()
                    ),
                )
                .in_file(&state_path));
            }
            Err(e) => {
                return Err(Error::new(
                    ErrorCode::WriteFailed,
                    format!(
                        "could not inspect undo state `{}`: {e}",
                        state_path.display()
                    ),
                )
                .in_file(&state_path));
            }
        }
    }
    let before: Vec<Before> = states
        .iter()
        .map(|f| Before {
            path: oj.resolve_file(&f.path),
            print: f.before.clone(),
        })
        .collect();
    placing.extend(replacements);
    placing_targets.extend(replacement_targets);
    let mut undo_entry = Entry {
        undid: undid.clone(),
        files: states,
        ..Entry::new(number, EntryKind::Undo)
    };
    undo_entry.state = Some(CommitState::Pending);
    let pending_line = line_of(text.as_deref(), &undo_entry);
    let done_line = state_line(None, number, CommitState::Done, Vec::new());

    let committed = commit_original_replacements(
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
        &placing_targets,
        oj.guard(),
    );
    if committed.is_ok() {
        let mut entries_after = entries;
        undo_entry.state = Some(CommitState::Done);
        entries_after.push(undo_entry);
        oj.prune(&entries_after);
    } else if let Ok(entries_now) = oj.read_entries() {
        oj.prune(&entries_now);
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
    if request.dry_run {
        if let Err(error) = oj.refuse_unfinished() {
            return Reply::from(Err(error));
        }
        return match accept_original_inner(&oj, request, settings, via, None) {
            Ok(OriginalAccepted::Done(report)) => Reply::from(Ok(report)),
            Ok(OriginalAccepted::ChecksFailed(report, error)) => Reply::partly(report, error),
            Err(error) => Reply::from(Err(error)),
        };
    }
    // Never create `.agentdrafter/originals/` or `journal.lock` when no
    // journal exists yet.
    match oj.journal_exists_checked() {
        Ok(false) => {
            if let Some(seen) = request.newest_seen {
                if seen != 0 {
                    return Reply::from(Err(moved_on(&[], seen, 0)));
                }
            }
            return Reply::from(Err(Error::new(
                ErrorCode::NothingToAccept,
                format!(
                    "there is nothing to accept: no revision of `{}` is in place to issue; \
                     nothing was issued",
                    oj.source.display()
                ),
            )));
        }
        Ok(true) => {}
        Err(error) => return Reply::from(Err(error)),
    }
    // Hold `journal.lock` continuously across recovery -> reading journal ->
    // verifying revision DWGs -> checking -> committing accept so no concurrent
    // revision can interleave.
    let lock = match oj.lock() {
        Ok(l) => l,
        Err(error) => return Reply::from(Err(error)),
    };
    let rolled_back = match oj.recover_locked(&lock) {
        Ok(rb) => rb,
        Err(error) => return Reply::from(Err(error)),
    };
    let reply = match accept_original_inner(&oj, request, settings, via, Some(&lock)) {
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
    let (text, entries) = oj.read_text()?.unzip();
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
        let display_path = PathBuf::from(oj.display_file(&rev_rel));
        oj.verify_output_parent_real(&rev_rel, &full)?;
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
        latest_dwg = Some((display_path, bytes));
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

    debug_assert!(
        locked.is_some(),
        "non-dry-run accept must hold journal.lock from before reading the journal"
    );
    let _ = locked;

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

    commit_original_replacements(
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
        &[],
        oj.guard(),
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
        if is_drawing(Path::new(&f.path)) {
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
/// in either:
/// 1. its parent folder's `.agentdrafter/originals/*/journal.jsonl`, or
/// 2. when `--output` placed the revision outside the original's folder, the
///    original's journal referenced by the sibling `<rev>.changes.json`
///    `original_path` field.
///
/// Never acquires a lock, never traverses symlinks, and never modifies or
/// creates any file or directory.
pub(super) fn issued_of_revision_dwg(dwg: &Path, bytes: &[u8]) -> Option<StampRevision> {
    // Refuse symlinks/reparse points on the queried DWG itself: a symlink
    // pointing at an accepted revision (or vice versa) never stamps as issued.
    #[cfg(any(unix, windows))]
    let dwg_id = crate::original::regular_file_id_no_follow(dwg)?;
    #[cfg(not(any(unix, windows)))]
    {
        let meta = fs::symlink_metadata(dwg).ok()?;
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return None;
        }
    }

    let parent = dwg
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));

    let mut candidates: Vec<(PathBuf, PathBuf)> = Vec::new();

    // 1. Journals directly beside `dwg` under `<parent>/.agentdrafter/originals/*/`.
    if matches!(
        verify_real_dir_chain(parent, &[".agentdrafter", "originals"]),
        Ok(true)
    ) {
        let originals_dir = parent.join(ORIGINALS_JOURNAL_FOLDER);
        if let Ok(read_dir) = fs::read_dir(&originals_dir) {
            let mut subdirs: Vec<PathBuf> = read_dir
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()))
                .map(|e| e.path())
                .collect();
            subdirs.sort();
            for sub in subdirs {
                candidates.push((parent.to_path_buf(), sub.join("journal.jsonl")));
            }
        }
    }

    // 2. Child, sibling, or external folder revision: inspect sibling
    //    `<rev>.changes.json` (read-only, non-symlink) for `original_path`.
    if let Some(stem) = dwg.file_stem().map(|s| s.to_string_lossy().into_owned()) {
        let changes_path = dwg.with_file_name(format!("{stem}.{REVISION_CHANGES_EXTENSION}"));
        if let Ok(NoFollowRead::File(changes_bytes)) =
            read_regular_no_follow(dwg, None, &changes_path)
        {
            if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(&changes_bytes) {
                if let Some(orig_str) = map.get("original_path").and_then(Value::as_str) {
                    let orig_path = Path::new(orig_str);
                    let orig_dir = orig_path
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new("."));
                    let key = original_key(orig_path);
                    if matches!(
                        verify_real_dir_chain(orig_dir, &[".agentdrafter", "originals", &key]),
                        Ok(true)
                    ) {
                        let jpath = orig_dir
                            .join(ORIGINALS_JOURNAL_FOLDER)
                            .join(&key)
                            .join("journal.jsonl");
                        if !candidates.iter().any(|(_, p)| p == &jpath) {
                            candidates.push((orig_dir.to_path_buf(), jpath));
                        }
                    }
                }
            }
        }
    }

    if candidates.is_empty() {
        return None;
    }

    let print = fingerprint(bytes);
    let mut best: Option<Entry> = None;

    for (orig_dir, journal_path) in candidates {
        let Ok(NoFollowRead::File(jbytes)) = read_regular_no_follow(dwg, None, &journal_path)
        else {
            continue;
        };
        let Ok(read) = parse_journal_bytes(&journal_path, &jbytes) else {
            continue;
        };
        let found = read.entries.iter().rev().find(|e| {
            e.kind == EntryKind::Accept
                && e.finished()
                && e.revision.iter().any(|f| {
                    if f.sha256 != print {
                        return false;
                    }
                    let recorded_path = Path::new(&f.path);
                    let resolved = if recorded_path.is_absolute() {
                        if let Some(rec_parent) =
                            recorded_path.parent().filter(|d| !d.as_os_str().is_empty())
                        {
                            if verify_no_symlink_ancestors(rec_parent).is_err() {
                                return false;
                            }
                        }
                        recorded_path.to_path_buf()
                    } else {
                        if verify_no_symlink_intermediate_dirs(&orig_dir, recorded_path).is_err() {
                            return false;
                        }
                        orig_dir.join(recorded_path)
                    };
                    #[cfg(any(unix, windows))]
                    {
                        crate::original::regular_file_id_no_follow(&resolved) == Some(dwg_id)
                    }
                    #[cfg(not(any(unix, windows)))]
                    {
                        fs::symlink_metadata(&resolved)
                            .is_ok_and(|m| m.file_type().is_file() && !m.file_type().is_symlink())
                            && is_same_file(&resolved, dwg)
                    }
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

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn test_original_temp_scope(resolved: &Path) -> String {
    original_temp_scope(resolved)
}
