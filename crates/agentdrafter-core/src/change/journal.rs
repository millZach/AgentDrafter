//! The journal: the record, in a drawing set's own folder, of every change
//! set and every failed attempt, which a later session reads to see what
//! was done and to undo it.
//!
//! The journal is the file [`JOURNAL_FILE`], in JSON lines: one entry per
//! line, each a JSON object, appended in order and never rewritten, so an
//! agent reads it a line at a time. A change set's entry is written ahead of
//! its files, as Zach decided on 2026-11-11: added and synced, `pending`,
//! before any of them is put in place, and marked done by a line of its own
//! once all are (`commit.rs`), so a change set lands only with its entry.
//! An undo's and an accept's are written the same way. An entry a killed
//! process left pending is rolled back by the next command that reads the
//! set, and marked so by a line of its own (`recover.rs`). A failed attempt
//! changed nothing, and its entry is added after it, whole.
//!
//! Every entry is added holding the set's journal lock, [`LOCK_FILE`], so
//! that a command reading the set while another process commits waits for
//! that commit to finish rather than rolling it back.
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
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::commit::{add_line, commit, Ahead, Before, Replacement};
use super::{inconsistent, ChangeReport, ChangeRequest, Inconsistent};
use crate::description::read_set_settings;
use crate::error::{Error, ErrorCode};
use crate::interface::Interface;
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

/// The file, relative to a drawing set's folder, that a process holds
/// locked while it adds to the set's journal, and while it rolls back an
/// entry another left pending. It holds nothing.
pub const LOCK_FILE: &str = ".agentdrafter/journal.lock";

/// How long a command waits for another process to finish adding to the
/// set's journal before it gives up.
const LOCK_WAIT: Duration = Duration::from_secs(30);

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
    /// The reviewer's accept, or an agent's on their behalf: every change
    /// set still in place is issued, and can no longer be undone.
    Accept,
}

/// Where the commit of a change set, an undo or an accept, written ahead of
/// its files, got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitState {
    /// Written, before any of its files was put in place, and not yet
    /// marked done: its commit has not finished.
    Pending,
    /// Every file was put in place, and the entry marked done.
    Done,
    /// Its commit never finished, since the process writing it stopped, and
    /// a later command put every file back as it was before, byte for byte:
    /// it changed nothing.
    RolledBack,
}

/// A line of the journal that marks an earlier entry, written pending,
/// done or rolled back. It is no entry of its own and takes no number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateLine {
    /// The entry it marks.
    entry: u64,
    state: CommitState,
    /// When it was added, in UTC.
    at: String,
    /// For a rollback, each file it put back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileState>,
}

/// How an entry whose commit never finished was rolled back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RollBack {
    /// When, in UTC.
    pub at: String,
    /// Each file the rollback put back: its SHA-256 as the rollback found
    /// it, `before`, and as it put it back, `after`.
    pub files: Vec<FileState>,
}

/// One entry of the journal, as its line holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Its number, counted from 1 in the order entries were added. A change
    /// set is undone by its entry's number.
    pub entry: u64,
    pub kind: EntryKind,
    /// Where its commit got to: `pending` on the entry's own line, written
    /// before any of its files was put in place, and, as read, what a later
    /// line marks it, `done` or `rolled_back`. None for a failed attempt,
    /// and for an entry written before entries were written ahead, each of
    /// which was written whole once its files were in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<CommitState>,
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
    /// Each object a change set added, changed, moved or deleted, on each
    /// sheet it reached, so the viewer can show what it changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<ObjectChanged>,
    /// The interface an accept came through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<Interface>,
    /// The change sets an accept issued: every one still in place that an
    /// earlier accept had not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issued: Vec<u64>,
    /// The failed checks an accept was told to accept despite.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_checks: Vec<CheckFailed>,
    /// Each file of the set as an accept issued it: its path, relative to
    /// the set's folder, and the SHA-256 of its bytes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revision: Vec<FileIssued>,
    /// How it was rolled back, when it was: read from the line that marks
    /// it so, not from its own.
    #[serde(skip)]
    pub rolled_back: Option<RollBack>,
}

impl Entry {
    /// An entry of `kind` numbered `entry`, made now, with nothing else
    /// recorded yet.
    fn new(entry: u64, kind: EntryKind) -> Self {
        Self {
            entry,
            kind,
            state: None,
            at: now(),
            change_set: None,
            landed_on: Vec::new(),
            failed_on: Vec::new(),
            error: None,
            undid: Vec::new(),
            files: Vec::new(),
            folders_made: Vec::new(),
            pruned: Vec::new(),
            objects: Vec::new(),
            via: None,
            issued: Vec::new(),
            failed_checks: Vec::new(),
            revision: Vec::new(),
            rolled_back: None,
        }
    }

    /// Whether its commit finished: it was written whole, or marked done.
    /// An entry pending or rolled back changed nothing, as far as the set
    /// is concerned, and is passed over.
    pub fn finished(&self) -> bool {
        !matches!(
            self.state,
            Some(CommitState::Pending | CommitState::RolledBack)
        )
    }
}

/// One object a change set touched, by its ID on one sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectChanged {
    /// The object's ID, such as `DEMO-002/border`.
    pub id: String,
    /// What the change set did to it.
    pub how: HowChanged,
}

/// What a change set did to an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HowChanged {
    Added,
    Changed,
    Deleted,
}

/// A failed check an accept was told to accept despite, as the checker
/// found it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckFailed {
    pub sheet: String,
    /// The check, such as `off_sheet`.
    pub rule: String,
    /// The object that fails it, by its ID; none for a check of the whole
    /// sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub message: String,
}

/// One file of the set as an accept issued it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIssued {
    /// Its path, relative to the set's folder, with `/` between folders.
    pub path: String,
    /// The SHA-256 of its bytes.
    pub sha256: String,
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
    /// The accept that issued a change set, which can no longer be undone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_by: Option<u64>,
    /// How an entry whose commit never finished was rolled back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rolled_back: Option<RollBack>,
}

/// Reads a drawing set's journal: every change set and failed attempt, and
/// every undo, with which change sets can still be undone. An entry a
/// commit that never finished left pending is rolled back first, and the
/// reply says so under `rolled_back`.
pub fn journal(request: &JournalRequest) -> Reply<JournalReport> {
    match super::recover(&request.set) {
        Ok(rolled_back) => Reply::from(read_journal(&request.set)).after(rolled_back),
        Err(error) => Reply::from(Err(error)),
    }
}

fn read_journal(folder: &Path) -> Result<JournalReport, Error> {
    read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let entries = read_entries(folder)?;
    let fates = fates(&entries);
    let can_undo = undoable(&entries, &fates);
    let count = |kind: EntryKind| {
        entries
            .iter()
            .filter(|e| e.kind == kind && e.finished())
            .count()
    };
    let in_state = |state: CommitState| -> Vec<u64> {
        entries
            .iter()
            .filter(|e| e.state == Some(state))
            .map(|e| e.entry)
            .collect()
    };
    let rolled_back = in_state(CommitState::RolledBack);
    let pending = in_state(CommitState::Pending);
    let message = if entries.is_empty() {
        "the journal is empty: no change set has been applied to the set".to_owned()
    } else {
        let undos = counted(count(EntryKind::Undo), "undo");
        let accepts = count(EntryKind::Accept);
        let rest = if accepts == 0 {
            format!(" and {undos}")
        } else {
            format!(", {undos} and {}", counted(accepts, "accept"))
        };
        let mut rolled = match rolled_back.as_slice() {
            [] => String::new(),
            [one] => format!(
                "; entry {one} was rolled back, since its commit never finished, and changed \
                 nothing"
            ),
            more => format!(
                "; entries {} were rolled back, since their commits never finished, and changed \
                 nothing",
                listed(more)
            ),
        };
        if let Some(entry) = pending.first() {
            rolled.push_str(&format!(
                "; entry {entry} is pending: its commit has not finished"
            ));
        }
        format!(
            "the journal records {}, {}{rest}{rolled}. {}",
            counted(count(EntryKind::ChangeSet), "change set"),
            counted(count(EntryKind::FailedAttempt), "failed attempt"),
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
            let landed = recorded.kind == EntryKind::ChangeSet && recorded.finished();
            let undo_state = landed.then(|| {
                if fate.undone_by.is_some() {
                    UndoState::Undone
                } else if fate.pruned_by.is_some() {
                    UndoState::Pruned
                } else {
                    UndoState::Kept
                }
            });
            JournalEntry {
                rolled_back: recorded.rolled_back.clone(),
                recorded,
                undo_state,
                undone_by: fate.undone_by,
                pruned_by: fate.pruned_by,
                issued_by: fate.issued_by,
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
    pub issued_by: Option<u64>,
}

/// What became of each change set, by its entry. An entry whose commit
/// never finished did nothing to any.
pub(super) fn fates(entries: &[Entry]) -> BTreeMap<u64, Fate> {
    let mut fates: BTreeMap<u64, Fate> = BTreeMap::new();
    for entry in entries.iter().filter(|e| e.finished()) {
        for &undone in &entry.undid {
            fates.entry(undone).or_default().undone_by = Some(entry.entry);
        }
        for &pruned in &entry.pruned {
            fates.entry(pruned).or_default().pruned_by = Some(entry.entry);
        }
        for &issued in &entry.issued {
            fates.entry(issued).or_default().issued_by = Some(entry.entry);
        }
    }
    fates
}

/// The change sets an undo can take now, the oldest first: those that keep
/// their undo state and are not issued.
pub(super) fn undoable(entries: &[Entry], fates: &BTreeMap<u64, Fate>) -> Vec<u64> {
    entries
        .iter()
        .filter(|e| keeps_state(e, fates))
        .filter(|e| fates.get(&e.entry).is_none_or(|f| f.issued_by.is_none()))
        .map(|e| e.entry)
        .collect()
}

/// The change sets still in place, the oldest first: every change set that
/// landed and has not been undone.
pub(super) fn in_place<'a>(
    entries: &'a [Entry],
    fates: &'a BTreeMap<u64, Fate>,
) -> impl Iterator<Item = &'a Entry> + 'a {
    entries.iter().filter(move |e| {
        e.kind == EntryKind::ChangeSet
            && e.finished()
            && fates.get(&e.entry).is_none_or(|f| f.undone_by.is_none())
    })
}

/// Whether `entry` is a change set that landed and still keeps its undo
/// state.
pub(super) fn keeps_state(entry: &Entry, fates: &BTreeMap<u64, Fate>) -> bool {
    entry.kind == EntryKind::ChangeSet
        && entry.finished()
        && fates
            .get(&entry.entry)
            .is_none_or(|f| f.undone_by.is_none() && f.pruned_by.is_none())
}

/// The journal's entries, the oldest first; none when there is no journal.
pub(super) fn read_entries(folder: &Path) -> Result<Vec<Entry>, Error> {
    Ok(read_text(folder)?.map(|t| t.1).unwrap_or_default())
}

/// The journal's text and its entries, or `None` when there is no journal.
/// A last line written in part is refused, as any line that is not an
/// entry is: the next command cuts it off (`recover.rs`).
fn read_text(folder: &Path) -> Result<Option<(String, Vec<Entry>)>, Error> {
    let Some(read) = read_journal_file(folder)? else {
        return Ok(None);
    };
    if let Some(torn) = read.torn {
        let path = folder.join(JOURNAL_FILE);
        return Err(Error::new(
            ErrorCode::ReadFailed,
            format!(
                "line {} of the set's journal is not a journal entry: it was written only in \
                 part, by a process that stopped while it wrote it",
                torn.line
            ),
        )
        .at_position(torn.line, 1)
        .in_file(&path));
    }
    Ok(Some((read.text, read.entries)))
}

/// The journal as read, with the last line, if a process that stopped while
/// it wrote it left it in part, set apart.
pub(super) struct JournalRead {
    /// The journal's text, but a last line written in part.
    pub text: String,
    pub entries: Vec<Entry>,
    pub torn: Option<Torn>,
}

/// A last line of the journal written only in part: it ends with no line
/// break and is no JSON. Every line is written with its line break, so only
/// a write that stopped half-way leaves one, and only at the end.
#[derive(Clone, Copy)]
pub(super) struct Torn {
    /// Its number, counted from 1.
    pub line: usize,
    /// Where it begins, in bytes: the journal's length without it.
    pub at: u64,
}

/// The set's journal, or `None` when there is none. A line that is not an
/// entry, or marks an entry no line before it holds pending, is refused,
/// with its line, unless it is the last and was written only in part.
pub(super) fn read_journal_file(folder: &Path) -> Result<Option<JournalRead>, Error> {
    let path = folder.join(JOURNAL_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the set's journal: {e}"),
            )
            .in_file(&path))
        }
    };
    let split = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let (head, tail) = bytes.split_at(split);
    let mut text = String::from_utf8(head.to_vec()).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("the set's journal is not text: {e}"),
        )
        .in_file(&path)
    })?;
    let mut entries: Vec<Entry> = Vec::new();
    let mut lines = 0;
    for (i, line) in text.lines().enumerate() {
        lines = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        let read = parse_line(line).map_err(|e| not_entry(&path, i + 1, &e))?;
        mark(&mut entries, read, i + 1, &path)?;
    }
    let mut torn = None;
    if !tail.iter().all(u8::is_ascii_whitespace) {
        let line = lines + 1;
        match std::str::from_utf8(tail).ok().map(|t| (t, parse_line(t))) {
            Some((last, Ok(read))) => {
                mark(&mut entries, read, line, &path)?;
                text.push_str(last);
            }
            _ => {
                torn = Some(Torn {
                    line,
                    at: split as u64,
                })
            }
        }
    }
    Ok(Some(JournalRead {
        text,
        entries,
        torn,
    }))
}

/// One line of the journal: an entry, or the state it came to.
enum Line {
    Entry(Box<Entry>),
    State(StateLine),
}

fn parse_line(line: &str) -> Result<Line, serde_json::Error> {
    match serde_json::from_str::<Entry>(line) {
        Ok(entry) => Ok(Line::Entry(Box::new(entry))),
        Err(e) => serde_json::from_str(line).map(Line::State).map_err(|_| e),
    }
}

fn not_entry(path: &Path, line: usize, e: &serde_json::Error) -> Error {
    Error::new(
        ErrorCode::ReadFailed,
        format!("line {line} of the set's journal is not a journal entry: {e}"),
    )
    .at_position(line, e.column().max(1))
    .in_file(path)
}

/// Adds line `line` of the journal to `entries`: an entry, or the state an
/// entry before it, pending, came to.
fn mark(entries: &mut Vec<Entry>, read: Line, line: usize, path: &Path) -> Result<(), Error> {
    let state = match read {
        Line::Entry(entry) => {
            entries.push(*entry);
            return Ok(());
        }
        Line::State(state) => state,
    };
    let marked = entries
        .iter_mut()
        .rev()
        .find(|e| e.entry == state.entry)
        .filter(|e| e.state == Some(CommitState::Pending) && state.state != CommitState::Pending)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "line {line} of the set's journal marks entry {} {}, but no line before it \
                     holds entry {} pending",
                    state.entry,
                    state_words(state.state),
                    state.entry
                ),
            )
            .at_position(line, 1)
            .in_file(path)
        })?;
    marked.state = Some(state.state);
    if state.state == CommitState::RolledBack {
        marked.rolled_back = Some(RollBack {
            at: state.at,
            files: state.files,
        });
    }
    Ok(())
}

/// A commit's state, in words.
fn state_words(state: CommitState) -> &'static str {
    match state {
        CommitState::Pending => "pending",
        CommitState::Done => "done",
        CommitState::RolledBack => "rolled back",
    }
}

/// The entry a commit that never finished left pending, if there is one.
/// Only the journal's newest entry can be: every command rolls it back
/// before it adds another.
pub(super) fn pending(entries: &[Entry]) -> Option<&Entry> {
    entries
        .iter()
        .rev()
        .find(|e| e.state == Some(CommitState::Pending))
}

/// `item` as one line of the journal, after a line break when the
/// journal's text does not end with one.
fn line_of(text: Option<&str>, item: &impl Serialize) -> Vec<u8> {
    // An entry holds only strings, numbers, lists and string-keyed maps,
    // which always serialize, and JSON escapes every line break in them.
    let mut line = serde_json::to_string(item).expect("an entry serializes to JSON");
    line.push('\n');
    if text.is_some_and(|t| !t.is_empty() && !t.ends_with('\n')) {
        line.insert(0, '\n');
    }
    line.into_bytes()
}

/// The line that marks entry `entry` `state`, made now, with the files a
/// rollback put back, after a line break when `text`, the journal's, does
/// not end with one.
pub(super) fn state_line(
    text: Option<&str>,
    entry: u64,
    state: CommitState,
    files: Vec<FileState>,
) -> Vec<u8> {
    line_of(
        text,
        &StateLine {
            entry,
            state,
            at: now(),
            files,
        },
    )
}

/// Holds the journal lock of the set in `folder`, [`LOCK_FILE`], until the
/// file it gives is dropped, or, as for a killed process, closed. It waits
/// up to [`LOCK_WAIT`] for a process that holds it to let it go. It is
/// taken before anything is written, so when it cannot be, nothing was.
pub(super) fn lock(folder: &Path) -> Result<File, Error> {
    let path = folder.join(LOCK_FILE);
    let failed = |what: &str, why: String| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not {what} the set's journal lock: {why}; nothing was changed"),
        )
        .in_file(&path)
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| failed("make the folder of", e.to_string()))?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| failed("open", e.to_string()))?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if start.elapsed() < LOCK_WAIT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(TryLockError::WouldBlock) => {
                return Err(failed(
                    "take",
                    format!(
                        "another process has held it for {} seconds, adding to the journal",
                        LOCK_WAIT.as_secs()
                    ),
                ))
            }
            Err(TryLockError::Error(e)) => return Err(failed("take", e.to_string())),
        }
    }
}

/// The error for a commit that finds an entry another commit left pending,
/// which no command has rolled back yet.
fn left_pending(entry: &Entry) -> Error {
    Error::new(
        ErrorCode::RollbackFailed,
        format!(
            "entry {} of the set's journal is pending: the commit that wrote it never finished, \
             and it has not been rolled back; nothing was changed. The next command on the set \
             rolls it back",
            entry.entry
        ),
    )
}

/// The number the next entry takes.
fn next_number(entries: &[Entry]) -> u64 {
    entries.iter().map(|e| e.entry).max().unwrap_or(0) + 1
}

/// What a change set did to its sheets, for its entry.
#[derive(Clone)]
pub(super) struct Landing {
    pub landed_on: Vec<String>,
    pub failed_on: Vec<SheetFailed>,
    /// The objects it added, changed or deleted, on each sheet it landed on.
    pub objects: Vec<ObjectChanged>,
}

/// Puts a change set's files in place with its undo state and its entry
/// in the journal, or none of it, and gives the entry's number. The undo
/// states of the change sets it leaves outside the newest
/// [`KEPT_UNDO_STATES`] are then pruned.
pub(super) fn commit_change_set(
    folder: &Path,
    change_set: &Value,
    landing: Landing,
    replacements: Vec<Replacement>,
) -> Result<u64, Error> {
    let _lock = lock(folder)?;
    let (text, entries_before) = read_text(folder)?.unzip();
    let entries_before = entries_before.unwrap_or_default();
    if let Some(entry) = pending(&entries_before) {
        return Err(left_pending(entry));
    }
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
    // The undo state goes in place first, so that a commit stopped at any
    // step after it can be rolled back from it.
    let mut placing: Vec<Replacement> = states
        .into_iter()
        .map(|(print, bytes)| (store.join(print), bytes))
        .filter(|(path, _)| !path.is_file())
        .map(|(path, bytes)| Replacement {
            path,
            bytes: Some(bytes),
            what: "an undo state",
        })
        .collect();
    let before: Vec<Before> = replacements
        .iter()
        .zip(&files)
        .map(|(r, f)| Before {
            path: r.path.clone(),
            print: f.before.clone(),
        })
        .collect();
    placing.extend(replacements);

    let fates = fates(&entries_before);
    let change_sets: Vec<&Entry> = entries_before
        .iter()
        .filter(|e| e.kind == EntryKind::ChangeSet && e.finished())
        .collect();
    // With this one, the newest KEPT_UNDO_STATES keep theirs.
    let outside = (change_sets.len() + 1).saturating_sub(KEPT_UNDO_STATES);
    let pruned: Vec<u64> = change_sets[..outside]
        .iter()
        .filter(|e| keeps_state(e, &fates))
        .map(|e| e.entry)
        .collect();

    let made: Vec<PathBuf> = folders_made.iter().map(|f| folder.join(f)).collect();
    let entry = Entry {
        change_set: Some(change_set.clone()),
        landed_on: landing.landed_on,
        failed_on: landing.failed_on,
        files,
        folders_made,
        pruned,
        objects: landing.objects,
        ..Entry::new(number, EntryKind::ChangeSet)
    };
    let written = Written {
        text: text.as_deref(),
        entries: entries_before,
    };
    commit_entry(folder, written, entry, &before, &made, &placing)?;
    Ok(number)
}

/// The journal as a commit found it, holding the lock: its text and its
/// entries.
struct Written<'a> {
    text: Option<&'a str>,
    entries: Vec<Entry>,
}

/// Commits `entry`, written ahead of `replacements` (`commit.rs`), while
/// the caller holds the journal lock, and then prunes the undo states no
/// change set needs: after it, or, when it failed, after the entries the
/// journal holds then.
fn commit_entry(
    folder: &Path,
    written: Written,
    mut entry: Entry,
    before: &[Before],
    folders_made: &[PathBuf],
    replacements: &[Replacement],
) -> Result<(), Error> {
    entry.state = Some(CommitState::Pending);
    let journal = folder.join(JOURNAL_FILE);
    let committed = commit(
        &Ahead {
            journal: &journal,
            entry: entry.entry,
            pending: &line_of(written.text, &entry),
            done: &state_line(None, entry.entry, CommitState::Done, Vec::new()),
            before,
            states: &folder.join(UNDO_STATES_FOLDER),
            folders_made,
        },
        replacements,
    );
    if committed.is_ok() {
        let mut entries = written.entries;
        entry.state = Some(CommitState::Done);
        entries.push(entry);
        prune(folder, &entries);
    } else if let Ok(entries) = read_entries(folder) {
        // The undo states a commit that failed saved go too, unless its
        // entry is still pending, whose rollback needs them.
        prune(folder, &entries);
    }
    committed
}

/// Records a change set that changed nothing as a failed attempt, unless it
/// was a dry run, which is not recorded, or it landed and its entry is
/// recorded already. An attempt that cannot be recorded, such as on a
/// folder that is not a drawing set, is left unrecorded: it changed nothing.
///
/// `attempted` is what the change set did to its sheets when its files
/// could not be put in place: the sheets it would have landed on are
/// recorded as failed with the commit's error, beside those it failed on.
pub(super) fn record_failed_attempt(
    request: &ChangeRequest,
    outcome: &mut Result<ChangeReport, Error>,
    attempted: Option<Landing>,
) {
    if request.dry_run {
        return;
    }
    let folder = &request.set;
    let (landing, error) = match outcome {
        Ok(report) if report.journal_entry.is_none() => (
            Landing {
                objects: Vec::new(),
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
                objects: Vec::new(),
                landed_on: Vec::new(),
                failed_on: attempted
                    .map(|landing| {
                        let unwritten = landing.landed_on.into_iter().map(|sheet| SheetFailed {
                            sheet,
                            code: code_text(error.code),
                            message: error.message.clone(),
                        });
                        unwritten.chain(landing.failed_on).collect()
                    })
                    .unwrap_or_default(),
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
    let Ok(_lock) = lock(folder) else {
        return;
    };
    let Ok(read) = read_text(folder) else {
        return;
    };
    let (text, entries) = read.unzip();
    let entries = entries.unwrap_or_default();
    // An entry left pending is the next command's to roll back first.
    if pending(&entries).is_some() {
        return;
    }
    let number = next_number(&entries);
    let entry = Entry {
        change_set: Some(request.change_set.clone()),
        landed_on: landing.landed_on,
        failed_on: landing.failed_on,
        error,
        ..Entry::new(number, EntryKind::FailedAttempt)
    };
    let line = line_of(text.as_deref(), &entry);
    if add_line(&folder.join(JOURNAL_FILE), &line, "a failed attempt").is_ok() {
        if let Ok(report) = outcome {
            report.journal_entry = Some(number);
        }
    }
}

/// Puts back the files of an undo, with the undo's entry in the journal, or
/// none of it, and gives the entry's number.
///
/// The bytes each file has before the undo are saved as undo states first,
/// as a change set's are, so that an undo stopped at any step can be rolled
/// back; once it is done, they are pruned with the undone change sets'.
///
/// The caller holds the journal lock, `_locked`, from before it read the
/// journal and the files to work the undo out, so its entry records the
/// files as they are on disk as it commits: each is checked again here,
/// whether or not its bytes are kept already.
pub(super) fn commit_undo(
    folder: &Path,
    _locked: &File,
    undid: Vec<u64>,
    files: Vec<FileState>,
    replacements: Vec<Replacement>,
) -> Result<u64, Error> {
    let (text, entries) = read_text(folder)?.unzip();
    let entries = entries.unwrap_or_default();
    if let Some(entry) = pending(&entries) {
        return Err(left_pending(entry));
    }
    let number = next_number(&entries);
    let store = folder.join(UNDO_STATES_FOLDER);
    let mut placing: Vec<Replacement> = Vec::new();
    for file in &files {
        let now = read_state(&folder.join(&file.path))?;
        if now.as_deref().map(fingerprint) != file.before {
            return Err(Error::new(
                ErrorCode::ChangedByHand,
                format!(
                    "`{}` changed while the undo was worked out; nothing was changed",
                    folder.join(&file.path).display()
                ),
            )
            .at_field("change_set"));
        }
        let (Some(print), Some(bytes)) = (&file.before, now) else {
            continue;
        };
        let path = store.join(print);
        if path.is_file() || placing.iter().any(|r| r.path == path) {
            continue;
        }
        placing.push(Replacement {
            path,
            bytes: Some(bytes),
            what: "an undo state",
        });
    }
    let before: Vec<Before> = files
        .iter()
        .map(|f| Before {
            path: folder.join(&f.path),
            print: f.before.clone(),
        })
        .collect();
    placing.extend(replacements);
    let entry = Entry {
        undid,
        files,
        ..Entry::new(number, EntryKind::Undo)
    };
    let written = Written {
        text: text.as_deref(),
        entries,
    };
    commit_entry(folder, written, entry, &before, &[], &placing)?;
    Ok(number)
}

/// What an accept records: the change sets it issues, the failed checks it
/// was told to accept despite, and each file of the set as it issues it.
pub(super) struct Accepting {
    pub via: Interface,
    pub issued: Vec<u64>,
    pub failed_checks: Vec<CheckFailed>,
    pub revision: Vec<FileIssued>,
}

/// Adds an accept to the journal, and gives its entry's number. It writes
/// no file of the set: what it issues is recorded in its entry. It is
/// refused when the journal's newest entry is no longer `newest`, the one
/// the accept was worked out from.
pub(super) fn commit_accept(
    folder: &Path,
    newest: u64,
    accepting: Accepting,
) -> Result<u64, Error> {
    let _lock = lock(folder)?;
    let (text, entries) = read_text(folder)?.unzip();
    let entries = entries.unwrap_or_default();
    if let Some(entry) = pending(&entries) {
        return Err(left_pending(entry));
    }
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
    let entry = Entry {
        via: Some(accepting.via),
        issued: accepting.issued,
        failed_checks: accepting.failed_checks,
        revision: accepting.revision,
        ..Entry::new(number, EntryKind::Accept)
    };
    let written = Written {
        text: text.as_deref(),
        entries,
    };
    commit_entry(folder, written, entry, &[], &[], &[])?;
    Ok(number)
}

/// Takes away every undo state that neither a change set that keeps its
/// undo state needs, nor an entry left pending, whose rollback needs it:
/// those of change sets pruned or undone, and any file left in the folder
/// that is not an undo state. A file that cannot be taken away is left; it
/// is needed by nothing.
pub(super) fn prune(folder: &Path, entries: &[Entry]) {
    let fates = fates(entries);
    let needed: BTreeSet<&str> = entries
        .iter()
        .filter(|e| keeps_state(e, &fates) || e.state == Some(CommitState::Pending))
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
pub(super) fn relative(folder: &Path, path: &Path) -> String {
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
pub(super) fn now() -> String {
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
            objects: vec![ObjectChanged {
                id: "DEMO-001/line-1".into(),
                how: HowChanged::Added,
            }],
            ..Entry::new(3, EntryKind::ChangeSet)
        };
        let text = String::from_utf8(line_of(Some("{}"), &entry)).unwrap();
        assert!(text.starts_with('\n') && text.ends_with('\n'));
        assert_eq!(text.trim().lines().count(), 1);
        let back: Entry = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(back, entry);
    }
}
