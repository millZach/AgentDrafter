//! Accept: the working revision issued, as the reviewer issues it from the
//! viewer, or an agent on their behalf.
//!
//! Zach decided on 2026-11-11 that accept is a command like every other,
//! reached through every interface, so an agent can accept when the
//! reviewer asks it to. Its rules are the plan's:
//!
//! - It is refused while the set is inconsistent.
//! - It checks every sheet first. When checks fail it lists them and issues
//!   nothing, unless the caller says it has seen them and accepts despite
//!   them (`despite_failed_checks`); the viewer asks the reviewer first.
//! - It marks every change set still in place issued, and an issued change
//!   set cannot be undone.
//! - The journal records it, with the interface it came through, so the
//!   reviewer can tell an agent's accept from their own; with the change
//!   sets it issued, the failed checks it accepted despite, and the
//!   fingerprint of each file of the set, so an output made from those
//!   files says, in its stamp, that it came from an issued revision.
//!
//! An accept writes no file of the set: what it issues is in its entry,
//! written ahead and marked done as a change set's is; an accept whose
//! commit never finished is rolled back by the next command, and issued
//! nothing.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::inconsistent;
use super::journal::{
    commit_accept, fates, in_place, listed, read_entries, Accepting, CheckFailed, Entry, EntryKind,
    FileIssued,
};
use super::recover::recover;
use crate::build::is_same_file;
use crate::check::{check_with, CheckReport, CheckRequest};
use crate::description::{read_set_settings, SavedSet};
use crate::error::{Error, ErrorCode};
use crate::interface::Interface;
use crate::reply::Reply;
use crate::set::{set_of_dwg, SET_SETTINGS_FILE};
use crate::settings::Settings;
use crate::stamp::{fingerprint, SourceKind};

/// A request to accept a drawing set's working revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
    /// Accept even when checks fail, once the caller has seen them.
    #[serde(default)]
    pub despite_failed_checks: bool,
    /// The journal's newest entry the caller has looked at, 0 for an empty
    /// journal: the accept is refused if the journal holds a newer one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_seen: Option<u64>,
    /// Report what the accept would do, and write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// What an accept did, or would do in a dry run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AcceptReport {
    /// The drawing set's folder.
    pub set: String,
    /// Whether this was a dry run, which wrote nothing.
    pub dry_run: bool,
    /// The journal entry that records the accept; absent for a dry run and
    /// for an accept that issued nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_entry: Option<u64>,
    /// The interface the accept came through.
    pub via: Interface,
    /// What the accept did, in words.
    pub message: String,
    /// The change sets it issues, the oldest first: every one still in
    /// place that no earlier accept issued.
    pub issued: Vec<u64>,
    /// The sheets those change sets landed on, in the set's order.
    pub sheets: Vec<String>,
    /// The accept before it, whose revision this one follows, if there is
    /// one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follows: Option<u64>,
    /// The checker's report on every sheet of the set, read from its saved
    /// files: each failed check under `findings`.
    pub checks: CheckReport,
    /// Each file of the set as the accept issues it: its path, relative to
    /// the set's folder, and the SHA-256 of its bytes.
    pub revision: Vec<FileIssued>,
}

/// Accepts a drawing set's working revision, or reports what that would do
/// in a dry run, reading fonts from the font folders `settings` names to
/// check its sheets. `via` is the interface the accept came through, which
/// the journal records.
///
/// The reply is `ok` with the report once the accept is recorded, or for a
/// dry run that would be; not `ok`, with the report and a `checks_failed`
/// error, when checks fail and `despite_failed_checks` was not given; and
/// not `ok`, with only an error, when the accept is refused: the set is
/// inconsistent, the journal has moved on past `newest_seen`, there is
/// nothing to accept, or a sheet cannot be read. An entry a commit that
/// never finished left pending is rolled back first, and the reply says so
/// under `rolled_back`.
pub fn accept_with(
    request: &AcceptRequest,
    settings: &Settings,
    via: Interface,
) -> Reply<AcceptReport> {
    let rolled_back = match recover(&request.set) {
        Ok(rolled_back) => rolled_back,
        Err(error) => return Reply::from(Err(error)),
    };
    let reply = match accept_set(request, settings, via) {
        Ok(Accepted::Done(report)) => Reply::from(Ok(report)),
        Ok(Accepted::ChecksFailed(report, error)) => Reply::partly(report, error),
        Err(error) => Reply::from(Err(error)),
    };
    reply.after(rolled_back)
}

/// An accept's outcome short of an error.
enum Accepted {
    Done(AcceptReport),
    /// Checks failed and the caller did not accept despite them: nothing
    /// was issued.
    ChecksFailed(AcceptReport, Error),
}

fn accept_set(
    request: &AcceptRequest,
    settings: &Settings,
    via: Interface,
) -> Result<Accepted, Error> {
    let folder = &request.set;
    read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let entries = read_entries(folder)?;
    let newest = entries.last().map_or(0, |e| e.entry);
    if let Some(seen) = request.newest_seen {
        if seen != newest {
            return Err(moved_on(&entries, seen, newest));
        }
    }
    if let Some(mark) = inconsistent(folder) {
        return Err(Error::new(
            ErrorCode::Inconsistent,
            format!(
                "the set cannot be accepted while it is inconsistent: {}; nothing was issued",
                mark.message
            ),
        ));
    }

    let fates = fates(&entries);
    let issued: Vec<&Entry> = in_place(&entries, &fates)
        .filter(|e| fates.get(&e.entry).is_none_or(|f| f.issued_by.is_none()))
        .collect();
    let numbers: Vec<u64> = issued.iter().map(|e| e.entry).collect();
    let set = SavedSet::read(folder)?;
    let landed: BTreeSet<String> = issued
        .iter()
        .flat_map(|e| e.landed_on.iter().map(|s| s.to_uppercase()))
        .collect();
    let sheets: Vec<String> = set
        .names()
        .into_iter()
        .filter(|s| landed.contains(&s.to_uppercase()))
        .map(str::to_owned)
        .collect();
    let revision = revision(folder, &set)?;

    let last_accept = entries
        .iter()
        .rev()
        .find(|e| e.kind == EntryKind::Accept && e.finished());
    if numbers.is_empty() {
        if let Some(last) = last_accept.filter(|e| same_files(&e.revision, &revision)) {
            return Err(Error::new(
                ErrorCode::NothingToAccept,
                format!(
                    "there is nothing to accept: no change set has landed since the accept in \
                     entry {}, and every file of the set is as it issued it; nothing was issued",
                    last.entry
                ),
            ));
        }
    }

    let checks = check_with(
        &CheckRequest {
            source: folder.clone(),
            sheets: Vec::new(),
        },
        settings,
    )?;
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
    let mut report = AcceptReport {
        set: folder.display().to_string(),
        dry_run: request.dry_run,
        journal_entry: None,
        via,
        message: String::new(),
        issued: numbers.clone(),
        sheets,
        follows: last_accept.map(|e| e.entry),
        checks,
        revision,
    };
    if !failed.is_empty() && !request.despite_failed_checks {
        report.message = format!(
            "{}; nothing was issued. Accept again with despite_failed_checks to accept the set \
             despite them",
            failed_words(&failed)
        );
        let error = Error::new(ErrorCode::ChecksFailed, report.message.clone());
        return Ok(Accepted::ChecksFailed(report, error));
    }
    let what = issues_words(&report);
    let despite = if failed.is_empty() {
        "Every check passes.".to_owned()
    } else {
        format!("It was accepted despite {}.", failed_words(&failed))
    };
    if request.dry_run {
        report.message =
            format!("a dry run, which wrote nothing: the accept would issue {what}. {despite}");
        return Ok(Accepted::Done(report));
    }
    let number = commit_accept(
        folder,
        newest,
        Accepting {
            via,
            issued: numbers,
            failed_checks: failed,
            revision: report.revision.clone(),
        },
    )?;
    report.journal_entry = Some(number);
    report.message = format!(
        "accepted: issued {what}, recorded in entry {number}; an issued change set cannot be \
         undone. {despite}"
    );
    Ok(Accepted::Done(report))
}

/// What an accept issues, in words.
fn issues_words(report: &AcceptReport) -> String {
    let files = match report.revision.len() {
        1 => "1 file".to_owned(),
        n => format!("{n} files"),
    };
    let since = report
        .follows
        .map_or_else(String::new, |a| format!(" since the accept in entry {a}"));
    match report.issued.as_slice() {
        [] => format!("the set as it stands, its {files}, which no change set has changed{since}"),
        [one] => format!(
            "change set {one}, which landed on {}, and the set's {files}",
            sheets_words(&report.sheets)
        ),
        more => format!(
            "change sets {}, which landed on {}, and the set's {files}",
            listed(more),
            sheets_words(&report.sheets)
        ),
    }
}

fn sheets_words(sheets: &[String]) -> String {
    let quoted: Vec<String> = sheets.iter().map(|s| format!("`{s}`")).collect();
    match quoted.as_slice() {
        [] => "no sheet".to_owned(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The failed checks, in words.
fn failed_words(failed: &[CheckFailed]) -> String {
    let count = match failed.len() {
        1 => "1 failed check".to_owned(),
        n => format!("{n} failed checks"),
    };
    let each: Vec<String> = failed
        .iter()
        .map(|f| format!("{} on `{}`: {}", f.rule, f.sheet, f.message))
        .collect();
    format!("{count}: {}", each.join("; "))
}

/// The error for a journal that has moved on past the entry the caller saw.
fn moved_on(entries: &[Entry], seen: u64, newest: u64) -> Error {
    let since: Vec<String> = entries
        .iter()
        .filter(|e| e.entry > seen)
        .map(|e| {
            let kind = match e.kind {
                EntryKind::ChangeSet => "change set",
                EntryKind::FailedAttempt => "failed attempt",
                EntryKind::Undo => "undo",
                EntryKind::Accept => "accept",
            };
            format!("{kind} {}", e.entry)
        })
        .collect();
    let message = if seen > newest {
        format!(
            "the journal's newest entry is {newest}, not {seen}: look at the journal as it is, \
             then accept again; nothing was issued"
        )
    } else {
        format!(
            "the journal has moved on since entry {seen}, the newest the accept was asked to \
             follow: {} came since. Look at them, then accept again; nothing was issued",
            since.join(", ")
        )
    };
    Error::new(ErrorCode::MovedOn, message).at_field("newest_seen")
}

/// Whether two revisions hold the same files with the same bytes.
fn same_files(a: &[FileIssued], b: &[FileIssued]) -> bool {
    let set = |files: &[FileIssued]| -> BTreeSet<(String, String)> {
        files
            .iter()
            .map(|f| (f.path.clone(), f.sha256.clone()))
            .collect()
    };
    set(a) == set(b)
}

/// Each file of the set's working revision, with its fingerprint: its
/// settings, its rows, each template its rows name and the block library
/// each names, and the DWG of each sheet built into its output folder.
fn revision(folder: &Path, set: &SavedSet) -> Result<Vec<FileIssued>, Error> {
    let settings = read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let mut paths: Vec<PathBuf> = vec![folder.join(SET_SETTINGS_FILE), folder.join(&settings.rows)];
    for row in &set.rows {
        let record = set.sheet(row)?;
        paths.push(record.file.clone());
        if let Some(library) = &record.sheet.block_library {
            let beside = record.file.parent().unwrap_or(Path::new(""));
            paths.push(beside.join(library));
        }
    }
    for row in &set.rows {
        let dwg = folder
            .join(&settings.output)
            .join(format!("{}.dwg", row.sheet));
        if dwg.is_file() {
            paths.push(dwg);
        }
    }
    let mut revision: Vec<FileIssued> = Vec::new();
    for path in paths {
        let relative = relative_to(folder, &path);
        if revision.iter().any(|f| f.path == relative) {
            continue;
        }
        let bytes = fs::read(&path).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("could not read a file of the set to issue it: {e}"),
            )
            .in_file(&path)
        })?;
        revision.push(FileIssued {
            path: relative,
            sha256: fingerprint(&bytes),
        });
    }
    Ok(revision)
}

/// `path`, which is inside the set's `folder` or written from it, relative
/// to it, with `.` and `..` worked out as written and `/` between folders,
/// such as `blocks.yaml` for `folder/templates/../blocks.yaml` and
/// `../issued/A.dwg` for `folder/../issued/A.dwg`.
///
/// This is the one rule an accept names the set's files by and a stamp
/// looks them up by. It gives the same name whether `path` still holds its
/// `..`, as a path joined onto the folder does on most systems, or the join
/// already took it up, as joining onto a Windows `\\?\` path does, since
/// both paths are worked out before one is taken from the other.
pub(crate) fn relative_to(folder: &Path, path: &Path) -> String {
    let (folder, folder_roots) = as_written(folder);
    let (path, path_roots) = as_written(path);
    let shared = folder.iter().zip(&path).take_while(|(a, b)| a == b).count();
    let up = &folder[shared..];
    if folder_roots != path_roots || shared < folder_roots || up.iter().any(|p| p == "..") {
        // No way from the folder to the path as written: the path itself.
        return path.join("/");
    }
    let mut parts: Vec<&str> = vec![".."; up.len()];
    parts.extend(path[shared..].iter().map(String::as_str));
    parts.join("/")
}

/// The parts of `path`, with `.` and `..` worked out as written, and how
/// many of them at the start are its root: a drive or a share on Windows,
/// and `/`. A `..` takes off the folder before it, if there is one, and
/// stays when there is none, except at the root.
fn as_written(path: &Path) -> (Vec<String>, usize) {
    let mut parts: Vec<String> = Vec::new();
    let mut roots = 0;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => {
                parts.push(component.as_os_str().to_string_lossy().into_owned());
                roots += 1;
            }
            Component::ParentDir => {
                if parts.len() > roots && parts.last().is_some_and(|p| p != "..") {
                    parts.pop();
                } else if roots == 0 {
                    parts.push("..".to_owned());
                }
            }
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
        }
    }
    (parts, roots)
}

/// A check's rule as JSON writes it, such as `off_sheet`.
fn rule_text(rule: crate::check::Rule) -> String {
    match serde_json::to_value(rule) {
        Ok(serde_json::Value::String(text)) => text,
        _ => format!("{rule:?}"),
    }
}

/// What an output's stamp says of the revision it was made from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StampRevision {
    /// Whether every file the output was made from is, byte for byte, a
    /// file of a revision an accept issued.
    pub issued: bool,
    /// The accept that issued it: its entry in the set's journal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<u64>,
    /// When the accept was recorded, in UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<String>,
    /// The interface the accept came through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<Interface>,
    /// Why an output drawn from a DWG on its own cannot be from an issued
    /// revision, when the drawing set the DWG's build mark names cannot be
    /// found: the DWG names no set, the set is not there, or it is no
    /// longer the set the DWG was built from. None otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl StampRevision {
    /// Not issued, and why when there is more to say than that no accept
    /// issued the files.
    fn not_issued(why: Option<String>) -> StampRevision {
        StampRevision {
            issued: false,
            accept: None,
            accepted_at: None,
            via: None,
            why,
        }
    }
}

/// What a stamp says of the revision files were read from, each with its
/// kind, the record first: issued, by the newest accept of their drawing
/// set whose revision holds every one of them with the bytes read, or not
/// issued.
///
/// A DWG drawn on its own names its set in its build mark, as Zach decided
/// on 2026-11-11, so its set is found wherever its output folder is
/// ([`set_of_dwg`]), and when it cannot be, the revision says why. For any
/// other record, the set is the nearest folder above it that holds a set's
/// settings.
pub(crate) fn stamp_revision(files: &[(SourceKind, &Path, &[u8])]) -> StampRevision {
    match files {
        [(SourceKind::Dwg, dwg, bytes)] => dwg_revision(dwg, bytes),
        _ => files_revision(files),
    }
}

/// What a stamp says of the revision the DWG at `dwg` came from, its bytes
/// as read: issued when the set its build mark names is still the set it
/// was built from, and an accept of the set issued the DWG, at its place in
/// the set's output folder, with these bytes.
fn dwg_revision(dwg: &Path, bytes: &[u8]) -> StampRevision {
    let set = match set_of_dwg(dwg) {
        Ok(set) => set,
        Err(why) => return StampRevision::not_issued(Some(why)),
    };
    let Ok(entries) = read_entries(&set.folder) else {
        return StampRevision::not_issued(Some(
            "the journal of the drawing set the DWG's build mark names cannot be read, so no \
             accept of the set can be found"
                .to_owned(),
        ));
    };
    // The newest accept that issued this file with these bytes, the file
    // named in it as the DWG is named, or by another name of the same file
    // as the file system tells it, such as `dwg/DEMO-001.dwg` for
    // `demo-001.dwg` where letter case is ignored. Each accept is asked in
    // turn, newest first, so no older accept's name for it shadows a newer.
    let print = fingerprint(bytes);
    let accept = entries.iter().rev().find(|e| {
        e.kind == EntryKind::Accept
            && e.finished()
            && e.revision.iter().any(|f| {
                f.sha256 == print
                    && (f.path == set.path || is_same_file(&set.folder.join(&f.path), dwg))
            })
    });
    issued(accept)
}

/// What a stamp says of the revision a sheet description or a template
/// and the files it draws on came from, their set the nearest folder above
/// the first.
fn files_revision(files: &[(SourceKind, &Path, &[u8])]) -> StampRevision {
    let not_issued = StampRevision::not_issued(None);
    let Some(&(_, first, _)) = files.first() else {
        return not_issued;
    };
    let Some(folder) = set_folder_of(first) else {
        return not_issued;
    };
    let Ok(entries) = read_entries(&folder) else {
        return not_issued;
    };
    let mut read: Vec<(String, String)> = Vec::with_capacity(files.len());
    for &(_, path, bytes) in files {
        let Ok(path) = path.canonicalize() else {
            return not_issued;
        };
        let Ok(inside) = path.strip_prefix(&folder) else {
            return not_issued;
        };
        read.push((relative_to(Path::new(""), inside), fingerprint(bytes)));
    }
    issued_by(&entries, &read)
}

/// Issued by the newest finished accept among `entries` whose revision
/// holds every file `read` lists, by its path in the set and its
/// fingerprint; not issued when none does.
fn issued_by(entries: &[Entry], read: &[(String, String)]) -> StampRevision {
    let accept = entries.iter().rev().find(|e| {
        e.kind == EntryKind::Accept
            && e.finished()
            && read.iter().all(|(path, print)| {
                e.revision
                    .iter()
                    .any(|f| &f.path == path && &f.sha256 == print)
            })
    });
    issued(accept)
}

/// Issued by `accept`, when there is one; otherwise not issued.
fn issued(accept: Option<&Entry>) -> StampRevision {
    match accept {
        Some(accept) => StampRevision {
            issued: true,
            accept: Some(accept.entry),
            accepted_at: Some(accept.at.clone()),
            via: accept.via,
            why: None,
        },
        None => StampRevision::not_issued(None),
    }
}

/// The drawing set a file belongs to: the nearest folder above it, the
/// file's own path resolved, that holds a set's settings. None for a file
/// in no set.
fn set_folder_of(file: &Path) -> Option<PathBuf> {
    let file = file.canonicalize().ok()?;
    file.ancestors()
        .skip(1)
        .find(|folder| folder.join(SET_SETTINGS_FILE).is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_named_from_the_set_as_written() {
        let set = Path::new("plant");
        assert_eq!(relative_to(set, &set.join("rows.yaml")), "rows.yaml");
        assert_eq!(
            relative_to(set, &set.join("templates").join("..").join("blocks.yaml")),
            "blocks.yaml"
        );
        assert_eq!(
            relative_to(set, &set.join(".").join("dwg").join("A.dwg")),
            "dwg/A.dwg"
        );
        assert_eq!(
            relative_to(set, &set.join("..").join("shared.yaml")),
            "../shared.yaml"
        );
    }

    /// A path whose `..` the join already took up, as joining onto a
    /// Windows `\\?\` path does, is named as the same path with its `..`.
    #[test]
    fn a_path_outside_the_set_is_named_alike_however_it_was_joined() {
        let set = Path::new("/jobs/demo-set");
        for joined in [
            set.join("..").join("issued").join("DEMO-001.dwg"),
            PathBuf::from("/jobs/issued/DEMO-001.dwg"),
        ] {
            assert_eq!(relative_to(set, &joined), "../issued/DEMO-001.dwg");
        }
        assert_eq!(
            relative_to(set, Path::new("/jobs/demo-set/dwg/DEMO-001.dwg")),
            "dwg/DEMO-001.dwg"
        );
        assert_eq!(
            relative_to(Path::new("/"), Path::new("/../x.yaml")),
            "x.yaml"
        );
        assert_eq!(
            relative_to(Path::new(""), Path::new("dwg/A.dwg")),
            "dwg/A.dwg"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_set_given_as_a_verbatim_path_names_its_files_as_any_other() {
        let set = Path::new(r"\\?\C:\jobs\demo-set");
        // Joined onto a `\\?\` path, `..` is taken up at once.
        let dwg = set.join("../issued").join("DEMO-001.dwg");
        assert_eq!(dwg, Path::new(r"\\?\C:\jobs\issued\DEMO-001.dwg"));
        assert_eq!(relative_to(set, &dwg), "../issued/DEMO-001.dwg");
        assert_eq!(
            relative_to(set, &set.join("../outside/ansi-b.yaml")),
            "../outside/ansi-b.yaml"
        );
        assert_eq!(
            relative_to(set, &set.join("dwg").join("DEMO-001.dwg")),
            "dwg/DEMO-001.dwg"
        );
    }
}
