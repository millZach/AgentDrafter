//! The journal: the record, in a drawing set's own folder, of every change
//! set and every failed attempt, which a later session reads to see what
//! was done and to undo it.
//!
//! The journal is the file [`JOURNAL_FILE`], in JSON lines: one entry per
//! line, each a JSON object, appended in order and never rewritten, so an
//! agent reads it a line at a time. A change set's entry is added in the
//! same all-or-nothing step as its files (`commit.rs`), so a change set
//! lands only with its entry. A failed attempt changed nothing, and its
//! entry is added after it.
//!
//! An undo state is the bytes a change set's undo needs: each file it wrote,
//! as it was before. The entry holds each file's SHA-256 before and after;
//! the bytes before are kept once each, named by their SHA-256, in the
//! folder [`UNDO_STATES_FOLDER`]. Zach decided on 2026-11-11 that the undo
//! states of the [`KEPT_UNDO_STATES`] newest change sets are kept: when a
//! change set lands, the undo state of every change set it leaves outside
//! the newest 50 is pruned, and its entry names them under `pruned`. The
//! journal still records each pruned change set. A change set that has been
//! undone needs its undo state no more, so it is pruned too.
//!
//! Nothing here depends on git, or on the set's folder being a repository.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::commit::{commit, Append, Replacement};
use super::{inconsistent, ChangeReport, ChangeRequest, Inconsistent};
use crate::description::read_set_settings;
use crate::error::{Error, ErrorCode};
use crate::reply::Reply;
use crate::set::SET_SETTINGS_FILE;
use crate::stamp::fingerprint;

/// The journal's file, relative to a drawing set's folder, beside the
/// set's inconsistent mark.
pub const JOURNAL_FILE: &str = ".agentdrafter/journal.jsonl";

/// The folder, relative to a drawing set's folder, that holds the undo
/// states: each file's bytes before a change set wrote it, named by their
/// SHA-256.
pub const UNDO_STATES_FOLDER: &str = ".agentdrafter/undo-states";

/// How many of the newest change sets keep their undo state, as Zach
/// decided on 2026-11-11. An older change set's undo state is pruned.
pub const KEPT_UNDO_STATES: usize = 50;

/// What a journal entry records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A change set that landed on at least one sheet, or touched none.
    ChangeSet,
    /// A change set that changed nothing: one refused whole, one that
    /// failed on every sheet it touched, or one whose files could not be
    /// written.
    FailedAttempt,
    /// An undo of change sets, the newest first.
    Undo,
}

/// One entry of the journal, as its line holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Its number, counted from 1 in the order entries were added. A change
    /// set is undone by its entry's number.
    pub entry: u64,
    pub kind: EntryKind,
    /// When it was added, in UTC, such as `2026-11-11T18:57:46Z`.
    pub at: String,
    /// The change set as it was sent, with its operations; for a change set
    /// and a failed attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_set: Option<Value>,
    /// The sheets the change set landed on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub landed_on: Vec<String>,
    /// The sheets it failed on, each with why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_on: Vec<SheetFailed>,
    /// Why a failed attempt that was refused whole, or could not be
    /// written, did nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Refusal>,
    /// The change sets an undo undid, the newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undid: Vec<u64>,
    /// Each file the change set or the undo wrote, with its state before
    /// and after.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileState>,
    /// The folders the change set made for its files, relative to the set's
    /// folder, which its undo takes away when they are empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folders_made: Vec<String>,
    /// The change sets whose undo states were pruned when this one landed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pruned: Vec<u64>,
}

/// One sheet a change set failed on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetFailed {
    pub sheet: String,
    /// The error code of why it failed, such as `unknown_name`.
    pub code: String,
    pub message: String,
}

/// Why a failed attempt did nothing: the error its reply gave.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

/// One file an entry wrote: its path, relative to the set's folder with `/`
/// between folders, and the SHA-256 of its bytes before and after, `null`
/// where there was no file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Whether a change set can still be undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoState {
    /// Its undo state is kept: it can be undone, with every later change
    /// set.
    Kept,
    /// Its undo state was pruned, since it fell outside the newest change
    /// sets: it cannot be undone.
    Pruned,
    /// It has been undone.
    Undone,
}

/// A request to read a drawing set's journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
}

/// A drawing set's journal, as read.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JournalReport {
    /// The drawing set's folder.
    pub set: String,
    /// The journal's file.
    pub file: String,
    /// What the journal holds, in words.
    pub message: String,
    /// Every entry, the oldest first.
    pub entries: Vec<JournalEntry>,
    /// The change sets an undo can take now, the oldest first. Undoing one
    /// undoes every later one too.
    pub can_undo: Vec<u64>,
    /// How many of the newest change sets keep their undo state.
    pub kept_undo_states: usize,
    /// The set's inconsistent mark, if it is marked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inconsistent: Option<Inconsistent>,
}

/// One entry of the journal, with what has become of it since.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JournalEntry {
    #[serde(flatten)]
    pub recorded: Entry,
    /// For a change set, whether it can still be undone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_state: Option<UndoState>,
    /// The undo's entry, for a change set that has been undone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undone_by: Option<u64>,
    /// The entry of the change set whose landing pruned its undo state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pruned_by: Option<u64>,
}

/// Reads a drawing set's journal: every change set and failed attempt, and
/// every undo, with which change sets can still be undone.
pub fn journal(request: &JournalRequest) -> Reply<JournalReport> {
    Reply::from(read_journal(&request.set))
}

fn read_journal(folder: &Path) -> Result<JournalReport, Error> {
    read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let entries = read_entries(folder)?;
    let fates = fates(&entries);
    let can_undo: Vec<u64> = entries
        .iter()
        .filter(|e| keeps_state(e, &fates))
        .map(|e| e.entry)
        .collect();
    let count = |kind: EntryKind| entries.iter().filter(|e| e.kind == kind).count();
    let message = if entries.is_empty() {
        "the journal is empty: no change set has been applied to the set".to_owned()
    } else {
        format!(
            "the journal records {}, {} and {}. {}",
            counted(count(EntryKind::ChangeSet), "change set"),
            counted(count(EntryKind::FailedAttempt), "failed attempt"),
            counted(count(EntryKind::Undo), "undo"),
            match can_undo.as_slice() {
                [] => "No change set can be undone".to_owned(),
                [one] => format!("Change set {one} can be undone"),
                more => format!(
                    "Change sets {} can be undone, each with every later one",
                    listed(more)
                ),
            }
        )
    };
    let entries = entries
        .into_iter()
        .map(|recorded| {
            let fate = fates.get(&recorded.entry).copied().unwrap_or_default();
            let undo_state = (recorded.kind == EntryKind::ChangeSet).then(|| {
                if fate.undone_by.is_some() {
                    UndoState::Undone
                } else if fate.pruned_by.is_some() {
                    UndoState::Pruned
                } else {
                    UndoState::Kept
                }
            });
            JournalEntry {
                recorded,
                undo_state,
                undone_by: fate.undone_by,
                pruned_by: fate.pruned_by,
            }
        })
        .collect();
    Ok(JournalReport {
        set: folder.display().to_string(),
        file: folder.join(JOURNAL_FILE).display().to_string(),
        message,
        entries,
        can_undo,
        kept_undo_states: KEPT_UNDO_STATES,
        inconsistent: inconsistent(folder),
    })
}

/// `numbers` joined with commas and `and`.
pub(super) fn listed(numbers: &[u64]) -> String {
    let texts: Vec<String> = numbers.iter().map(u64::to_string).collect();
    match texts.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `n` and `thing`, made plural when `n` is not 1.
fn counted(n: usize, thing: &str) -> String {
    if n == 1 {
        format!("1 {thing}")
    } else {
        format!("{n} {thing}s")
    }
}

/// What became of a change set after it landed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Fate {
    pub undone_by: Option<u64>,
    pub pruned_by: Option<u64>,
}

/// What became of each change set, by its entry.
pub(super) fn fates(entries: &[Entry]) -> BTreeMap<u64, Fate> {
    let mut fates: BTreeMap<u64, Fate> = BTreeMap::new();
    for entry in entries {
        for &undone in &entry.undid {
            fates.entry(undone).or_default().undone_by = Some(entry.entry);
        }
        for &pruned in &entry.pruned {
            fates.entry(pruned).or_default().pruned_by = Some(entry.entry);
        }
    }
    fates
}

/// Whether `entry` is a change set that still keeps its undo state.
pub(super) fn keeps_state(entry: &Entry, fates: &BTreeMap<u64, Fate>) -> bool {
    entry.kind == EntryKind::ChangeSet
        && fates
            .get(&entry.entry)
            .is_none_or(|f| f.undone_by.is_none() && f.pruned_by.is_none())
}

/// The journal's entries, the oldest first; none when there is no journal.
pub(super) fn read_entries(folder: &Path) -> Result<Vec<Entry>, Error> {
    Ok(read_text(folder)?.map(|t| t.1).unwrap_or_default())
}

/// The journal's text and its entries, or `None` when there is no journal.
fn read_text(folder: &Path) -> Result<Option<(String, Vec<Entry>)>, Error> {
    let path = folder.join(JOURNAL_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the set's journal: {e}"),
            )
            .in_file(&path))
        }
    };
    let mut entries = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: Entry = serde_json::from_str(line).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "line {} of the set's journal is not a journal entry: {e}",
                    i + 1
                ),
            )
            .at_position(i + 1, e.column().max(1))
            .in_file(&path)
        })?;
        entries.push(entry);
    }
    Ok(Some((text, entries)))
}

/// The entry to add to the journal: its line, after a line break when the
/// journal's text does not end with one.
fn append_of(folder: &Path, text: Option<&str>, entry: &Entry) -> Append {
    // An entry holds only strings, numbers, lists and string-keyed maps,
    // which always serialize, and JSON escapes every line break in them.
    let mut line = serde_json::to_string(entry).expect("an entry serializes to JSON");
    line.push('\n');
    if text.is_some_and(|t| !t.is_empty() && !t.ends_with('\n')) {
        line.insert(0, '\n');
    }
    Append {
        path: folder.join(JOURNAL_FILE),
        bytes: line.into_bytes(),
        what: "the set's journal",
    }
}

/// The number the next entry takes.
fn next_number(entries: &[Entry]) -> u64 {
    entries.iter().map(|e| e.entry).max().unwrap_or(0) + 1
}

/// What a change set did to its sheets, for its entry.
pub(super) struct Landing {
    pub landed_on: Vec<String>,
    pub failed_on: Vec<SheetFailed>,
}

/// Puts a change set's files in place with its undo state and its entry
/// in the journal, or none of it, and gives the entry's number. The undo
/// states of the change sets it leaves outside the newest
/// [`KEPT_UNDO_STATES`] are then pruned.
pub(super) fn commit_change_set(
    folder: &Path,
    change_set: &Value,
    landing: Landing,
    mut replacements: Vec<Replacement>,
) -> Result<u64, Error> {
    let (text, entries_before) = read_text(folder)?.unzip();
    let entries_before = entries_before.unwrap_or_default();
    let number = next_number(&entries_before);

    let mut files = Vec::with_capacity(replacements.len());
    let mut states: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for replacement in &replacements {
        let before = read_state(&replacement.path)?;
        let before_print = before.as_deref().map(fingerprint);
        if let (Some(print), Some(bytes)) = (&before_print, before) {
            states.entry(print.clone()).or_insert(bytes);
        }
        files.push(FileState {
            path: relative(folder, &replacement.path),
            before: before_print,
            after: replacement.bytes.as_deref().map(fingerprint),
        });
    }
    let folders_made = folders_made(folder, &replacements);
    let store = folder.join(UNDO_STATES_FOLDER);
    for (print, bytes) in states {
        let path = store.join(&print);
        if !path.is_file() {
            replacements.push(Replacement {
                path,
                bytes: Some(bytes),
                what: "an undo state",
            });
        }
    }

    let fates = fates(&entries_before);
    let change_sets: Vec<&Entry> = entries_before
        .iter()
        .filter(|e| e.kind == EntryKind::ChangeSet)
        .collect();
    // With this one, the newest KEPT_UNDO_STATES keep theirs.
    let outside = (change_sets.len() + 1).saturating_sub(KEPT_UNDO_STATES);
    let pruned: Vec<u64> = change_sets[..outside]
        .iter()
        .filter(|e| keeps_state(e, &fates))
        .map(|e| e.entry)
        .collect();

    let entry = Entry {
        entry: number,
        kind: EntryKind::ChangeSet,
        at: now(),
        change_set: Some(change_set.clone()),
        landed_on: landing.landed_on,
        failed_on: landing.failed_on,
        error: None,
        undid: Vec::new(),
        files,
        folders_made,
        pruned,
    };
    commit(
        &replacements,
        Some(&append_of(folder, text.as_deref(), &entry)),
    )?;
    let mut entries = entries_before;
    entries.push(entry);
    prune(folder, &entries);
    Ok(number)
}

/// Records a change set that changed nothing as a failed attempt, unless it
/// was a dry run, which is not recorded, or it landed and its entry is
/// recorded already. An attempt that cannot be recorded, such as on a
/// folder that is not a drawing set, is left unrecorded: it changed nothing.
pub(super) fn record_failed_attempt(
    request: &ChangeRequest,
    outcome: &mut Result<ChangeReport, Error>,
) {
    if request.dry_run {
        return;
    }
    let folder = &request.set;
    let (landing, error) = match outcome {
        Ok(report) if report.journal_entry.is_none() => (
            Landing {
                landed_on: Vec::new(),
                failed_on: report
                    .sheets
                    .iter()
                    .filter_map(|s| {
                        s.error.as_ref().map(|e| SheetFailed {
                            sheet: s.sheet.clone(),
                            code: code_text(e.code),
                            message: e.message.clone(),
                        })
                    })
                    .collect(),
            },
            None,
        ),
        Ok(_) => return,
        Err(error) => (
            Landing {
                landed_on: Vec::new(),
                failed_on: Vec::new(),
            },
            Some(Refusal {
                code: code_text(error.code),
                message: error.message.clone(),
                field: error.field.clone(),
            }),
        ),
    };
    if !folder.join(SET_SETTINGS_FILE).is_file() {
        return;
    }
    let Ok(read) = read_text(folder) else {
        return;
    };
    let (text, entries) = read.unzip();
    let number = next_number(entries.as_deref().unwrap_or_default());
    let entry = Entry {
        entry: number,
        kind: EntryKind::FailedAttempt,
        at: now(),
        change_set: Some(request.change_set.clone()),
        landed_on: landing.landed_on,
        failed_on: landing.failed_on,
        error,
        undid: Vec::new(),
        files: Vec::new(),
        folders_made: Vec::new(),
        pruned: Vec::new(),
    };
    if commit(&[], Some(&append_of(folder, text.as_deref(), &entry))).is_ok() {
        if let Ok(report) = outcome {
            report.journal_entry = Some(number);
        }
    }
}

/// Puts back the files of an undo, with the undo's entry in the journal, or
/// none of it, and gives the entry's number.
pub(super) fn commit_undo(
    folder: &Path,
    undid: Vec<u64>,
    files: Vec<FileState>,
    replacements: &[Replacement],
) -> Result<u64, Error> {
    let (text, entries) = read_text(folder)?.unzip();
    let mut entries = entries.unwrap_or_default();
    let number = next_number(&entries);
    let entry = Entry {
        entry: number,
        kind: EntryKind::Undo,
        at: now(),
        change_set: None,
        landed_on: Vec::new(),
        failed_on: Vec::new(),
        error: None,
        undid,
        files,
        folders_made: Vec::new(),
        pruned: Vec::new(),
    };
    commit(
        replacements,
        Some(&append_of(folder, text.as_deref(), &entry)),
    )?;
    entries.push(entry);
    prune(folder, &entries);
    Ok(number)
}

/// Takes away every undo state no change set that keeps its undo state
/// needs: those of change sets pruned or undone, and any file left in the
/// folder that is not an undo state. A file that cannot be taken away is
/// left; it is needed by nothing.
fn prune(folder: &Path, entries: &[Entry]) {
    let fates = fates(entries);
    let needed: BTreeSet<&str> = entries
        .iter()
        .filter(|e| keeps_state(e, &fates))
        .flat_map(|e| e.files.iter().filter_map(|f| f.before.as_deref()))
        .collect();
    let store = folder.join(UNDO_STATES_FOLDER);
    let Ok(listed) = fs::read_dir(&store) else {
        return;
    };
    for item in listed.flatten() {
        let name = item.file_name();
        if !name.to_str().is_some_and(|n| needed.contains(n)) {
            let _ = fs::remove_file(item.path());
        }
    }
    let _ = fs::remove_dir(&store);
}

/// The bytes of the file at `path`, or `None` when there is none.
pub(super) fn read_state(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the file to keep its undo state: {e}"),
        )
        .in_file(path)),
    }
}

/// The bytes before a change set of the file whose SHA-256 is `print`, from
/// the undo states, checked against it.
pub(super) fn undo_state(folder: &Path, print: &str) -> Option<Vec<u8>> {
    let bytes = fs::read(folder.join(UNDO_STATES_FOLDER).join(print)).ok()?;
    (fingerprint(&bytes) == print).then_some(bytes)
}

/// `path` relative to the set's `folder`, with `/` between folders.
fn relative(folder: &Path, path: &Path) -> String {
    let inside = path.strip_prefix(folder).unwrap_or(path);
    let parts: Vec<String> = inside
        .components()
        .map(|c| match c {
            Component::ParentDir => "..".to_owned(),
            Component::CurDir => ".".to_owned(),
            other => other.as_os_str().to_string_lossy().into_owned(),
        })
        .collect();
    parts.join("/")
}

/// The folders, relative to the set's folder, that putting `replacements`
/// in place makes, the deepest first. The set's own `.agentdrafter` folder
/// is not among them: the journal stays in it.
fn folders_made(folder: &Path, replacements: &[Replacement]) -> Vec<String> {
    let own = folder.join(".agentdrafter");
    let mut made: Vec<PathBuf> = Vec::new();
    for replacement in replacements.iter().filter(|r| r.bytes.is_some()) {
        let Some(parent) = replacement.path.parent() else {
            continue;
        };
        for missing in parent.ancestors().take_while(|a| !a.exists()) {
            if missing != own && !missing.starts_with(&own) && !made.iter().any(|m| m == missing) {
                made.push(missing.to_owned());
            }
        }
    }
    made.sort_by_key(|m| std::cmp::Reverse(m.components().count()));
    made.iter().map(|m| relative(folder, m)).collect()
}

/// An error code as its JSON writes it, such as `unknown_name`.
fn code_text(code: ErrorCode) -> String {
    match serde_json::to_value(code) {
        Ok(Value::String(text)) => text,
        _ => format!("{code:?}"),
    }
}

/// The time now in UTC, to the second, such as `2026-11-11T18:57:46Z`.
fn now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    utc(seconds)
}

/// The UTC time `seconds` after 1970 began, as `YYYY-MM-DDTHH:MM:SSZ`.
fn utc(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX / 2);
    let left = seconds % 86_400;
    // Howard Hinnant's days-to-civil, for the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        left / 3_600,
        left % 3_600 / 60,
        left % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_written_in_utc() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(19_723 * 86_400 + 3_661), "2024-01-01T01:01:01Z");
        assert_eq!(utc(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(utc(1_794_423_466), "2026-11-11T18:57:46Z");
    }

    #[test]
    fn an_entry_reads_back_as_written() {
        let entry = Entry {
            entry: 3,
            kind: EntryKind::ChangeSet,
            at: "2026-11-11T18:57:46Z".into(),
            change_set: Some(serde_json::json!({"operations": [{"op": "move"}]})),
            landed_on: vec!["DEMO-001".into()],
            failed_on: vec![SheetFailed {
                sheet: "DEMO-002".into(),
                code: "unknown_name".into(),
                message: "two\nlines".into(),
            }],
            error: None,
            undid: Vec::new(),
            files: vec![FileState {
                path: "dwg/DEMO-001.dwg".into(),
                before: None,
                after: Some("ab".into()),
            }],
            folders_made: vec!["dwg".into()],
            pruned: vec![1],
        };
        let append = append_of(Path::new("set"), Some("{}"), &entry);
        let text = String::from_utf8(append.bytes).unwrap();
        assert!(text.starts_with('\n') && text.ends_with('\n'));
        assert_eq!(text.trim().lines().count(), 1);
        let back: Entry = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(back, entry);
    }
}
