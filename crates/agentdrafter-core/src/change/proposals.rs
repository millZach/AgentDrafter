//! Proposals: a change set that waits (#155, drafting app build 7).
//!
//! An agent that wants its work looked at first calls `propose` where it
//! would call `change`. The change set is checked as `change` checks it,
//! by a dry run on the set as it is, and recorded beside the journal, in
//! [`PROPOSALS_FOLDER`], one file per proposal, numbered on their own:
//! who proposed it, when, the journal's newest entry it was made against,
//! the change set and the sheets it reaches. Nothing else is written.
//!
//! A proposal waits until it ends, which the journal records, so that what
//! ends it lands in the same step as the entry that says so:
//!
//! - `keep` lands it through `change`'s own path, on the set as it stands
//!   then, each sheet fully or not at all (ADR-0005). Its change set's
//!   entry names the proposal, who proposed it and who kept it, and is
//!   undone and accepted as any change set's; undoing it does not bring
//!   the proposal back. A keep the core refuses is recorded as `change`
//!   records a failed attempt, naming the proposal too, and the proposal
//!   goes on waiting.
//! - `discard` adds a line to the journal, `discard`, naming the proposal
//!   and who discarded it. It changes no file of the set, so nothing undoes
//!   it, and an undo passes over it.
//!
//! Who proposed, kept or discarded is the caller's own words (`by`), or,
//! when it gives none, the interface it came through, such as `agent
//! (MCP)`. It is recorded and shown, and nothing else differs by it.

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::journal::{self, commit_discard, listed, now, Entry, EntryKind, Keeping};
use super::recover::{refuse_unfinished, Unfinished};
use super::{apply, land, ChangeReport, ChangeRequest};
use crate::description::read_set_settings;
use crate::error::{Error, ErrorCode};
use crate::interface::Interface;
use crate::reply::Reply;
use crate::set::SET_SETTINGS_FILE;
use crate::settings::Settings;

/// The folder, relative to a drawing set's folder, that holds its
/// proposals, one file each, `<number>.json`, beside the journal.
pub const PROPOSALS_FOLDER: &str = ".agentdrafter/proposals";

/// The file a process holds locked while it numbers and writes a proposal,
/// so no two take one number. It holds nothing.
const PROPOSALS_LOCK: &str = "proposals.lock";

/// The name the viewer's page gives the drafter, and the one a request
/// through the viewer with no `by` is recorded under.
pub const DRAFTER: &str = "drafter";

/// The name the drafter's assistant proposes under (#156): the MCP server
/// AgentDrafter starts it on records a proposal with no `by` under it, and
/// the journal reads `proposed by Assistant`.
pub const ASSISTANT: &str = "Assistant";

/// The most characters `by` may hold.
const MOST_BY: usize = 80;

/// A proposal, as its file holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    proposal: u64,
    by: String,
    at: String,
    /// The journal's newest entry when it was made, 0 for an empty journal.
    base_entry: u64,
    change_set: Value,
    /// The sheets it reached when it was made, in the set's order.
    sheets: Vec<String>,
}

/// A request to propose a change set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
    /// The change set, as `change` takes it.
    pub change_set: Value,
    /// Who proposes it, in the caller's words, such as `Assistant`; with
    /// none, the interface it came through.
    #[serde(
        default,
        deserialize_with = "text_given",
        skip_serializing_if = "Option::is_none"
    )]
    pub by: Option<String>,
}

/// A proposal recorded.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposeReport {
    /// The drawing set's folder.
    pub set: String,
    /// The proposal's number, which `keep` and `discard` take.
    pub proposal: u64,
    pub by: String,
    pub at: String,
    /// The journal's newest entry it was made against; 0 for an empty
    /// journal.
    pub base_entry: u64,
    /// The sheets it reaches, in the set's order.
    pub sheets: Vec<String>,
    /// Its file.
    pub file: String,
    /// What was recorded, in words.
    pub message: String,
    /// What keeping it would do on the set as it is now: its dry run.
    pub would: ChangeReport,
}

/// A request to list a drawing set's waiting proposals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalsRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
}

/// A drawing set's waiting proposals.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposalsReport {
    /// The drawing set's folder.
    pub set: String,
    /// The folder the proposals are kept in.
    pub folder: String,
    /// What is waiting, in words.
    pub message: String,
    /// Each proposal still waiting, the oldest first.
    pub proposals: Vec<ProposalListed>,
}

/// One waiting proposal.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProposalListed {
    pub proposal: u64,
    pub by: String,
    pub at: String,
    pub base_entry: u64,
    /// The change sets and undos that landed after it was made, by their
    /// entries, the oldest first: keeping it applies it to the set as it
    /// stands, with them.
    pub changed_since: Vec<u64>,
    /// The change set, with its operations, as `change` and `display-list`
    /// take it.
    pub change_set: Value,
    /// The sheets it reached when it was made, in the set's order.
    pub sheets: Vec<String>,
}

/// A request to keep a proposal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeepRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
    /// The proposal, by its number.
    pub proposal: u64,
    /// Who keeps it, in the caller's words; with none, the interface it
    /// came through.
    #[serde(
        default,
        deserialize_with = "text_given",
        skip_serializing_if = "Option::is_none"
    )]
    pub by: Option<String>,
}

/// A proposal kept: `change`'s reply, with the proposal and who proposed
/// and kept it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KeepReport {
    pub proposal: u64,
    pub proposed_by: String,
    pub kept_by: String,
    #[serde(flatten)]
    pub change: ChangeReport,
}

/// A request to discard a proposal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscardRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
    /// The proposal, by its number.
    pub proposal: u64,
    /// Who discards it, in the caller's words; with none, the interface it
    /// came through.
    #[serde(
        default,
        deserialize_with = "text_given",
        skip_serializing_if = "Option::is_none"
    )]
    pub by: Option<String>,
}

/// A proposal discarded.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DiscardReport {
    /// The drawing set's folder.
    pub set: String,
    pub proposal: u64,
    pub proposed_by: String,
    pub discarded_by: String,
    /// The discard's entry in the journal, which nothing undoes.
    pub journal_entry: u64,
    pub message: String,
}

/// `by` as given: text, never `null`, which the input's schema refuses
/// too; left out, it is `None`.
pub(super) fn text_given<'de, D: serde::Deserializer<'de>>(
    given: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(given).map(Some)
}

/// The pattern of `by` in the input's schema, which [`who`] holds it to:
/// no control character, and something besides spaces.
pub(crate) const BY_PATTERN: &str =
    r"^[^\x00-\x1f\x7f-\x9f]*[^\s\x00-\x1f\x7f-\x9f][^\x00-\x1f\x7f-\x9f]*$";

/// The most characters `by` may hold, spaces round it among them.
pub(crate) const BY_LONGEST: usize = MOST_BY;

/// Who a request is recorded as made by: its own `by`, trimmed, or the
/// interface it came through. `by` is refused as the input's schema
/// refuses it: more than [`MOST_BY`] characters, a control character, or
/// nothing but spaces.
pub(super) fn who(by: Option<&str>, via: Interface) -> Result<String, Error> {
    let Some(by) = by else {
        return Ok(match via {
            Interface::Cli => "agent (command line)",
            Interface::Http => "agent (HTTP API)",
            Interface::Mcp => "agent (MCP)",
            Interface::Python => "agent (Python)",
            Interface::Viewer => DRAFTER,
        }
        .to_owned());
    };
    if by.chars().count() > MOST_BY || by.chars().any(char::is_control) || by.trim().is_empty() {
        return Err(Error::usage(format!(
            "`by` is who it is, in up to {MOST_BY} characters on one line, such as `Assistant`"
        ))
        .at_field("by"));
    }
    Ok(by.trim().to_owned())
}

/// Refuses a proposal's number below 1, as the input's schema does.
fn numbered(proposal: u64) -> Result<(), Error> {
    if proposal == 0 {
        return Err(
            Error::usage("`proposal` is a proposal's number, counted from 1").at_field("proposal"),
        );
    }
    Ok(())
}

/// Records a change set as a proposal of the drawing set in `request.set`,
/// once a dry run on the set as it is finds it would land: a change set
/// the core refuses whole, or that a sheet would refuse for its records,
/// is not recorded. A sheet whose DWG cannot be written now, such as one
/// open in AutoCAD, does not stop it: that is for whoever keeps it.
pub fn propose_with(
    request: &ProposeRequest,
    settings: &Settings,
    via: Interface,
) -> Result<ProposeReport, Error> {
    let by = who(request.by.as_deref(), via)?;
    let folder = &request.set;
    // It writes nothing to the set, so it rolls nothing back: a set whose
    // journal holds an unfinished commit is refused, as a preview is.
    refuse_unfinished(folder, Unfinished::Proposals)?;
    // A change set sent for a guide's step names one the set holds, or
    // `keep` would refuse it: so is the proposal.
    super::guides::check_mark(folder, &request.change_set)?;
    let dry = ChangeRequest {
        set: folder.clone(),
        change_set: request.change_set.clone(),
        dry_run: true,
    };
    // As `keep` would land it: on every sheet it reaches or on none.
    let would = apply::apply(&dry, settings, &mut None, None, None, true)?;
    // What its records would be is checked, not whether each output can be
    // written now: a DWG open in AutoCAD, and the sheets of its template it
    // holds back, which would take a template edit with it, do not stop it.
    let refused: Vec<String> = would
        .sheets
        .iter()
        .filter_map(|s| {
            let e = s.error.as_ref()?;
            (!matches!(e.code, ErrorCode::WriteFailed | ErrorCode::SharedEditFailed))
                .then(|| format!("`{}`: {}", s.sheet, e.message))
        })
        .collect();
    if !refused.is_empty() {
        return Err(Error::new(
            ErrorCode::SheetsFailed,
            format!(
                "the proposal was not recorded, since the set as it is would refuse it on {}; \
                 nothing was changed",
                refused.join("; ")
            ),
        ));
    }
    let sheets: Vec<String> = would.sheets.iter().map(|s| s.sheet.clone()).collect();
    let base_entry = newest(&journal::read_entries(folder)?);

    let store = folder.join(PROPOSALS_FOLDER);
    let failed = |what: &str, path: &Path, e: std::io::Error| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not {what}: {e}; the proposal was not recorded"),
        )
        .in_file(path)
    };
    fs::create_dir_all(&store).map_err(|e| failed("make the proposals' folder", &store, e))?;
    let lock_path = store.join(PROPOSALS_LOCK);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| failed("open the proposals' lock", &lock_path, e))?;
    lock.lock()
        .map_err(|e| failed("take the proposals' lock", &lock_path, e))?;
    let number = numbers(&store)?.last().copied().unwrap_or(0) + 1;
    let proposal = Proposal {
        proposal: number,
        by: by.clone(),
        at: now(),
        base_entry,
        change_set: request.change_set.clone(),
        sheets: sheets.clone(),
    };
    let path = store.join(format!("{number}.json"));
    let mut text = serde_json::to_string_pretty(&proposal).expect("a proposal serializes to JSON");
    text.push('\n');
    // Written beside it and moved into place, so a proposal is never read
    // in part.
    let part = store.join(format!("{number}.json.part"));
    let written = fs::File::create(&part)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&part, &path));
    if let Err(e) = written {
        let _ = fs::remove_file(&part);
        return Err(failed("write the proposal", &path, e));
    }
    drop(lock);

    let reach = match sheets.as_slice() {
        [one] => format!("`{one}`"),
        more => format!("{} sheets", more.len()),
    };
    let failing = |code: ErrorCode| -> Vec<String> {
        would
            .sheets
            .iter()
            .filter(|s| s.error.as_ref().is_some_and(|e| e.code == code))
            .map(|s| format!("`{}`", s.sheet))
            .collect()
    };
    let (unwritable, held) = (
        failing(ErrorCode::WriteFailed),
        failing(ErrorCode::SharedEditFailed),
    );
    let note = match (unwritable.as_slice(), held.len()) {
        ([], _) => String::new(),
        (one, held) => format!(
            " ({} cannot be written now, such as a DWG open in AutoCAD{}, and must be by the \
             time it is kept)",
            if one.len() == 1 {
                format!("the DWG of {}", one[0])
            } else {
                format!("the DWGs of {}", one.join(", "))
            },
            match held {
                0 => String::new(),
                1 => ", which holds back 1 other sheet it reaches".to_owned(),
                n => format!(", which holds back {n} other sheets it reaches"),
            }
        ),
    };
    Ok(ProposeReport {
        set: folder.display().to_string(),
        proposal: number,
        message: format!(
            "proposal {number} recorded, by {by}: it would land on {reach}{note}. Nothing has \
             changed: `keep` lands it, on the set as it stands then, and `discard` ends it"
        ),
        by,
        at: proposal.at,
        base_entry,
        sheets,
        file: path.display().to_string(),
        would,
    })
}

/// Lists the drawing set's waiting proposals: those no change set has
/// kept and no discard has ended.
pub fn proposals(request: &ProposalsRequest) -> Result<ProposalsReport, Error> {
    let folder = &request.set;
    read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    refuse_unfinished(folder, Unfinished::Proposals)?;
    let entries = journal::read_entries(folder)?;
    let ended = ended(&entries);
    let store = folder.join(PROPOSALS_FOLDER);
    let mut waiting = Vec::new();
    for number in numbers(&store)? {
        if ended.contains(&number) {
            continue;
        }
        let p = read(folder, number)?;
        let changed_since = entries
            .iter()
            .filter(|e| e.entry > p.base_entry && e.finished())
            .filter(|e| matches!(e.kind, EntryKind::ChangeSet | EntryKind::Undo))
            .map(|e| e.entry)
            .collect();
        waiting.push(ProposalListed {
            proposal: p.proposal,
            by: p.by,
            at: p.at,
            base_entry: p.base_entry,
            changed_since,
            change_set: p.change_set,
            sheets: p.sheets,
        });
    }
    let numbers: Vec<u64> = waiting.iter().map(|p| p.proposal).collect();
    let message = match numbers.as_slice() {
        [] => "no proposal is waiting".to_owned(),
        [one] => format!("proposal {one} is waiting: `keep` lands it, `discard` ends it"),
        more => format!(
            "proposals {} are waiting: `keep` lands one, `discard` ends one",
            listed(more)
        ),
    };
    Ok(ProposalsReport {
        set: folder.display().to_string(),
        folder: store.display().to_string(),
        message,
        proposals: waiting,
    })
}

/// Keeps a proposal: lands its change set through `change`'s own path, on
/// the set as it stands, each sheet fully or not at all. The reply is
/// `change`'s, with the proposal; refused, the attempt is recorded as
/// `change` records one, and the proposal goes on waiting. A proposal that
/// has ended is refused (`proposal_ended`), and nothing is recorded.
pub fn keep_with(request: &KeepRequest, settings: &Settings, via: Interface) -> Reply<KeepReport> {
    let kept_by = match who(request.by.as_deref(), via).and_then(|by| {
        numbered(request.proposal)?;
        Ok(by)
    }) {
        Ok(by) => by,
        Err(error) => return Reply::from(Err(error)),
    };
    let folder = &request.set;
    let proposal = match waiting(folder, request.proposal) {
        Ok(p) => p,
        Err(error) => return Reply::from(Err(error)),
    };
    let keeping = Keeping {
        proposal: proposal.proposal,
        proposed_by: proposal.by.clone(),
        kept_by: kept_by.clone(),
    };
    let change = ChangeRequest {
        set: folder.clone(),
        change_set: proposal.change_set,
        dry_run: false,
    };
    let reply = land(&change, settings, Some(&keeping));
    Reply {
        ok: reply.ok,
        rolled_back: reply.rolled_back,
        warnings: reply.warnings,
        result: reply.result.map(|change| KeepReport {
            proposal: proposal.proposal,
            proposed_by: proposal.by,
            kept_by,
            change,
        }),
        error: reply.error,
    }
}

/// Discards a proposal: it ends without landing, and the journal records
/// it, with no file changed and nothing to undo. A proposal that has ended
/// is refused (`proposal_ended`).
pub fn discard(request: &DiscardRequest, via: Interface) -> Result<DiscardReport, Error> {
    let by = who(request.by.as_deref(), via)?;
    numbered(request.proposal)?;
    let folder = &request.set;
    let proposal = waiting(folder, request.proposal)?;
    let locked = journal::lock(folder)?;
    let entry = commit_discard(folder, &locked, proposal.proposal, &proposal.by, &by)?;
    drop(locked);
    Ok(DiscardReport {
        set: folder.display().to_string(),
        proposal: proposal.proposal,
        message: format!(
            "proposal {}, by {}, discarded by {by}, recorded in entry {entry} of the journal: \
             nothing changed, and nothing undoes a discard",
            proposal.proposal, proposal.by
        ),
        proposed_by: proposal.by,
        discarded_by: by,
        journal_entry: entry,
    })
}

/// Refuses a keep or a discard of proposal `number` when an entry of
/// `entries` ended it: a change set that kept it, undone since or not, or
/// a discard.
pub(super) fn refuse_ended(entries: &[Entry], number: u64) -> Result<(), Error> {
    let Some(end) = entries.iter().find(|e| ends(e, number)) else {
        return Ok(());
    };
    let how = match end.kind {
        EntryKind::Discard => format!(
            "it was discarded, by {}, in entry {} of the journal",
            end.discarded_by.as_deref().unwrap_or("someone"),
            end.entry
        ),
        _ => format!(
            "it was kept, by {}, as change set {}",
            end.kept_by.as_deref().unwrap_or("someone"),
            end.entry
        ),
    };
    Err(Error::new(
        ErrorCode::ProposalEnded,
        format!("proposal {number} has ended: {how}; nothing was changed"),
    )
    .at_field("proposal"))
}

/// Whether `entry` ended proposal `number`.
fn ends(entry: &Entry, number: u64) -> bool {
    entry.proposal == Some(number)
        && entry.finished()
        && matches!(entry.kind, EntryKind::ChangeSet | EntryKind::Discard)
}

/// The proposals `entries` ended.
fn ended(entries: &[Entry]) -> BTreeSet<u64> {
    entries
        .iter()
        .filter(|e| e.finished() && matches!(e.kind, EntryKind::ChangeSet | EntryKind::Discard))
        .filter_map(|e| e.proposal)
        .collect()
}

/// The journal's newest entry, 0 for none.
pub(super) fn newest(entries: &[Entry]) -> u64 {
    entries.iter().map(|e| e.entry).max().unwrap_or(0)
}

/// Proposal `number` of the set in `folder`, while it waits.
fn waiting(folder: &Path, number: u64) -> Result<Proposal, Error> {
    read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let store = folder.join(PROPOSALS_FOLDER);
    let all = numbers(&store)?;
    if !all.contains(&number) {
        let entries = journal::read_entries(folder)?;
        let ended = ended(&entries);
        let open: Vec<u64> = all.into_iter().filter(|n| !ended.contains(n)).collect();
        let which = match open.as_slice() {
            [] => "none is waiting".to_owned(),
            [one] => format!("proposal {one} is waiting"),
            more => format!("proposals {} are waiting", listed(more)),
        };
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!("the set has no proposal {number}; {which}"),
        )
        .at_field("proposal"));
    }
    refuse_ended(&journal::read_entries(folder)?, number)?;
    read(folder, number)
}

/// The numbers of the proposals kept in `store`, the lowest first; none
/// when there is no such folder.
pub(super) fn numbers(store: &Path) -> Result<Vec<u64>, Error> {
    let listing = match fs::read_dir(store) {
        Ok(listing) => listing,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the set's proposals: {e}"),
            )
            .in_file(store))
        }
    };
    let mut found: Vec<u64> = listing
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let stem = name.to_str()?.strip_suffix(".json")?;
            (!stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit()))
                .then(|| stem.parse().ok())
                .flatten()
        })
        .collect();
    found.sort_unstable();
    Ok(found)
}

/// Reads proposal `number` of the set in `folder`.
fn read(folder: &Path, number: u64) -> Result<Proposal, Error> {
    let path = folder.join(PROPOSALS_FOLDER).join(format!("{number}.json"));
    let text = fs::read_to_string(&path).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read proposal {number}: {e}"),
        )
        .in_file(&path)
    })?;
    let proposal: Proposal = serde_json::from_str(&text).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("proposal {number} cannot be read: {e}"),
        )
        .in_file(&path)
    })?;
    if proposal.proposal != number {
        return Err(Error::new(
            ErrorCode::ReadFailed,
            format!(
                "the file of proposal {number} holds proposal {}",
                proposal.proposal
            ),
        )
        .in_file(&path));
    }
    Ok(proposal)
}
