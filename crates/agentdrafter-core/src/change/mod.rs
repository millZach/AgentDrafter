//! Change sets: typed operations an agent sends to change a drawing set,
//! applied at once to its working revision.
//!
//! The working revision is the set's own files, edited in place, as Zach
//! decided on 2026-11-11: a change set writes the set's records, its
//! templates and its rows, and builds again only the DWGs of the sheets it
//! touched. History is the journal's (`journal.rs`), which records every
//! change set and failed attempt in the set's folder, and from which a later
//! session undoes them (`undo.rs`); git is optional.
//!
//! An operation names its target by ID or by name, never by coordinates.
//! An edit to an object of a template changes the template, so every sheet
//! built from it follows; an object added to one sheet goes in that sheet's
//! row; and a value the template takes from the row is set in the row (all
//! three Zach's decisions of 2026-11-11).
//!
//! Each sheet takes its part of a change set fully or not at all (ADR-0005):
//! a sheet that fails keeps its records and its DWG byte for byte. A
//! template edit lands only if every sheet built from the template takes
//! its whole part, as Zach decided. When a change set lands on some sheets
//! and fails on others, the set is marked inconsistent in a file in its
//! folder, [`MARK_FILE`], which every reply about the set reports until a
//! later change set names the failed sheets under `fixes` and lands on them.
//!
//! A dry run does all of this but write: it reports what would land, what
//! would fail and what the records would become, and changes nothing.
//!
//! A change set's entry in the journal is written ahead of its files and
//! marked done once they are all in place; an entry a killed process left
//! pending is rolled back by the next command that reads the set, a dry run
//! among them (`recover.rs`).

mod accept;
mod apply;
pub(crate) mod array;
pub(crate) mod break_join;
mod commit;
pub(crate) mod fillet_chamfer;
mod from_dimension;
mod guides;
mod history;
mod journal;
pub(crate) mod offset;
pub(crate) mod operations;
pub(crate) mod original_journal;
mod proposals;
mod recover;
pub(crate) mod replace;
pub(crate) mod trim_extend;
mod undo;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::build::BuildReport;
use crate::check::{check_with, CheckReport, CheckRequest};
use crate::description::{parse_mark, MarkFailure};
use crate::error::{Error, ErrorCode};
use crate::reply::Reply;
use crate::set::Notice;
use crate::settings::Settings;
use crate::units::Unit;

pub(crate) use operations::original_change_set_schema;
pub use operations::{change_set_schema, operation_name, operations, OperationInfo, OperationList};

pub use accept::{accept_with, AcceptReport, AcceptRequest, StampRevision};
pub(crate) use accept::{relative_to, stamp_revision};
pub(crate) use apply::fit_library_inserts;
#[cfg(test)]
pub(crate) use commit::{rollback_placed_new_files, STOP_AT};
pub use guides::{
    guide_with, guides_with, GuideChecked, GuideGiven, GuideListed, GuideMark, GuidePoint,
    GuideReport, GuideRequest, GuidesReport, GuidesRequest, PointListed, Setting, SnapNamed,
    StepGiven, StepListed, GUIDES_FOLDER, TOOLS as GUIDE_TOOLS,
};
pub(crate) use guides::{LONGEST_LABEL, LONGEST_SAY, LONGEST_TITLE, MOST_STEPS};
pub(crate) use history::FilesAsOf;
pub(crate) use journal::now;
pub use journal::{
    journal, CheckFailed, CommitState, Entry, EntryKind, FileIssued, FileState, HowChanged,
    JournalEntry, JournalReport, JournalRequest, ObjectChanged, OriginalRevisionMeta, Refusal,
    RollBack, SheetFailed, UndoState, JOURNAL_FILE, KEPT_UNDO_STATES, LOCK_FILE,
    UNDO_STATES_FOLDER,
};
pub use original_journal::{original_journal_file, ORIGINALS_JOURNAL_FOLDER};
pub use proposals::{
    discard, keep_with, proposals, propose_with, DiscardReport, DiscardRequest, KeepReport,
    KeepRequest, ProposalListed, ProposalsReport, ProposalsRequest, ProposeReport, ProposeRequest,
    ASSISTANT, DRAFTER, PROPOSALS_FOLDER,
};
pub(crate) use proposals::{BY_LONGEST, BY_PATTERN};
pub use recover::RolledBack;
pub(crate) use recover::{recover, refuse_unfinished, Unfinished};
pub use undo::{undo, FileUndone, UndoAction, UndoReport, UndoRequest};

/// The file, relative to a drawing set's folder, that marks the set
/// inconsistent while it is.
pub const MARK_FILE: &str = ".agentdrafter/inconsistent.yaml";

/// A request to apply a change set to a drawing set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
    /// The change set, as plain data: `{"operations": [...]}`, with the
    /// sheets it fixes under `fixes`. Its schema is [`change_set_schema`].
    pub change_set: Value,
    /// Report what the change set would do, and write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// What a change set did, or would do in a dry run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChangeReport {
    /// The drawing set's folder.
    pub set: String,
    /// The set's declared unit, which every length here is in.
    pub units: Unit,
    /// Whether this was a dry run, which wrote nothing.
    pub dry_run: bool,
    /// The journal entry that records the change set, or the failed
    /// attempt; undo takes a change set by it. Absent for a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_entry: Option<u64>,
    /// What the change set did, in words.
    pub message: String,
    /// How many sheets took their part.
    pub landed: usize,
    /// How many sheets did not, and were left as they were.
    pub failed: usize,
    /// Each sheet the change set touched, in the set's order.
    pub sheets: Vec<SheetChange>,
    /// Each operation, in the order sent.
    pub operations: Vec<OperationOutcome>,
    /// The records written, or that a dry run would write.
    pub records: Vec<String>,
    /// The set's inconsistent mark as it stands after the change set, or
    /// would after a dry run. Absent when the set is consistent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inconsistent: Option<Inconsistent>,
    /// The checker's reply on the sheets the change set landed on, which it
    /// read back from their saved files once they were written. A failed
    /// check does not stop a change set from landing. Absent for a dry run,
    /// which saves nothing to read back, and when no sheet took its part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Reply<CheckReport>>,
    /// For a change set sent for a guide's step, under its `guide`, whether
    /// it completes the step, checked against the step's operation (#157).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guide: Option<GuideChecked>,
}

/// What became of one sheet.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetChange {
    pub sheet: String,
    /// The sheet's template, as its row writes it.
    pub template: String,
    /// Whether the sheet took its part.
    pub ok: bool,
    /// The operations that were its part, by their place in the change set.
    pub operations: Vec<usize>,
    /// Whether a failure the set's mark holds for this sheet was cleared.
    pub fixed: bool,
    /// Whether its DWG was written: false when the rebuilt DWG is the one
    /// already there, byte for byte, and for a dry run.
    pub dwg_written: bool,
    /// What its build wrote, or would write, when it took its part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<BuildReport>,
    /// Why it did not take its part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
    /// What reading the sheet noticed that did not stop it.
    pub notices: Vec<Notice>,
}

/// What became of one operation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OperationOutcome {
    /// Its place in the change set, counted from 0.
    pub index: usize,
    /// Its name, such as `move`, whichever name or alias it was sent with.
    pub op: String,
    /// Whether it landed.
    pub ok: bool,
    /// What it does, in words.
    pub message: String,
    /// The sheets it reaches: one, or every sheet built from a template.
    pub sheets: Vec<String>,
    /// The record it writes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
    /// The ID of the object it adds or edits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The object as its record writes it before the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// The object as its record writes it after the operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// The row's value it sets, when the template takes the value from the
    /// row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_value: Option<RowValueSet>,
    /// The row's values it sets, when the template joins the value it was
    /// asked to set from a list of parts and the new text was matched to
    /// the parts: each value the parts hold.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub row_values: Vec<RowValueSet>,
    /// The objects whose place was written as their `id`, so that deleting
    /// an object before them did not move their IDs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ids_written: Vec<String>,
    /// The inserts of the block a `redefine_block` or an `attsync` brought
    /// into line, by ID, on every sheet it reaches: an insert of a
    /// template on every sheet built from it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inserts: Vec<String>,
    /// Each attribute value it removed from an insert, because the insert's
    /// block no longer carries the tag.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes_removed: Vec<AttributeRemoved>,
    /// What an import_blocks imported: the blocks it added and replaced,
    /// with the blocks each nests, the entries it added to the library's
    /// tables, what it could not carry, block by block, and its notices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported: Option<crate::import::BlocksImported>,
    /// Each snapped point it took, and the point it was worked out to,
    /// which the record writes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub snapped: Vec<SnapResolved>,
    /// The row values it created: when it set a value an object shares
    /// with its copies, the object's own, which every sheet built from the
    /// template was given, set to what the object showed there.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created_row_values: Vec<String>,
    /// Each copy a copy, an array or a mirror made, in the order it made
    /// them: its ID, the object it copies and where it went.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<Copied>,
    /// The objects a rotate, a scale or a mirror that erased its source
    /// transformed in place, by ID on the sheet the operation names.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transformed: Vec<String>,
    /// Each text, mtext or attribute value a `replace_text` operation changed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replacements: Vec<TextReplaced>,
    /// Each insert attribute a `set_attributes` operation changed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes_set: Vec<AttributeUpdated>,
    /// The ID of the second object created when `trim` (`keep: "ends"`) or
    /// `break` splits an object into two pieces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_id: Option<String>,
    /// The IDs of objects removed by the operation, such as the pieces
    /// absorbed into `targets[0]` by `join`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<String>,
    /// Why it did not land.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}

/// One text, multiline text or attribute value changed by `replace_text`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextReplaced {
    /// The sheet the object sits on.
    pub sheet: String,
    /// The object's ID on that sheet.
    pub id: String,
    /// Which kind of text changed: `text`, `mtext`, or `attribute`.
    pub kind: String,
    /// The attribute's tag, in capitals, when `kind` is `attribute`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Which occurrence of a repeated attribute tag changed, counted from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<usize>,
    /// The text before replacement.
    pub before: String,
    /// The text after replacement.
    pub after: String,
    /// How many matches were replaced in this text.
    pub count: usize,
    /// The row value written when the template takes this text from the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_value: Option<String>,
}

/// One insert attribute updated by `set_attributes`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributeUpdated {
    /// The sheet the insert sits on.
    pub sheet: String,
    /// The insert's ID on that sheet.
    pub id: String,
    /// The block the insert places.
    pub block: String,
    /// The attribute's tag, in capitals.
    pub tag: String,
    /// Which occurrence of a repeated attribute tag was set, counted from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<usize>,
    /// The attribute's value before the operation.
    pub before: String,
    /// The attribute's value after the operation.
    pub after: String,
    /// The row value written when the template takes this attribute from the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_value: Option<String>,
}

/// A copy a copy or an array made: an ordinary object with an ID of its
/// own, written where the object it copies is.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Copied {
    /// Its ID, on the sheet the operation names. A copy of a template's
    /// object is on every sheet built from the template, with the same ID
    /// after each sheet's name.
    pub id: String,
    /// The object it is a copy of.
    pub source: String,
    /// For a copy, which of its offsets, counted from 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    /// For a rectangular array, its row, counted from 0 up from the
    /// objects' own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    /// For a rectangular array, its column, counted from 0 across from the
    /// objects' own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// For a polar or a path array, its item, counted from 0, the objects
    /// themselves item 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<usize>,
    /// For a path array, how far along the path it is, in the declared
    /// unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub along: Option<f64>,
    /// How far it was moved from the object it copies, in the declared
    /// unit.
    pub by: [f64; 3],
    /// How far it was then turned, in degrees anticlockwise, about
    /// `about`; left out when it was not turned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turned: Option<f64>,
    /// The point it was turned about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<[f64; 3]>,
    /// For a mirror, the two points of the line it was mirrored across.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirrored_across: Option<[[f64; 3]; 2]>,
}

/// An attribute value a block operation removed from an insert, because
/// the insert's block no longer carries its tag, as AutoCAD's ATTSYNC
/// removes it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributeRemoved {
    /// The record that wrote it.
    pub record: String,
    /// The insert's place in the record, such as `objects[2]`,
    /// `rows[1].objects[0]` or, for an insert in another block of the
    /// library, `blocks.OUTER.objects[1]`.
    pub field: String,
    /// The insert's ID on each sheet it is on; none for an insert in a
    /// block, which is on every sheet that inserts that block.
    pub ids: Vec<String>,
    /// The tag, in capitals.
    pub tag: String,
    /// Which of the tag's values was removed, counted from 1, when the
    /// block still defines the tag but fewer times than the insert's list
    /// gave values for: an item past the last it keeps. Left out when the
    /// whole value was removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<usize>,
    /// The value as the record wrote it, such as `B` or `{row: REV}`; a
    /// row's value itself stays in its row.
    pub value: String,
}

/// A snapped point an operation took, and the point it named when the
/// operation applied, which the record writes as coordinates.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SnapResolved {
    /// Its place in the operation, such as `from` or `points[0]`.
    pub field: String,
    /// The object it snapped to: on the sheet its ID gives, or for one
    /// named by its name, on the first sheet the operation reaches.
    pub id: String,
    pub snap: crate::geometry::SnapKind,
    /// Which of the object's points of the snap, when it has several.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub which: Option<String>,
    /// Where it is, in the declared unit, on every sheet the operation
    /// reaches.
    pub at: crate::model::Point3,
}

/// A row's value an operation sets, because the template takes the value
/// it was asked to set from the row.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RowValueSet {
    pub sheet: String,
    /// The value's name, such as `TITLE`.
    pub value: String,
    /// Its text before.
    pub was: String,
    /// Its text after.
    pub now: String,
    /// Every place on the sheet the value fills.
    pub fills: Vec<Fill>,
}

/// One place on a sheet a row's value fills.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fill {
    /// The object's ID.
    pub id: String,
    /// Where in the object, such as `attributes.TITLE` or `contents`.
    pub at: String,
}

/// A drawing set's inconsistent mark: the sheets change sets failed on
/// while they landed on others.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Inconsistent {
    /// The mark's file.
    pub file: String,
    /// What it means and how it is cleared, in words.
    pub message: String,
    pub failures: Vec<Failure>,
}

/// One sheet a change set failed on while it landed on others.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Failure {
    pub sheet: String,
    /// The error code of why it failed.
    pub code: String,
    pub message: String,
    /// The sheets the change set landed on.
    pub landed_on: Vec<String>,
    /// The operations that were the sheet's part, as sent.
    pub operations: Vec<Value>,
}

/// Applies a change set, reading fonts from the font folders the
/// environment names ([`Settings::from_env`]). See [`change_with`].
pub fn change(request: &ChangeRequest) -> Reply<ChangeReport> {
    change_with(request, &Settings::from_env())
}

/// Applies a change set to a drawing set's working revision, or reports
/// what it would do in a dry run, reading fonts from the font folders
/// `settings` names to build the sheets it touches.
///
/// The reply is `ok` with the report when every sheet it touched took its
/// part; not `ok`, with the report and a `sheets_failed` error, when some
/// did not; and not `ok`, with only an error, when the change set itself is
/// wrong, such as an operation that does not exist, and nothing was done.
///
/// Every change set but a dry run is recorded in the set's journal: one
/// that landed in the same step as its files, and one that changed nothing
/// as a failed attempt.
///
/// An entry a commit that never finished left pending is rolled back first,
/// and the reply says so under `rolled_back`.
pub fn change_with(request: &ChangeRequest, settings: &Settings) -> Reply<ChangeReport> {
    match recover(&request.set) {
        Ok(rolled_back) => change_set(request, settings).after(rolled_back),
        Err(error) => Reply::from(Err(error)),
    }
}

fn change_set(request: &ChangeRequest, settings: &Settings) -> Reply<ChangeReport> {
    land(request, settings, None)
}

/// Applies a change set, as `change` does, keeping the proposal `keeping`
/// names, if any: its entry in the journal records the proposal's number,
/// who proposed it and who kept it, as a failed attempt's does.
fn land(
    request: &ChangeRequest,
    settings: &Settings,
    keeping: Option<&journal::Keeping>,
) -> Reply<ChangeReport> {
    // A change set sent for a guide's step names it under `guide`: the step
    // is checked to be one of the guide's, and what its own operation would
    // do now is worked out first, on the set as it is, to check it against.
    let mut guided = match guides::prepare(request, settings) {
        Ok(guided) => guided,
        Err(error) => return Reply::from(Err(error)),
    };
    // Whether it completes the step is checked on a dry run first, so its
    // journal entry records it: only the change set that completes a step
    // names, for the steps after it, what the step made.
    if let Some(guided) = guided.as_mut().filter(|_| !request.dry_run) {
        let dry = ChangeRequest {
            dry_run: true,
            ..guided.request.clone()
        };
        let would = apply::apply(&dry, settings, &mut None, None, keeping, keeping.is_some());
        let completes = would.is_ok_and(|report| guides::checked(guided, &report).completes);
        guides::mark_completes(guided, completes);
    }
    let request = guided.as_ref().map_or(request, |g| &g.request);
    let mut attempted = None;
    let mut outcome = apply::apply(
        request,
        settings,
        &mut attempted,
        None,
        keeping,
        keeping.is_some(),
    );
    journal::record_failed_attempt(request, &mut outcome, attempted, keeping);
    // Once the change set and its journal entry are written, the checker
    // reads the sheets it landed on back from the saved files.
    if let Ok(report) = &mut outcome {
        report.checks = checks(request, report, settings);
        if let Some(guided) = &guided {
            report.guide = Some(guides::checked(guided, report));
        }
    }
    match outcome {
        Ok(report) if report.failed > 0 => {
            let error = Error::new(ErrorCode::SheetsFailed, report.message.clone());
            Reply::partly(report, error)
        }
        outcome => Reply::from(outcome),
    }
}

/// A change set applied in memory to a drawing set as it is, for a
/// preview: what its dry run reports, the set's files as it would leave
/// them, and each object it would add, change or delete on each sheet it
/// would land on, as its journal entry would name them.
pub(crate) struct Previewed {
    pub report: ChangeReport,
    pub files: Arc<FilesAsOf>,
    pub objects: Vec<ObjectChanged>,
}

/// Applies `change_set` to the drawing set in `set` as a dry run does, and
/// keeps what it would write. Nothing is written and nothing goes in the
/// journal. An entry a commit left pending, or a journal's last line
/// written in part, is not rolled back, since that writes: the preview is
/// refused (`rollback_failed`). A change set the core refuses whole gives
/// its error, as `change` would.
pub(crate) fn preview(
    set: &Path,
    change_set: &Value,
    settings: &Settings,
    whole: bool,
) -> Result<Previewed, Error> {
    refuse_unfinished(set, Unfinished::Preview)?;
    // A guide's step it names is one the set holds, as `change` and `keep`
    // check it.
    guides::check_mark(set, change_set)?;
    let request = ChangeRequest {
        set: set.to_owned(),
        change_set: change_set.clone(),
        dry_run: true,
    };
    let mut kept = apply::Kept::default();
    let report = apply::apply(&request, settings, &mut None, Some(&mut kept), None, whole)?;
    Ok(Previewed {
        report,
        files: Arc::new(FilesAsOf::with_records(set, &kept.records)),
        objects: kept.objects,
    })
}

/// The checker's reply on the sheets a change set landed on, read back from
/// the set's saved files, as `agentdrafter check` reads them: never the
/// records this module held in memory, so the change set does not mark its
/// own work. None for a dry run, which saves nothing, and when no sheet took
/// its part.
fn checks(
    request: &ChangeRequest,
    report: &ChangeReport,
    settings: &Settings,
) -> Option<Reply<CheckReport>> {
    if report.dry_run {
        return None;
    }
    let landed: Vec<String> = report
        .sheets
        .iter()
        .filter(|sheet| sheet.ok)
        .map(|sheet| sheet.sheet.clone())
        .collect();
    if landed.is_empty() {
        return None;
    }
    let request = CheckRequest {
        source: request.set.clone(),
        sheets: landed,
    };
    Some(Reply::from(check_with(&request, settings)))
}

/// The drawing set's inconsistent mark, if it is marked. A mark that cannot
/// be read is reported as such, with no failures.
pub(crate) fn inconsistent(folder: &Path) -> Option<Inconsistent> {
    match read_mark(folder) {
        Ok(failures) if failures.is_empty() => None,
        Ok(failures) => Some(mark_report(folder, &failures)),
        Err(error) => Some(Inconsistent {
            file: folder.join(MARK_FILE).display().to_string(),
            message: format!(
                "the set's inconsistent mark cannot be read, so whether the set is consistent \
                 is not known: {}",
                error.message
            ),
            failures: Vec::new(),
        }),
    }
}

/// The failures the set's mark holds; none when it is not marked.
fn read_mark(folder: &Path) -> Result<Vec<MarkFailure>, Error> {
    let path = folder.join(MARK_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => parse_mark(&text).map_err(|e| e.or_in_file(&path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the set's inconsistent mark: {e}"),
        )
        .in_file(&path)),
    }
}

/// The mark holding `failures`, as a reply reports it.
fn mark_report(folder: &Path, failures: &[MarkFailure]) -> Inconsistent {
    let mut sheets: Vec<&str> = Vec::new();
    for failure in failures {
        if !sheets.contains(&failure.sheet.as_str()) {
            sheets.push(&failure.sheet);
        }
    }
    let (left, them) = if sheets.len() == 1 {
        ("was left as it was", "it")
    } else {
        ("were left as they were", "them")
    };
    Inconsistent {
        file: folder.join(MARK_FILE).display().to_string(),
        message: format!(
            "the set is inconsistent: a change set landed on some of its sheets and failed on \
             {}, which {left}. Fix {them} with a change set that names {them} under `fixes`, \
             or undo the change set; the reviewer cannot accept the set until then",
            quoted_list(&sheets),
        ),
        failures: failures
            .iter()
            .map(|f| Failure {
                sheet: f.sheet.clone(),
                code: f.code.clone(),
                message: f.message.clone(),
                landed_on: f.landed_on.clone(),
                operations: f
                    .operations
                    .iter()
                    .map(|o| serde_json::from_str(o).unwrap_or_else(|_| Value::String(o.clone())))
                    .collect(),
            })
            .collect(),
    }
}

/// `items` in backquotes, joined with commas and `and`.
fn quoted_list(items: &[&str]) -> String {
    let quoted: Vec<String> = items.iter().map(|i| format!("`{i}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests;
