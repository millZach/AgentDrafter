//! Revises an original DWG in place (in memory) and saves a new revision beside
//! it in the exact DWG version the original arrived in (ADR-0004, ADR-0005,
//! ADR-0006).
//!
//! The original DWG is strictly read-only: opened once as a `File`, never
//! overwritten, never followed through a symlink, and untouched objects keep
//! their raw bytes, handles, extended data, tables and custom records verbatim
//! through the `dwg::revise` seam.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(any(unix, windows))]
use crate::build::file_id_of_file;
#[cfg(not(any(unix, windows)))]
use crate::build::is_same_file;
use crate::build::FileId;
use crate::change::array::{self, At, Place};
use crate::change::break_join::{compute_break, compute_join, BreakSpec};
use crate::change::explode::compute_explode;
use crate::change::fillet_chamfer::{
    compute_chamfer_polyline, compute_chamfer_two, compute_fillet_polyline, compute_fillet_two,
    ChamferSpec,
};
use crate::change::offset::{compute_offset, OffsetBy, OffsetSide};
use crate::change::operations::{
    op_error, read_change_set_for_original, AddTo, Body, CornerMode, DrawOrderPlace, Motion,
    OpBreakBy, OpOffsetBy, OpOffsetSide, OpPoint, Operation, Pattern, Table, TableOp, Target,
    TransformBy,
};
use crate::change::original_journal::OriginalJournal;
use crate::change::replace::{TextMatcher, TextScope};
use crate::change::solid3d::{
    build_boolean_solid, build_extrude_solid, build_revolve_solid, extract_profile,
};
use crate::change::stretch::{compute_stretch, StretchAction, StretchBox};
use crate::change::trim_extend::{compute_extend, compute_trim};
use crate::change::{
    AttributeUpdated, HowChanged, ObjectChanged, OriginalRevisionMeta, SnapResolved, TextReplaced,
};
use crate::check::{check_dwg_bytes, CheckReport};
use crate::description::{
    parse_added_object_json, parse_layer_patch_json, parse_length_json, parse_new_layer_json,
};
use crate::dwg::{
    LockedLayerHit, MutationHandles, RevisionSession, RevisionVerification, TargetSpace,
    UnexpectedChange,
};
use crate::error::{Error, ErrorCode};
use crate::geometry::curve::Curve;
use crate::geometry::Reader;
use crate::model::{DecodeReport, Object, Point3, Solid3D};
#[cfg(any(unix, windows))]
use crate::original::regular_file_id_no_follow;
use crate::original::{
    dwg_loaded_from_drawing, handle_id, is_drawing_file, is_dwg, is_dxf, on_disk_path, plural,
    read_regular_no_follow, DwgLoaded, NoFollowRead, HANDLE_ID,
};
use crate::plot::{plot_dwg_bytes, plot_style, PlotReport};
use crate::query::snap::one_point;
use crate::reply::Reply;
use crate::settings::Settings;
use crate::stamp::fingerprint;
use crate::text::placement::SheetFonts;
use crate::units::Unit;
use crate::warning::{Warnings, Warns};

/// File extension written beside a revision DWG holding its change list and
/// revision report (e.g. `P-101.rev1.changes.json`).
pub const REVISION_CHANGES_EXTENSION: &str = "changes.json";

/// Identity of an original or revision DWG file recorded in reports and change
/// lists.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RevisionFile {
    /// File name (such as `P-101.dwg` or `P-101.rev1.dwg`).
    pub file: String,
    /// SHA-256 fingerprint of the DWG bytes.
    pub sha256: String,
    /// Exact DWG version tag (such as `AC1027` or `AC1032`).
    pub dwg_version: String,
}

/// One copy created by `copy`, `array`, or `mirror` on an original DWG.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RevisionCopy {
    pub id: String,
    pub handle: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub along: Option<f64>,
    pub by: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turned: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirrored_across: Option<[[f64; 3]; 2]>,
}

/// Outcome of one operation in an original-DWG revision.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RevisionOperationOutcome {
    pub index: usize,
    pub op: String,
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub handles_touched: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub handles_added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub handles_deleted: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<RevisionCopy>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub snapped: Vec<SnapResolved>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replacements: Vec<TextReplaced>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes_set: Vec<AttributeUpdated>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transformed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<String>,
}

/// The active working revision a chained revision was built on top of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParentRevision {
    /// Journal entry number of the parent working revision.
    pub entry: u64,
    /// File name of the parent working revision.
    pub file: String,
    /// SHA-256 fingerprint of the parent working revision bytes.
    pub sha256: String,
}

/// What `agentdrafter change <original.dwg> ...` returns.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RevisionReport {
    /// The original DWG path as requested.
    pub source: String,
    /// Canonical sheet name of the original (its on-disk stem).
    pub sheet: String,
    /// Whether this was a dry run, which wrote nothing.
    pub dry_run: bool,
    /// The journal entry that records this revision (or failed attempt);
    /// `undo` takes a revision by it. Absent for a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal_entry: Option<u64>,
    /// What the revision did (or would do, or why it was refused), in words.
    pub message: String,
    /// Metadata of the original DWG, read once and never modified.
    pub original: RevisionFile,
    /// When chained onto an active working revision, identifies that parent
    /// working revision. Absent when built directly from the original DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<ParentRevision>,
    /// Metadata of the revision DWG (same DWG version as the original).
    /// Absent when refused before writing (such as incomplete decode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<RevisionFile>,
    /// The revision DWG path written (or that a dry run would write).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_dwg: Option<String>,
    /// The change list & revision report file written beside the revision DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_list: Option<String>,
    /// The revision PDF written beside the revision DWG, when plotted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf: Option<String>,
    /// Why the revision PDF could not be plotted, when plotting failed cleanly
    /// without discarding or corrupting the revision DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_error: Option<Error>,
    /// The plot report when the revision PDF was plotted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plot: Option<PlotReport>,
    /// The drawing's declared unit; none when unitless.
    pub units: Option<Unit>,
    /// Decode report from reading the original DWG.
    pub decode: DecodeReport,
    /// Whether the reviewer allowed revising despite incomplete decode.
    pub allowed_loss: bool,
    /// Per-operation results in order.
    pub operations: Vec<RevisionOperationOutcome>,
    /// Requested additions that failed to appear, or requested deletions
    /// (including deleted child handles) that survived in the serialized
    /// output. Never bypassable with `--allow-loss` (ADR-0005).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_failures: Vec<UnexpectedChange>,
    /// Any handle or layer added, removed, or changed outside what the change
    /// set requested, detected by re-reading the serialized revision bytes.
    pub unexpected_changes: Vec<UnexpectedChange>,
    /// Checker reply on the revision DWG (re-read from its serialized bytes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Reply<CheckReport>>,
}

impl Warns for RevisionReport {
    fn warn(&self, warnings: &mut Warnings) {
        if let Some(plot) = &self.plot {
            plot.warn(warnings);
        }
        if let Some(checks) = self.checks.as_ref().and_then(|c| c.result.as_ref()) {
            checks.warn(warnings);
        }
    }
}

/// The JSON file written beside `<rev-stem>.dwg` as `<rev-stem>.changes.json`,
/// holding the change list, original/revision fingerprints, accepted decode
/// losses, unexpected-change diff report, and checker results.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RevisionChangeListFile {
    /// Format version (1), reserved for forward compatibility with journal/undo/accept.
    pub format: u32,
    pub original: RevisionFile,
    /// Canonical path of the original DWG when this revision was saved outside
    /// the original's folder, so read-only issued-stamp lookup can locate the
    /// original's journal without mutating anything. Omitted when the revision
    /// lives inside the original's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
    /// When chained onto an active working revision, identifies that parent
    /// working revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<ParentRevision>,
    pub revision: RevisionFile,
    pub sheet: String,
    pub units: Option<Unit>,
    pub decode_report: DecodeReport,
    pub allowed_loss: bool,
    pub change_set: Value,
    pub operations: Vec<RevisionOperationOutcome>,
    pub unexpected_changes: Vec<UnexpectedChange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_error: Option<Error>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Reply<CheckReport>>,
}

/// Re-export [`UnexpectedChange`] and [`UnexpectedChangeKind`] at the core API
/// without leaking `dwg`.
pub use crate::dwg::{
    UnexpectedChange as RevisionUnexpectedChange,
    UnexpectedChangeKind as RevisionUnexpectedChangeKind,
};

/// A request to apply a change set to an original `.dwg` file, saving a new
/// revision beside it (or at `output`) without modifying the original.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviseRequest {
    /// The original `.dwg` file to revise.
    pub set: PathBuf,
    /// The change set, as plain data: `{"operations": [...]}`.
    pub change_set: Value,
    /// Report what the revision would do, and write nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
    /// Where to write the revision `.dwg` (must end `.dwg`, never overwriting
    /// the original or any existing file). Without it, `<stem>.rev1.dwg`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    /// Allow revising an original the DWG library could not decode whole,
    /// recording the accepted losses in the reply and report (ADR-0004).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_loss: bool,
    /// Expected SHA-256 hex fingerprint of the original `.dwg` bytes read from
    /// disk. When provided, `change` refuses with `changed_by_hand` if the
    /// original's bytes do not match, even when `allow_loss` is true (ADR-0004).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_original_sha256: Option<String>,
    /// CTB plot style file name (such as `monochrome.ctb`) for the revision PDF.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_style: Option<String>,
    /// Skip plotting the revision PDF beside the revision.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_pdf: bool,
}

/// Applies a change set to an original DWG using environment settings.
pub fn revise(request: &ReviseRequest) -> Reply<RevisionReport> {
    revise_with(request, &Settings::from_env())
}

/// Applies a change set to an original DWG, writing a new revision DWG (in the
/// original's exact DWG version), its `.changes.json` change list & report, and
/// (unless `no_pdf` or `dry_run`) its revision PDF. Never modifies the original.
///
/// On non-dry-run calls, acquires the original's journal lock, rolls back any
/// unfinished pending commit first (reporting `rolled_back`), commits the
/// revision write-ahead (`pending` -> atomic file placement -> `done`), and
/// records a `failed_attempt` entry on refusal or error. On `--dry-run`, takes
/// no write lock, writes no journal entry, and refuses if an unfinished
/// pending commit exists.
pub fn revise_with(request: &ReviseRequest, settings: &Settings) -> Reply<RevisionReport> {
    let source = request.set.as_path();
    if !is_drawing_file(source) {
        return Reply::from(Err(Error::usage(
            "revising an original requires a `.dwg` or `.dxf` file, not a folder",
        )
        .in_file(source)));
    }

    // Single read of the original DWG: open once, capture identity, read bytes.
    let mut source_file = match File::open(source) {
        Ok(f) => f,
        Err(e) => {
            return Reply::from(Err(Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the DWG: {e}"),
            )
            .in_file(source)));
        }
    };
    #[cfg(any(unix, windows))]
    let source_id = file_id_of_file(&source_file);
    #[cfg(not(any(unix, windows)))]
    let source_id: Option<FileId> = None;
    let mut original_bytes = Vec::new();
    if let Err(e) = source_file.read_to_end(&mut original_bytes) {
        return Reply::from(Err(Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the DWG: {e}"),
        )
        .in_file(source)));
    }
    drop(source_file);

    let resolved_source = on_disk_path(source, source_id);
    let oj = OriginalJournal::from_opened(source, &resolved_source, source_id);

    if request.dry_run {
        if let Err(error) = oj.refuse_unfinished() {
            return Reply::from(Err(error));
        }
        return match prepare_revision(
            request,
            settings,
            &oj,
            &resolved_source,
            &original_bytes,
            source_id,
        ) {
            Ok(PreparedRevision::Ready(ready)) => Reply::from(Ok(*ready.report)),
            Ok(PreparedRevision::Refused(report, error)) => Reply::partly(*report, error),
            Err(error) => Reply::from(Err(error)),
        };
    }

    // Hold the original's journal lock across crash recovery -> revision
    // computation -> atomic file placement -> journal `done` (or failed attempt).
    let locked = match oj.lock() {
        Ok(lock) => lock,
        Err(error) => return Reply::from(Err(error)),
    };
    let rolled_back = match oj.recover_locked(&locked) {
        Ok(rb) => rb,
        Err(error) => return Reply::from(Err(error)),
    };

    let reply = match prepare_revision(
        request,
        settings,
        &oj,
        &resolved_source,
        &original_bytes,
        source_id,
    ) {
        Ok(PreparedRevision::Ready(mut ready)) => {
            let mut new_files: Vec<(&Path, &[u8], &'static str)> = vec![
                (&ready.paths.dwg, &ready.revision_bytes, "revision DWG"),
                (
                    &ready.paths.changes,
                    &ready.change_list_bytes,
                    "revision change list",
                ),
            ];
            if let (Some(pdf_path), Some(pdf_bytes)) =
                (ready.paths.pdf.as_deref(), ready.pdf_bytes.as_deref())
            {
                new_files.push((pdf_path, pdf_bytes, "revision PDF"));
            }
            let objects = objects_changed_from_outcomes(&ready.report.operations);
            let rev_meta = ready
                .report
                .revision
                .as_ref()
                .expect("ready revision has metadata");
            let meta = OriginalRevisionMeta {
                original_file: ready.report.original.file.clone(),
                original_sha256: ready.report.original.sha256.clone(),
                dwg_version: rev_meta.dwg_version.clone(),
                revision_dwg: oj.rel_path(&ready.paths.dwg),
                revision_sha256: rev_meta.sha256.clone(),
                allowed_loss: ready.report.allowed_loss,
                standalone_output: Some(request.output.is_some()),
                revision_path: None,
            };
            match oj.commit_revision(&locked, &request.change_set, objects, meta, &new_files) {
                Ok(entry_num) => {
                    ready.report.journal_entry = Some(entry_num);
                    Reply::from(Ok(*ready.report))
                }
                Err(error) => {
                    let _ = oj.record_failed_attempt(&locked, &request.change_set, &error);
                    Reply::from(Err(error))
                }
            }
        }
        Ok(PreparedRevision::Refused(mut report, error)) => {
            report.journal_entry = oj.record_failed_attempt(&locked, &request.change_set, &error);
            Reply::partly(*report, error)
        }
        Err(error) => {
            let _ = oj.record_failed_attempt(&locked, &request.change_set, &error);
            Reply::from(Err(error))
        }
    };
    reply.after(rolled_back)
}

enum PreparedRevision {
    Ready(Box<ReadyRevision>),
    Refused(Box<RevisionReport>, Error),
}

struct ReadyRevision {
    paths: OutputPaths,
    revision_bytes: Vec<u8>,
    change_list_bytes: Vec<u8>,
    pdf_bytes: Option<Vec<u8>>,
    report: Box<RevisionReport>,
}

enum OpenedRevisionBase {
    Open {
        session: Box<RevisionSession>,
        original_meta: RevisionFile,
        parent_revision: Option<ParentRevision>,
        decode: DecodeReport,
        units: Option<Unit>,
    },
    Refused(Box<RevisionReport>, Error),
}

#[inline(never)]
fn open_session_boxed(bytes: &[u8], path: &Path) -> Result<Box<RevisionSession>, Error> {
    RevisionSession::open(bytes, path).map(Box::new)
}

#[inline(never)]
fn inspect_original_header_and_decode(
    original_bytes: &[u8],
    source: &Path,
) -> Result<(String, Option<Unit>, DecodeReport), Error> {
    let session = open_session_boxed(original_bytes, source)?;
    session
        .check_writable_version()
        .map_err(|e| e.or_in_file(source))?;
    Ok((
        session.dwg_version().to_owned(),
        session.initial_contents().units,
        session.decode_report().clone(),
    ))
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn build_incomplete_decode_refusal(
    source: &Path,
    sheet_name: String,
    dry_run: bool,
    original_meta: RevisionFile,
    parent_revision: Option<ParentRevision>,
    units: Option<Unit>,
    decode: DecodeReport,
) -> OpenedRevisionBase {
    let detail = decode_refusal_message(source, &decode);
    let report = Box::new(RevisionReport {
        source: source.display().to_string(),
        sheet: sheet_name,
        dry_run,
        journal_entry: None,
        message: detail.clone(),
        original: original_meta,
        parent_revision,
        revision: None,
        revision_dwg: None,
        change_list: None,
        pdf: None,
        pdf_error: None,
        plot: None,
        units,
        decode,
        allowed_loss: false,
        operations: Vec::new(),
        requested_failures: Vec::new(),
        unexpected_changes: Vec::new(),
        checks: None,
    });
    let error = Error::new(ErrorCode::IncompleteDecode, detail).in_file(source);
    OpenedRevisionBase::Refused(report, error)
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn open_revision_base(
    request: &ReviseRequest,
    oj: &OriginalJournal,
    source: &Path,
    source_id: Option<FileId>,
    original_bytes: &[u8],
    original_file_name: String,
    original_sha256: String,
    sheet_name: &str,
) -> Result<OpenedRevisionBase, Error> {
    // When no explicit `--output` is given (sequential working revisions
    // `<stem>.rev1.dwg`, `<stem>.rev2.dwg`, ...) and this original already has
    // an active non-standalone working revision matching this exact original
    // SHA-256, chain this change set on top of the verified active working
    // revision so successive edits accumulate instead of dropping earlier
    // edits, while keeping the original file strictly read-only and preserving
    // the original's DWG version and DecodeReport (ADR-0004, ADR-0006).
    let active_tip = if request.output.is_none() {
        oj.latest_active_revision()?
            .filter(|tip| tip.original_sha256 == original_sha256)
    } else {
        None
    };

    if let Some(tip) = active_tip {
        let rev_bytes = match read_regular_no_follow(source, source_id, &tip.path) {
            Ok(NoFollowRead::File(bytes)) => bytes,
            Ok(NoFollowRead::Missing) => {
                return Err(Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "cannot apply a further revision to `{}`: its active working revision \
                         `{}` (from entry {}) is missing on disk; restore it or undo entry {} \
                         first",
                        source.display(),
                        tip.path.display(),
                        tip.entry,
                        tip.entry
                    ),
                )
                .in_file(&tip.path));
            }
            Ok(NoFollowRead::Symlink | NoFollowRead::NotRegular | NoFollowRead::SameAsOriginal) => {
                return Err(Error::new(
                    ErrorCode::ChangedByHand,
                    format!(
                        "cannot apply a further revision to `{}`: its active working revision \
                         `{}` (from entry {}) is no longer a regular revision file; undo entry {} \
                         first",
                        source.display(),
                        tip.path.display(),
                        tip.entry,
                        tip.entry
                    ),
                )
                .in_file(&tip.path));
            }
            Err(e) => {
                return Err(Error::new(
                    ErrorCode::ReadFailed,
                    format!(
                        "could not read active working revision `{}`: {e}",
                        tip.path.display()
                    ),
                )
                .in_file(&tip.path));
            }
        };
        let actual_rev_sha = fingerprint(&rev_bytes);
        if actual_rev_sha != tip.revision_sha256 {
            return Err(Error::new(
                ErrorCode::ChangedByHand,
                format!(
                    "cannot apply a further revision to `{}`: its active working revision `{}` \
                     (from entry {}) was modified outside AgentDrafter since it was written; \
                     restore it or undo entry {} first",
                    source.display(),
                    tip.path.display(),
                    tip.entry,
                    tip.entry
                ),
            )
            .in_file(&tip.path));
        }
        // Inspect the untouched original in a separate leaf frame so its
        // `DecodeReport` stays authoritative (a clean original is never locked
        // just because an earlier revision passed `--allow-loss`, and a damaged
        // original keeps its full decode report in chained revisions).
        let (orig_version, orig_units, orig_decode) =
            inspect_original_header_and_decode(original_bytes, source)?;
        let original_meta = RevisionFile {
            file: original_file_name,
            sha256: original_sha256,
            dwg_version: orig_version,
        };
        let parent_revision = Some(ParentRevision {
            entry: tip.entry,
            file: tip.file,
            sha256: tip.revision_sha256,
        });
        if !orig_decode.decoded && !request.allow_loss {
            return Ok(build_incomplete_decode_refusal(
                source,
                sheet_name.to_owned(),
                request.dry_run,
                original_meta,
                parent_revision,
                orig_units,
                orig_decode,
            ));
        }
        let tip_session = open_session_boxed(&rev_bytes, &tip.path)?;
        tip_session
            .check_writable_version()
            .map_err(|e| e.or_in_file(source))?;
        let units = tip_session.initial_contents().units.or(orig_units);
        // If the original was clean (`decoded: true`) but the tip somehow failed
        // to decode whole, enforce the decode gate on the tip too. Otherwise
        // unconditionally preserve `orig_decode` so non-fatal warnings on a
        // `decoded: true` original (such as header code-page warnings) are never
        // dropped by chaining through `rev1`.
        if orig_decode.decoded && !tip_session.decode_report().decoded && !request.allow_loss {
            let tip_decode = tip_session.decode_report().clone();
            return Ok(build_incomplete_decode_refusal(
                source,
                sheet_name.to_owned(),
                request.dry_run,
                original_meta,
                parent_revision,
                units,
                tip_decode,
            ));
        }
        Ok(OpenedRevisionBase::Open {
            session: tip_session,
            original_meta,
            parent_revision,
            decode: orig_decode,
            units,
        })
    } else {
        let orig_session = open_session_boxed(original_bytes, source)?;
        orig_session
            .check_writable_version()
            .map_err(|e| e.or_in_file(source))?;
        let original_meta = RevisionFile {
            file: original_file_name,
            sha256: original_sha256,
            dwg_version: orig_session.dwg_version().to_owned(),
        };
        let decode = orig_session.decode_report().clone();
        let units = orig_session.initial_contents().units;
        if !decode.decoded && !request.allow_loss {
            return Ok(build_incomplete_decode_refusal(
                source,
                sheet_name.to_owned(),
                request.dry_run,
                original_meta,
                None,
                units,
                decode,
            ));
        }
        Ok(OpenedRevisionBase::Open {
            session: orig_session,
            original_meta,
            parent_revision: None,
            decode,
            units,
        })
    }
}

#[inline(never)]
fn serialize_and_verify_boxed(
    session: &RevisionSession,
    dwg_path: &Path,
) -> Result<Box<RevisionVerification>, Error> {
    session.serialize_and_verify(dwg_path).map(Box::new)
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn build_ready_revision(
    request: &ReviseRequest,
    oj: &OriginalJournal,
    source: &Path,
    sheet_name: String,
    paths: OutputPaths,
    original_meta: RevisionFile,
    parent_revision: Option<ParentRevision>,
    revision_meta: RevisionFile,
    revision_bytes: Vec<u8>,
    units: Option<Unit>,
    decode: DecodeReport,
    outcomes: Vec<RevisionOperationOutcome>,
    unexpected_changes: Vec<UnexpectedChange>,
    checks: Option<Reply<CheckReport>>,
    pdf_bytes: Option<Vec<u8>>,
    plot_report: Option<PlotReport>,
    pdf_error: Option<Error>,
    pdf_written_path: Option<String>,
) -> Result<Box<ReadyRevision>, Error> {
    let original_path = if oj.is_directly_in_orig_dir(&paths.dwg) {
        None
    } else {
        Some(oj.resolved_source.to_string_lossy().into_owned())
    };
    let change_list_file = RevisionChangeListFile {
        format: 1,
        original: original_meta.clone(),
        original_path,
        parent_revision: parent_revision.clone(),
        revision: revision_meta.clone(),
        sheet: sheet_name.clone(),
        units,
        decode_report: decode.clone(),
        allowed_loss: request.allow_loss,
        change_set: request.change_set.clone(),
        operations: outcomes.clone(),
        unexpected_changes: unexpected_changes.clone(),
        pdf: pdf_written_path.clone(),
        pdf_error: pdf_error.clone(),
        checks: checks.clone(),
    };
    let mut change_list_bytes = serde_json::to_vec_pretty(&change_list_file).map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not serialize the revision change list: {e}"),
        )
    })?;
    change_list_bytes.push(b'\n');

    let message = revision_message(
        source,
        &paths.dwg,
        &revision_meta.dwg_version,
        outcomes.len(),
        request.dry_run,
        !decode.decoded && request.allow_loss,
        unexpected_changes.len(),
        pdf_written_path.as_deref(),
        pdf_error.as_ref(),
    );

    let report = Box::new(RevisionReport {
        source: source.display().to_string(),
        sheet: sheet_name,
        dry_run: request.dry_run,
        journal_entry: None,
        message,
        original: original_meta,
        parent_revision,
        revision: Some(revision_meta),
        revision_dwg: Some(paths.dwg.display().to_string()),
        change_list: Some(paths.changes.display().to_string()),
        pdf: pdf_written_path,
        pdf_error,
        plot: plot_report,
        units,
        decode,
        allowed_loss: request.allow_loss,
        operations: outcomes,
        requested_failures: Vec::new(),
        unexpected_changes,
        checks,
    });
    Ok(Box::new(ReadyRevision {
        paths,
        revision_bytes,
        change_list_bytes,
        pdf_bytes,
        report,
    }))
}

#[inline(never)]
fn prepare_revision(
    request: &ReviseRequest,
    settings: &Settings,
    oj: &OriginalJournal,
    resolved_source: &Path,
    original_bytes: &[u8],
    source_id: Option<FileId>,
) -> Result<PreparedRevision, Error> {
    let source = request.set.as_path();

    // Validate plot style early (even in dry-run) so bad plot-style options fail
    // cleanly before doing any work.
    let style = if request.no_pdf {
        if request.plot_style.is_some() {
            return Err(Error::usage(
                "`plot_style` names a plot style for the revision PDF, and `no_pdf` skips \
                 plotting it; give one or the other",
            )
            .at_field("plot_style"));
        }
        None
    } else {
        Some(plot_style(request.plot_style.as_deref(), settings)?)
    };

    let original_sha256 = fingerprint(original_bytes);
    // Core-enforced SHA-256 binding (ADR-0004): refuse even when `allow_loss`
    // is true if the original file on disk no longer matches the exact content
    // the caller inspected and approved.
    if let Some(expected) = request.expected_original_sha256.as_deref() {
        let trimmed = expected.trim();
        if trimmed.is_empty() || !original_sha256.eq_ignore_ascii_case(trimmed) {
            return Err(Error::new(
                ErrorCode::ChangedByHand,
                format!(
                    "the original DWG `{}` changed on disk (expected sha256 `{}`, found `{}`): \
                     re-read and re-confirm before revising",
                    source.display(),
                    trimmed,
                    original_sha256
                ),
            )
            .in_file(source));
        }
    }

    let sheet_name = oj.sheet.clone();
    let original_file_name = resolved_source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());

    // Validate/choose output paths before touching anything on disk.
    let paths = resolve_output_paths(source, source_id, request.output.as_deref(), request.no_pdf)?;

    let (mut session, original_meta, parent_revision, decode, units) = match open_revision_base(
        request,
        oj,
        source,
        source_id,
        original_bytes,
        original_file_name,
        original_sha256,
        &sheet_name,
    )? {
        OpenedRevisionBase::Open {
            session,
            original_meta,
            parent_revision,
            decode,
            units,
        } => (session, original_meta, parent_revision, decode, units),
        OpenedRevisionBase::Refused(report, error) => {
            return Ok(PreparedRevision::Refused(report, error));
        }
    };
    let effective_units = units.unwrap_or(Unit::Mm);

    // Refuse drawing-set-only change-set keys (`fixes`, `guide`) on originals.
    if let Value::Object(map) = &request.change_set {
        if map
            .get("fixes")
            .is_some_and(|v| !v.as_array().is_some_and(Vec::is_empty))
        {
            return Err(Error::usage(
                "`fixes` clears a drawing set's inconsistent mark and is not used when revising \
                 an original DWG",
            )
            .at_field("fixes"));
        }
        if map.contains_key("guide") {
            return Err(Error::usage(
                "`guide` belongs to a drawing set and is not used when revising an original DWG",
            )
            .at_field("guide"));
        }
    }

    // Parse all operations with the drawing's units and default sheet name.
    let parsed = read_change_set_for_original(&request.change_set, effective_units, &sheet_name)?;

    // Apply every operation in order to the in-memory CadDocument.
    // All-or-nothing (ADR-0005): any error returns before writing a single file.
    let mut outcomes = Vec::with_capacity(parsed.operations.len());
    let mut cached_record: Option<DwgLoaded> = None;

    for op in &parsed.operations {
        let outcome = match &op.body {
            Body::AttachXref {
                to,
                xref,
                place,
                position,
                scale,
                rotation,
                layer,
                space,
                id,
                insert_name,
            } => revise_attach_xref(
                &mut session,
                &sheet_name,
                effective_units,
                op,
                to,
                xref,
                *place,
                position,
                *scale,
                *rotation,
                layer,
                space.as_deref(),
                id.as_deref(),
                insert_name.as_deref(),
            )?,
            Body::DetachXref { to, name } => {
                revise_detach_xref(&mut session, &sheet_name, op, to, name)?
            }
            Body::BindXref { to, name, mode } => revise_bind_xref(
                &mut session,
                source,
                &sheet_name,
                effective_units,
                op,
                to,
                name,
                *mode,
                settings,
            )?,
            Body::AddGroup {
                to,
                name,
                targets,
                description,
                selectable,
            } => revise_add_group(
                &mut session,
                &mut cached_record,
                source,
                &sheet_name,
                op,
                to,
                name,
                targets,
                description,
                *selectable,
            )?,
            Body::Ungroup { to, name } => revise_ungroup(&mut session, &sheet_name, op, to, name)?,
            Body::EditGroup {
                to,
                name,
                add,
                remove,
                rename,
                description,
                selectable,
            } => revise_edit_group(
                &mut session,
                &mut cached_record,
                source,
                &sheet_name,
                op,
                to,
                name,
                add,
                remove,
                rename.as_deref(),
                description.as_deref(),
                *selectable,
            )?,
            Body::SetAnnotationScale {
                to,
                scale,
                anno_all_visible,
                add_scales,
                remove_scales,
            } => {
                check_add_target_sheet(to, &sheet_name, op)?;
                let (msg, handles) = session
                    .set_annotation_scale(
                        scale.as_deref(),
                        *anno_all_visible,
                        add_scales,
                        remove_scales,
                    )
                    .map_err(|e| {
                        let f = e.field.as_deref().unwrap_or("");
                        op_error(op.index, op.spec.name, e.code, e.message, f)
                    })?;
                RevisionOperationOutcome {
                    index: op.index,
                    op: op.spec.name.to_owned(),
                    ok: true,
                    message: msg,
                    id: None,
                    handle: handles.touched.first().cloned(),
                    handles_touched: handles.touched,
                    handles_added: handles.added,
                    handles_deleted: handles.deleted,
                    copies: Vec::new(),
                    snapped: Vec::new(),
                    replacements: Vec::new(),
                    attributes_set: Vec::new(),
                    split_id: None,
                    deleted: Vec::new(),
                    transformed: Vec::new(),
                    created: Vec::new(),
                }
            }
            Body::SetPointStyle { to, pdmode, pdsize } => {
                check_add_target_sheet(to, &sheet_name, op)?;
                let msg = session.set_point_style(*pdmode, *pdsize).map_err(|e| {
                    let f = e.field.as_deref().unwrap_or("");
                    op_error(op.index, op.spec.name, e.code, e.message, f)
                })?;
                RevisionOperationOutcome {
                    index: op.index,
                    op: op.spec.name.to_owned(),
                    ok: true,
                    message: msg,
                    id: None,
                    handle: None,
                    handles_touched: Vec::new(),
                    handles_added: Vec::new(),
                    handles_deleted: Vec::new(),
                    copies: Vec::new(),
                    snapped: Vec::new(),
                    replacements: Vec::new(),
                    attributes_set: Vec::new(),
                    split_id: None,
                    deleted: Vec::new(),
                    transformed: Vec::new(),
                    created: Vec::new(),
                }
            }
            Body::AddMLineStyle { to, name, style } => {
                check_add_target_sheet(to, &sheet_name, op)?;
                let tables = session.tables_for_parsing();
                let parsed_style = crate::description::parse_mline_style_json(
                    name,
                    style,
                    units.unwrap_or(Unit::Mm),
                    &tables.linetypes,
                )
                .map_err(|e| {
                    let f = e.field.as_deref().unwrap_or("");
                    op_error(op.index, op.spec.name, e.code, e.message, f)
                })?;
                let (msg, handles) = session.add_mline_style(&parsed_style).map_err(|e| {
                    let f = e.field.as_deref().unwrap_or("");
                    op_error(op.index, op.spec.name, e.code, e.message, f)
                })?;
                RevisionOperationOutcome {
                    index: op.index,
                    op: op.spec.name.to_owned(),
                    ok: true,
                    message: msg,
                    id: None,
                    handle: handles
                        .added
                        .first()
                        .or_else(|| handles.touched.first())
                        .cloned(),
                    handles_touched: handles.touched,
                    handles_added: handles.added,
                    handles_deleted: handles.deleted,
                    copies: Vec::new(),
                    snapped: Vec::new(),
                    replacements: Vec::new(),
                    attributes_set: Vec::new(),
                    split_id: None,
                    deleted: Vec::new(),
                    transformed: Vec::new(),
                    created: Vec::new(),
                }
            }
            Body::SetObjectScales {
                targets,
                annotative,
                scales,
                add,
                remove,
            } => {
                let record = ensure_record(&session, &mut cached_record, source, &sheet_name)?;
                let mut hexes: Vec<String> = Vec::new();
                let mut ids: Vec<String> = Vec::new();
                for (target, field) in targets {
                    for found in
                        resolve_target_all(&session, record, target, &sheet_name, op, field)?
                    {
                        if !hexes.iter().any(|h| h.eq_ignore_ascii_case(&found.handle)) {
                            hexes.push(found.handle);
                            ids.push(found.id);
                        }
                    }
                }
                let handles = session
                    .set_object_scales(&hexes, *annotative, scales.as_deref(), add, remove)
                    .map_err(|e| {
                        let f = e.field.as_deref().unwrap_or("target");
                        op_error(op.index, op.spec.name, e.code, e.message, f)
                    })?;
                RevisionOperationOutcome {
                    index: op.index,
                    op: op.spec.name.to_owned(),
                    ok: true,
                    message: format!("updated annotation scales on {}", ids.join(", ")),
                    id: (ids.len() == 1).then(|| ids[0].clone()),
                    handle: handles.touched.first().cloned(),
                    handles_touched: handles.touched,
                    handles_added: handles.added,
                    handles_deleted: handles.deleted,
                    copies: Vec::new(),
                    snapped: Vec::new(),
                    replacements: Vec::new(),
                    attributes_set: Vec::new(),
                    split_id: None,
                    deleted: Vec::new(),
                    transformed: ids,
                    created: Vec::new(),
                }
            }
            _ => apply_revision_op(
                &mut session,
                &mut cached_record,
                source,
                &sheet_name,
                effective_units,
                op,
                settings,
            )?,
        };
        outcomes.push(outcome);
        // Invalidate cached SheetRecord so subsequent ops see updated geometry/IDs.
        cached_record = None;
    }

    // Serialize revision bytes in the original's DWG version, re-read strictly,
    // and run the structural diff self-check.
    let verified = serialize_and_verify_boxed(&session, &paths.dwg)?;
    if let Some((report, error)) = enforce_verification_gates(
        source,
        &sheet_name,
        request.dry_run,
        request.allow_loss,
        &original_meta,
        parent_revision.as_ref(),
        units,
        &decode,
        &outcomes,
        &verified,
    ) {
        return Ok(PreparedRevision::Refused(report, error));
    }
    let revision_bytes = verified.bytes;
    let unexpected_changes = verified.unexpected_changes;
    let revision_dwg_version = session.dwg_version().to_owned();
    drop(session);

    let revision_sha256 = fingerprint(&revision_bytes);
    let revision_file_name = paths
        .dwg
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| paths.dwg.display().to_string());
    let revision_meta = RevisionFile {
        file: revision_file_name,
        sha256: revision_sha256,
        dwg_version: revision_dwg_version,
    };

    // Run checker on the serialized revision bytes (works in dry-run too!).
    let checks = Some(Reply::from(check_dwg_bytes(
        &paths.dwg,
        sheet_name.clone(),
        &revision_bytes,
        settings,
    )));

    // Plot revision PDF in memory unless `--no-pdf` or `--dry-run`.
    let mut pdf_bytes: Option<Vec<u8>> = None;
    let mut plot_report: Option<PlotReport> = None;
    let mut pdf_error: Option<Error> = None;
    let mut pdf_written_path: Option<String> = None;
    if !request.dry_run {
        if let (Some(pdf_path), Some(style)) = (&paths.pdf, &style) {
            match plot_dwg_bytes(&paths.dwg, &revision_bytes, pdf_path, style, settings) {
                Ok((bytes, report)) => {
                    pdf_bytes = Some(bytes);
                    plot_report = Some(report);
                    pdf_written_path = Some(pdf_path.display().to_string());
                }
                Err(err) => {
                    pdf_error = Some(err);
                }
            }
        }
    }

    let ready = build_ready_revision(
        request,
        oj,
        source,
        sheet_name,
        paths,
        original_meta,
        parent_revision,
        revision_meta,
        revision_bytes,
        units,
        decode,
        outcomes,
        unexpected_changes,
        checks,
        pdf_bytes,
        plot_report,
        pdf_error,
        pdf_written_path,
    )?;
    Ok(PreparedRevision::Ready(ready))
}

fn objects_changed_from_outcomes(outcomes: &[RevisionOperationOutcome]) -> Vec<ObjectChanged> {
    let mut out: Vec<ObjectChanged> = Vec::new();
    for outcome in outcomes {
        if !outcome.copies.is_empty() {
            for copy in &outcome.copies {
                out.push(ObjectChanged {
                    id: copy.id.clone(),
                    how: HowChanged::Added,
                });
            }
            continue;
        }
        if !outcome.created.is_empty()
            || !outcome.deleted.is_empty()
            || !outcome.transformed.is_empty()
        {
            for id in &outcome.deleted {
                out.push(ObjectChanged {
                    id: id.clone(),
                    how: HowChanged::Deleted,
                });
            }
            for id in &outcome.created {
                out.push(ObjectChanged {
                    id: id.clone(),
                    how: HowChanged::Added,
                });
            }
            for id in &outcome.transformed {
                out.push(ObjectChanged {
                    id: id.clone(),
                    how: HowChanged::Changed,
                });
            }
            if let Some(id) = outcome.id.as_ref() {
                if !outcome.deleted.contains(id)
                    && !outcome.created.contains(id)
                    && !outcome.transformed.contains(id)
                {
                    out.push(ObjectChanged {
                        id: id.clone(),
                        how: HowChanged::Changed,
                    });
                }
            }
            continue;
        }
        let Some(id) = outcome.id.as_ref() else {
            continue;
        };
        let how = if outcome.op == "delete" {
            HowChanged::Deleted
        } else if outcome.op.starts_with("add_") || outcome.op == "insert_block" {
            HowChanged::Added
        } else {
            HowChanged::Changed
        };
        out.push(ObjectChanged {
            id: id.clone(),
            how,
        });
    }
    out
}

// ── Output path selection & safe non-clobbering placement ─────────────────

struct OutputPaths {
    dwg: PathBuf,
    changes: PathBuf,
    pdf: Option<PathBuf>,
}

fn with_dwg_replaced(dwg_path: &Path, suffix: &str) -> PathBuf {
    let stem = dwg_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    dwg_path.with_file_name(format!("{stem}.{suffix}"))
}

fn resolve_output_paths(
    source: &Path,
    source_id: Option<FileId>,
    explicit_output: Option<&Path>,
    no_pdf: bool,
) -> Result<OutputPaths, Error> {
    let source_is_dxf = is_dxf(source);
    let ext = if source_is_dxf { "dxf" } else { "dwg" };
    if let Some(output) = explicit_output {
        if output.file_name().is_none() {
            return Err(
                Error::usage("the revision's output path does not name a file")
                    .at_field("output")
                    .in_file(output),
            );
        }
        let matches = if source_is_dxf {
            is_dxf(output)
        } else {
            is_dwg(output)
        };
        if !matches {
            return Err(
                Error::usage(format!("the revision's output path must end `.{ext}`"))
                    .at_field("output")
                    .in_file(output),
            );
        }
        let changes = with_dwg_replaced(output, REVISION_CHANGES_EXTENSION);
        let pdf = (!no_pdf).then(|| with_dwg_replaced(output, "pdf"));
        refuse_existing_or_original(source, source_id, output, "revision drawing")?;
        refuse_existing_or_original(source, source_id, &changes, "revision change list")?;
        if let Some(pdf_path) = &pdf {
            refuse_existing_or_original(source, source_id, pdf_path, "revision PDF")?;
        }
        return Ok(OutputPaths {
            dwg: output.to_path_buf(),
            changes,
            pdf,
        });
    }

    // Default beside the original: `<stem>.rev1.<ext>`, `<stem>.rev2.<ext>`, ...
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    for n in 1u64..=100_000 {
        let dwg = source.with_file_name(format!("{stem}.rev{n}.{ext}"));
        let changes = source.with_file_name(format!("{stem}.rev{n}.{REVISION_CHANGES_EXTENSION}"));
        let pdf = (!no_pdf).then(|| source.with_file_name(format!("{stem}.rev{n}.pdf")));
        let dwg_free = path_absent(&dwg)?;
        let changes_free = path_absent(&changes)?;
        let pdf_free = match &pdf {
            Some(p) => path_absent(p)?,
            None => true,
        };
        if dwg_free && changes_free && pdf_free {
            return Ok(OutputPaths { dwg, changes, pdf });
        }
    }
    Err(Error::new(
        ErrorCode::WriteFailed,
        "could not find a free revision number beside the original; pass `--output` to choose a \
         path",
    )
    .in_file(source))
}

fn path_absent(path: &Path) -> Result<bool, Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(Error::new(
            ErrorCode::WriteFailed,
            format!("could not inspect `{}`: {e}", path.display()),
        )
        .in_file(path)),
    }
}

fn refuse_existing_or_original(
    source: &Path,
    source_id: Option<FileId>,
    candidate: &Path,
    what: &str,
) -> Result<(), Error> {
    // Check if candidate names the original without reopening `source` or
    // following symlinks (ADR-0004).
    let same_as_source = {
        #[cfg(any(unix, windows))]
        {
            let _ = source;
            source_id.is_some() && regular_file_id_no_follow(candidate) == source_id
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = source_id;
            is_same_file(source, candidate)
        }
    };
    if same_as_source {
        return Err(Error::usage(format!(
            "the {what} path is the original itself, which is never overwritten (ADR-0004); \
             choose another path"
        ))
        .in_file(candidate));
    }
    match fs::symlink_metadata(candidate) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(meta) if meta.file_type().is_symlink() => Err(Error::new(
            ErrorCode::OutputExists,
            format!(
                "the {what} path is a symbolic link; AgentDrafter never follows or overwrites a \
                 symbolic link, so choose another path"
            ),
        )
        .in_file(candidate)),
        Ok(meta) if meta.is_dir() => Err(Error::new(
            ErrorCode::OutputExists,
            format!("the {what} path is a folder, not a file; choose another path"),
        )
        .in_file(candidate)),
        Ok(_) => Err(Error::new(
            ErrorCode::OutputExists,
            format!(
                "a file already exists at the {what} path; revisions never overwrite existing \
                 files, so choose another path"
            ),
        )
        .in_file(candidate)),
        Err(e) => Err(Error::new(
            ErrorCode::WriteFailed,
            format!("could not inspect the {what} path: {e}"),
        )
        .in_file(candidate)),
    }
}

// ── Operation dispatch & target/snap resolution ───────────────────────────

fn ensure_record<'a>(
    session: &RevisionSession,
    cached: &'a mut Option<DwgLoaded>,
    source: &Path,
    sheet_name: &str,
) -> Result<&'a DwgLoaded, Error> {
    if cached.is_none() {
        let drawing = session.current_contents()?;
        *cached = Some(dwg_loaded_from_drawing(
            source,
            sheet_name.to_owned(),
            drawing,
            None,
        ));
    }
    Ok(cached.as_ref().expect("populated above"))
}

struct ResolvedTarget {
    id: String,
    handle: String,
    model_index: Option<usize>,
}

fn resolve_target(
    session: &RevisionSession,
    record: &DwgLoaded,
    target: &Target,
    sheet_name: &str,
    op: &Operation,
    field: &str,
) -> Result<ResolvedTarget, Error> {
    let dwg_origin = record
        .record
        .dwg
        .as_ref()
        .expect("DWG record has DwgOrigin");
    match target {
        Target::Id { sheet, id } => {
            let full = format!("{sheet}/{id}");
            for (i, obj_rec) in record.record.objects.iter().enumerate() {
                if obj_rec.id.eq_ignore_ascii_case(&full) {
                    return Ok(ResolvedTarget {
                        id: obj_rec.id.clone(),
                        handle: dwg_origin.handles[i].clone(),
                        model_index: Some(i),
                    });
                }
            }
            // Also allow targeting an unmodeled entity (or a canonical/zero-padded
            // hex handle alias) in this original directly by `{sheet}/handle:{HEX}`.
            if sheet.eq_ignore_ascii_case(sheet_name) {
                if let Some(hex) = id.strip_prefix(HANDLE_ID).or_else(|| {
                    (id.len() > HANDLE_ID.len()
                        && id.is_char_boundary(HANDLE_ID.len())
                        && id[..HANDLE_ID.len()].eq_ignore_ascii_case(HANDLE_ID))
                    .then(|| &id[HANDLE_ID.len()..])
                }) {
                    let trimmed = hex.trim();
                    if let Ok(num) = u64::from_str_radix(trimmed, 16) {
                        if num != 0 && session.locked_layer_of(trimmed).is_ok() {
                            let canonical = format!("{num:X}");
                            let model_index = dwg_origin
                                .handles
                                .iter()
                                .position(|h| h.eq_ignore_ascii_case(&canonical));
                            let resolved_id = model_index
                                .map(|i| record.record.objects[i].id.clone())
                                .unwrap_or_else(|| handle_id(sheet_name, &canonical));
                            return Ok(ResolvedTarget {
                                id: resolved_id,
                                handle: canonical,
                                model_index,
                            });
                        }
                    }
                }
            }
            if !sheet.eq_ignore_ascii_case(sheet_name) {
                return Err(op_error(
                    op.index,
                    op.spec.name,
                    ErrorCode::UnknownName,
                    format!(
                        "no sheet `{sheet}` in this original; the drawing's sheet is `{sheet_name}`"
                    ),
                    &format!("{field}.id"),
                ));
            }
            Err(op_error(
                op.index,
                op.spec.name,
                ErrorCode::UnknownName,
                format!("no object on `{sheet_name}` has ID `{full}`"),
                &format!("{field}.id"),
            ))
        }
        Target::Name { name, sheet } => {
            if let Some(s) = sheet {
                if !s.eq_ignore_ascii_case(sheet_name) {
                    return Err(op_error(
                        op.index,
                        op.spec.name,
                        ErrorCode::UnknownName,
                        format!(
                            "no sheet `{s}` in this original; the drawing's sheet is `{sheet_name}`"
                        ),
                        &format!("{field}.sheet"),
                    ));
                }
            }
            let matches: Vec<usize> = record
                .record
                .objects
                .iter()
                .enumerate()
                .filter(|(_, o)| {
                    o.name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(name))
                })
                .map(|(i, _)| i)
                .collect();
            match matches.as_slice() {
                [] => Err(op_error(
                    op.index,
                    op.spec.name,
                    ErrorCode::UnknownName,
                    format!("no object on `{sheet_name}` is named `{name}`"),
                    &format!("{field}.name"),
                )),
                [only] => Ok(ResolvedTarget {
                    id: record.record.objects[*only].id.clone(),
                    handle: dwg_origin.handles[*only].clone(),
                    model_index: Some(*only),
                }),
                many => {
                    let ids: Vec<String> = many
                        .iter()
                        .map(|&i| format!("`{}`", record.record.objects[i].id))
                        .collect();
                    Err(op_error(
                        op.index,
                        op.spec.name,
                        ErrorCode::BadValue,
                        format!(
                            "name `{name}` on `{sheet_name}` is ambiguous and matches {} objects: \
                             {}; name one by its ID instead",
                            many.len(),
                            ids.join(", ")
                        ),
                        &format!("{field}.name"),
                    ))
                }
            }
        }
        Target::Group { group, .. } => Err(op_error(
            op.index,
            op.spec.name,
            ErrorCode::BadValue,
            format!(
                "group `{group}` names a group of objects, which `{}` does not take as a single \
                 target; target one object by its ID or name",
                op.spec.name
            ),
            &format!("{field}.group"),
        )),
    }
}

/// Resolves `target`, expanding `Target::Group` into every member entity of the
/// named group in `ACAD_GROUP`.
fn resolve_target_all(
    session: &RevisionSession,
    record: &DwgLoaded,
    target: &Target,
    sheet_name: &str,
    op: &Operation,
    field: &str,
) -> Result<Vec<ResolvedTarget>, Error> {
    let Target::Group { group, sheet } = target else {
        return Ok(vec![resolve_target(
            session, record, target, sheet_name, op, field,
        )?]);
    };
    if let Some(s) = sheet {
        if !s.eq_ignore_ascii_case(sheet_name) {
            return Err(op_error(
                op.index,
                op.spec.name,
                ErrorCode::UnknownName,
                format!("no sheet `{s}` in this original; the drawing's sheet is `{sheet_name}`"),
                &format!("{field}.sheet"),
            ));
        }
    }
    let (canonical, member_handles) = session.group_member_handles(group).map_err(|e| {
        op_error(
            op.index,
            op.spec.name,
            e.code,
            e.message,
            &format!("{field}.group"),
        )
    })?;
    if member_handles.is_empty() {
        return Err(op_error(
            op.index,
            op.spec.name,
            ErrorCode::BadValue,
            format!("group `{canonical}` has no objects on `{sheet_name}`"),
            &format!("{field}.group"),
        ));
    }
    let dwg_origin = record
        .record
        .dwg
        .as_ref()
        .expect("DWG record has DwgOrigin");
    let mut out = Vec::with_capacity(member_handles.len());
    for hex in member_handles {
        let model_index = dwg_origin
            .handles
            .iter()
            .position(|h| h.eq_ignore_ascii_case(&hex));
        let id = model_index
            .map(|i| record.record.objects[i].id.clone())
            .unwrap_or_else(|| handle_id(sheet_name, &hex));
        out.push(ResolvedTarget {
            id,
            handle: hex,
            model_index,
        });
    }
    Ok(out)
}

fn canonical_handle_value(hex: &str) -> Option<u64> {
    u64::from_str_radix(hex.trim(), 16).ok().filter(|&v| v != 0)
}

fn resolve_unique_targets(
    session: &RevisionSession,
    record: &DwgLoaded,
    targets: &[(Target, String)],
    sheet_name: &str,
    op: &Operation,
) -> Result<Vec<ResolvedTarget>, Error> {
    let mut resolved = Vec::with_capacity(targets.len());
    let mut seen_handles: HashSet<u64> = HashSet::with_capacity(targets.len());
    for (target, field) in targets {
        let expanded = resolve_target_all(session, record, target, sheet_name, op, field)?;
        let from_group = matches!(target, Target::Group { .. });
        for found in expanded {
            let Some(handle_num) = canonical_handle_value(&found.handle) else {
                return Err(op_error(
                    op.index,
                    op.spec.name,
                    ErrorCode::BadValue,
                    format!("invalid handle `{}` on `{}`", found.handle, found.id),
                    field,
                ));
            };
            if !seen_handles.insert(handle_num) {
                if from_group {
                    continue;
                }
                return Err(op_error(
                    op.index,
                    op.spec.name,
                    ErrorCode::BadValue,
                    format!(
                        "object `{}` (handle `{:X}`) is listed more than once in the target list",
                        found.id, handle_num
                    ),
                    field,
                ));
            }
            check_unlocked_with_children(session, &found.handle, &found.id, op, field)?;
            resolved.push(found);
        }
    }
    Ok(resolved)
}

fn check_unlocked(
    session: &RevisionSession,
    handle: &str,
    id: &str,
    op: &Operation,
    field: &str,
) -> Result<(), Error> {
    if let Some(layer) = session.locked_layer_of(handle)? {
        return Err(op_error(
            op.index,
            op.spec.name,
            ErrorCode::Locked,
            format!("object `{id}` is on locked layer `{layer}`"),
            field,
        ));
    }
    Ok(())
}

fn check_unlocked_with_children(
    session: &RevisionSession,
    handle: &str,
    id: &str,
    op: &Operation,
    field: &str,
) -> Result<(), Error> {
    if let Some(LockedLayerHit {
        layer,
        attribute_tag,
    }) = session.locked_layer_with_children(handle)?
    {
        let message = match attribute_tag {
            Some(tag) => format!("object `{id}` has attribute `{tag}` on locked layer `{layer}`"),
            None => format!("object `{id}` is on locked layer `{layer}`"),
        };
        return Err(op_error(
            op.index,
            op.spec.name,
            ErrorCode::Locked,
            message,
            field,
        ));
    }
    Ok(())
}

fn resolve_op_point(
    point: &OpPoint,
    record: &DwgLoaded,
    settings: &Settings,
    op: &Operation,
    snapped: &mut Vec<SnapResolved>,
) -> Result<[f64; 3], Error> {
    match point {
        OpPoint::At(p) => Ok(*p),
        OpPoint::Snap(ps) => {
            let (obj, pt) = one_point(
                &[&record.record],
                &ps.snap,
                settings,
                &format!("operations[{}].{}", op.index, ps.field),
            )?;
            snapped.push(SnapResolved {
                field: ps.field.clone(),
                id: obj.id,
                snap: pt.snap,
                which: pt.which,
                at: pt.at,
            });
            Ok([pt.at.x, pt.at.y, pt.at.z])
        }
    }
}

fn resolve_motion(
    motion: &Motion,
    record: &DwgLoaded,
    settings: &Settings,
    op: &Operation,
    snapped: &mut Vec<SnapResolved>,
) -> Result<[f64; 3], Error> {
    match motion {
        Motion::By(delta) => Ok(*delta),
        Motion::Between { from, to } => {
            let a = resolve_op_point(from, record, settings, op, snapped)?;
            let b = resolve_op_point(to, record, settings, op, snapped)?;
            Ok([b[0] - a[0], b[1] - a[1], b[2] - a[2]])
        }
    }
}

fn check_add_target_sheet(to: &AddTo, sheet_name: &str, op: &Operation) -> Result<(), Error> {
    match to {
        AddTo::Template(_) => Err(op_error(
            op.index,
            op.spec.name,
            ErrorCode::Usage,
            "an original DWG has no template; omit `sheet` or name the drawing's sheet",
            "template",
        )),
        AddTo::Sheet(s) if s.eq_ignore_ascii_case(sheet_name) => Ok(()),
        AddTo::Sheet(s) => Err(op_error(
            op.index,
            op.spec.name,
            ErrorCode::UnknownName,
            format!("no sheet `{s}` in this original; the drawing's sheet is `{sheet_name}`"),
            "sheet",
        )),
    }
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn revise_attach_xref(
    session: &mut RevisionSession,
    sheet_name: &str,
    units: Unit,
    op: &Operation,
    to: &AddTo,
    xref: &crate::model::Xref,
    place: bool,
    position: &Value,
    scale: f64,
    rotation: f64,
    layer: &str,
    space: Option<&str>,
    id: Option<&str>,
    insert_name: Option<&str>,
) -> Result<RevisionOperationOutcome, Error> {
    let index = op.index;
    let op_name = op.spec.name;
    check_add_target_sheet(to, sheet_name, op)?;
    let tables = session.tables_for_parsing();
    if !layer.eq_ignore_ascii_case(crate::model::LAYER_ZERO)
        && !tables
            .layers
            .iter()
            .any(|l| l.name.eq_ignore_ascii_case(layer))
    {
        return Err(op_error(
            index,
            op_name,
            ErrorCode::UnknownName,
            format!("unknown layer `{layer}` in the drawing"),
            "layer",
        ));
    }
    let target_space = match space {
        None => TargetSpace::Model,
        Some(s) if s.eq_ignore_ascii_case("model") => TargetSpace::Model,
        Some(s) if s.eq_ignore_ascii_case("paper") => {
            let first = tables.layouts.first().ok_or_else(|| {
                op_error(
                    index,
                    op_name,
                    ErrorCode::BadValue,
                    "the drawing has no paper-space layout",
                    "space",
                )
            })?;
            TargetSpace::Layout(first.clone())
        }
        Some(s) => {
            let found = tables
                .layouts
                .iter()
                .find(|l| l.eq_ignore_ascii_case(s))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("no layout named `{s}` in the drawing"),
                        "space",
                    )
                })?;
            TargetSpace::Layout(found.clone())
        }
    };
    let coords = match position {
        Value::Array(items) if items.len() == 3 => {
            let mut pts = [0.0; 3];
            for (i, item) in items.iter().enumerate() {
                pts[i] = parse_length_json(item, units, &format!("position[{i}]"))
                    .map_err(|e| op_error(index, op_name, e.code, e.message, "position"))?;
            }
            crate::model::Point3::new(pts[0], pts[1], pts[2])
        }
        _ => {
            return Err(op_error(
                index,
                op_name,
                ErrorCode::WrongType,
                "expected `position`, a 3D point `[x, y, z]`",
                "position",
            ));
        }
    };
    let insert = crate::model::Insert {
        block: xref.name.clone(),
        position: coords,
        scale,
        rotation: crate::description::normalize_degrees(rotation),
        mirrored: false,
        readable_attributes: false,
        attributes: Vec::new(),
        state: std::collections::BTreeMap::new(),
        evaluated_block: None,
        name: insert_name.map(str::to_owned),
        layer: layer.to_owned(),
        properties: Default::default(),
        annotative: false,
        annotation_scales: Vec::new(),
    };
    let (hex, handles) = if place {
        session
            .attach_xref(xref, &insert, &target_space, sheet_name, id, insert_name)
            .map_err(|e| op_error(index, op_name, e.code, e.message, "name"))?
    } else {
        let h = session
            .attach_xref_record_only(xref)
            .map_err(|e| op_error(index, op_name, e.code, e.message, "name"))?;
        (String::new(), h)
    };
    let obj_id = if place {
        Some(match id {
            Some(local) => format!("{sheet_name}/{local}"),
            None => handle_id(sheet_name, &hex),
        })
    } else {
        None
    };
    let msg = if place {
        format!(
            "attached {} external reference `{}` (`{}`) and placed insert `{}`",
            xref.kind.name(),
            xref.name,
            xref.path,
            obj_id.as_deref().unwrap_or(&hex)
        )
    } else {
        format!(
            "attached {} external reference `{}` (`{}`)",
            xref.kind.name(),
            xref.name,
            xref.path
        )
    };
    Ok(RevisionOperationOutcome {
        index,
        op: op_name.to_owned(),
        ok: true,
        message: msg,
        id: obj_id,
        handle: (!hex.is_empty()).then_some(hex),
        handles_touched: handles.touched,
        handles_added: handles.added,
        handles_deleted: handles.deleted,
        copies: Vec::new(),
        snapped: Vec::new(),
        replacements: Vec::new(),
        attributes_set: Vec::new(),
        split_id: None,
        deleted: Vec::new(),
        transformed: Vec::new(),
        created: Vec::new(),
    })
}

#[inline(never)]
fn revise_detach_xref(
    session: &mut RevisionSession,
    sheet_name: &str,
    op: &Operation,
    to: &AddTo,
    name: &str,
) -> Result<RevisionOperationOutcome, Error> {
    let index = op.index;
    let op_name = op.spec.name;
    check_add_target_sheet(to, sheet_name, op)?;
    let handles = session
        .detach_xref(name)
        .map_err(|e| op_error(index, op_name, e.code, e.message, "name"))?;
    Ok(RevisionOperationOutcome {
        index,
        op: op_name.to_owned(),
        ok: true,
        message: format!(
            "detached external reference `{name}` (removed {} insert{})",
            handles.deleted.len(),
            plural(handles.deleted.len())
        ),
        id: None,
        handle: None,
        handles_touched: handles.touched,
        handles_added: handles.added,
        handles_deleted: handles.deleted,
        copies: Vec::new(),
        snapped: Vec::new(),
        replacements: Vec::new(),
        attributes_set: Vec::new(),
        split_id: None,
        deleted: Vec::new(),
        transformed: Vec::new(),
        created: Vec::new(),
    })
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn revise_bind_xref(
    session: &mut RevisionSession,
    source: &Path,
    sheet_name: &str,
    units: Unit,
    op: &Operation,
    to: &AddTo,
    name: &str,
    mode: crate::model::XrefBindMode,
    settings: &Settings,
) -> Result<RevisionOperationOutcome, Error> {
    let index = op.index;
    let op_name = op.spec.name;
    check_add_target_sheet(to, sheet_name, op)?;
    let tables = session.tables_for_parsing();
    let style_sheet = crate::model::Sheet {
        units,
        block_library: None,
        linetypes: Vec::new(),
        layers: Vec::new(),
        text_styles: tables.styles,
        table_styles: Vec::new(),
        mleader_styles: Vec::new(),
        dimension_styles: Vec::new(),
        mline_styles: Vec::new(),
        pdmode: 0,
        pdsize: 0.0,
        blocks: Vec::new(),
        xrefs: Vec::new(),
        resolved_xrefs: Vec::new(),
        resolved_media: Vec::new(),
        resolved_shapes: Vec::new(),
        objects: Vec::new(),
        paper: None,
        page_setup: None,
        ltscale: 1.0,
        layout: None,
        other_layouts: Vec::new(),
        wipeout_frame: crate::model::WipeoutFrame::Off,
        image_frame: crate::model::WipeoutFrame::Off,
        custom_properties: std::collections::BTreeMap::new(),
        groups: Vec::new(),
        annotation_scales: Vec::new(),
        annotation_scale: None,
        anno_all_visible: None,
    };
    let fonts = SheetFonts::find(&style_sheet, settings);
    let start_of = |t: &crate::text::Text| fonts.start(t);
    let handles = session
        .bind_xref(name, source, mode, &start_of)
        .map_err(|e| op_error(index, op_name, e.code, e.message, "name"))?;
    Ok(RevisionOperationOutcome {
        index,
        op: op_name.to_owned(),
        ok: true,
        message: format!(
            "bound external reference `{name}` into a local block (mode: {})",
            mode.name()
        ),
        id: None,
        handle: handles.touched.first().cloned(),
        handles_touched: handles.touched,
        handles_added: handles.added,
        handles_deleted: handles.deleted,
        copies: Vec::new(),
        snapped: Vec::new(),
        replacements: Vec::new(),
        attributes_set: Vec::new(),
        split_id: None,
        deleted: Vec::new(),
        transformed: Vec::new(),
        created: Vec::new(),
    })
}

fn apply_revision_op(
    session: &mut RevisionSession,
    cached: &mut Option<DwgLoaded>,
    source: &Path,
    sheet_name: &str,
    units: Unit,
    op: &Operation,
    settings: &Settings,
) -> Result<RevisionOperationOutcome, Error> {
    let op_name = op.spec.name;
    let index = op.index;
    match &op.body {
        Body::Add {
            to,
            keys,
            snaps,
            draw_order,
        } => {
            check_add_target_sheet(to, sheet_name, op)?;
            let kind = match op.spec.kind {
                crate::change::operations::Kind::Add(object, None)
                    if matches!(
                        object,
                        "line"
                            | "polyline"
                            | "arc"
                            | "circle"
                            | "ellipse"
                            | "spline"
                            | "hatch"
                            | "text"
                            | "mtext"
                            | "donut"
                            | "solid"
                            | "revision_cloud"
                            | "wipeout"
                            | "image"
                            | "pdf_underlay"
                            | "point"
                            | "xline"
                            | "ray"
                            | "mline"
                            | "tolerance"
                            | "center_mark"
                            | "center_line"
                            | "shape"
                            | "solid3d"
                            | "insert"
                    ) =>
                {
                    object
                }
                crate::change::operations::Kind::AddBox
                | crate::change::operations::Kind::AddCylinder => "solid3d",
                _ => {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::Usage,
                        format!(
                            "operation `{op_name}` is not supported when revising an original DWG \
                             yet"
                        ),
                        "",
                    ))
                }
            };
            let record = ensure_record(session, cached, source, sheet_name)?;
            let mut resolved_keys = keys.clone();
            let mut snapped = Vec::new();
            for ps in snaps {
                let (obj, pt) = one_point(
                    &[&record.record],
                    &ps.snap,
                    settings,
                    &format!("operations[{index}].{}", ps.field),
                )?;
                let mut root = Value::Object(resolved_keys);
                if let Some(slot) = root.pointer_mut(&ps.pointer) {
                    *slot = json!([pt.at.x, pt.at.y, pt.at.z]);
                }
                resolved_keys = match root {
                    Value::Object(m) => m,
                    _ => unreachable!(),
                };
                snapped.push(SnapResolved {
                    field: ps.field.clone(),
                    id: obj.id,
                    snap: pt.snap,
                    which: pt.which,
                    at: pt.at,
                });
            }
            let tables = session.tables_for_parsing();
            let parsed = parse_added_object_json(
                kind,
                &resolved_keys,
                units,
                &tables.layers,
                &tables.linetypes,
                &tables.styles,
                &tables.blocks,
                &tables.layouts,
                &tables.dimension_styles,
                &tables.mline_styles,
            )
            .map_err(|e| {
                let field = e.field.as_deref().unwrap_or("");
                let trimmed = field.strip_prefix(&format!("{kind}.")).unwrap_or(field);
                op_error(index, op_name, e.code, e.message, trimmed)
            })?;

            // Enforce uniqueness of explicit `id` and `name` against the sheet.
            if let Some(explicit_id) = &parsed.id {
                let full = format!("{sheet_name}/{explicit_id}");
                if record
                    .record
                    .objects
                    .iter()
                    .any(|o| o.id.eq_ignore_ascii_case(&full))
                {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::DuplicateId,
                        format!("an object with ID `{full}` already exists on `{sheet_name}`"),
                        "id",
                    ));
                }
            }
            if let Some(explicit_name) = &parsed.name {
                if record.record.objects.iter().any(|o| {
                    o.name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(explicit_name))
                }) {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::DuplicateName,
                        format!(
                            "an object named `{explicit_name}` already exists on `{sheet_name}`"
                        ),
                        "name",
                    ));
                }
            }
            let space = match parsed.space {
                None => TargetSpace::Model,
                Some(idx) => TargetSpace::Layout(tables.layouts[idx].clone()),
            };
            let resolved_order = match draw_order {
                Some(place) => Some(resolve_revise_draw_order(
                    session,
                    record,
                    place,
                    &space,
                    sheet_name,
                    op,
                    "draw_order",
                )?),
                None => None,
            };
            let fonts = SheetFonts::find(&record.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let (hex, handles) = session.add_object(
                &parsed.object,
                &space,
                sheet_name,
                parsed.id.as_deref(),
                parsed.name.as_deref(),
                &start_of,
            )?;
            if let Some(order) = resolved_order {
                apply_revise_draw_order(session, &space, std::slice::from_ref(&hex), &order)?;
            }
            let id = match &parsed.id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &hex),
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("added {kind} `{id}` (handle `{hex}`)"),
                id: Some(id),
                handle: Some(hex),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped,
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::AddRevisionCloud {
            to,
            around,
            margin,
            keys,
            draw_order,
        } => {
            if let Some(to) = to {
                check_add_target_sheet(to, sheet_name, op)?;
            }
            let record = ensure_record(session, cached, source, sheet_name)?;
            let mut found = Vec::with_capacity(around.len());
            for (target, key) in around {
                for f in resolve_target_all(session, record, target, sheet_name, op, key)? {
                    found.push((f, key.as_str()));
                }
            }
            let fonts = SheetFonts::find(&record.record.sheet, settings);
            let drawn = crate::query::measuring_objects(&record.record.sheet, &fonts);
            let mut points: Vec<[f64; 2]> = Vec::new();
            for (f, key) in &found {
                let Some(place) = record
                    .record
                    .objects
                    .iter()
                    .position(|o| o.id.eq_ignore_ascii_case(&f.id))
                else {
                    continue;
                };
                let before = points.len();
                points.extend(
                    drawn
                        .items
                        .iter()
                        .filter(|item| item.object == place)
                        .flat_map(|item| item.points()),
                );
                points.extend(
                    drawn
                        .boundaries
                        .iter()
                        .filter(|(object, _)| *object == place)
                        .flat_map(|(_, b)| [b.min, b.max]),
                );
                if points.len() == before {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::BadValue,
                        format!(
                            "`{}` draws nothing, so it has no extents to frame with `around`",
                            f.id
                        ),
                        key,
                    ));
                }
            }
            if points.is_empty() {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::BadValue,
                    "the `around` objects have no drawn extents to frame",
                    "around",
                ));
            }
            let (min, max) = points.iter().fold(
                ([f64::MAX, f64::MAX], [f64::MIN, f64::MIN]),
                |(min, max), p| {
                    (
                        [min[0].min(p[0]), min[1].min(p[1])],
                        [max[0].max(p[0]), max[1].max(p[1])],
                    )
                },
            );
            let raw_w = (max[0] - min[0]).max(0.0);
            let raw_h = (max[1] - min[1]).max(0.0);
            let (arc_length, write_arc_length) =
                match keys.get("arc_length").filter(|v| !v.is_null()) {
                    Some(value) => match parse_length_json(value, units, "arc_length") {
                        Ok(len) if len.is_finite() && len > 0.0 => (len, false),
                        Ok(_) => {
                            return Err(op_error(
                                index,
                                op_name,
                                ErrorCode::BadValue,
                                "a revision cloud's arc_length is more than 0",
                                "arc_length",
                            ))
                        }
                        Err(e) => {
                            return Err(op_error(index, op_name, e.code, e.message, "arc_length"))
                        }
                    },
                    None => (
                        crate::model::RevisionCloud::default_arc_length(raw_w, raw_h),
                        true,
                    ),
                };
            let margin_len = match margin {
                Some(value) => match parse_length_json(value, units, "margin") {
                    Ok(m) if m.is_finite() && m >= 0.0 => m,
                    Ok(_) => {
                        return Err(op_error(
                            index,
                            op_name,
                            ErrorCode::BadValue,
                            "a revision cloud's margin cannot be negative",
                            "margin",
                        ))
                    }
                    Err(e) => return Err(op_error(index, op_name, e.code, e.message, "margin")),
                },
                None => crate::model::RevisionCloud::default_margin(arc_length),
            };
            let x0 = min[0] - margin_len;
            let y0 = min[1] - margin_len;
            let x1 = max[0] + margin_len;
            let y1 = max[1] + margin_len;
            if !(x1 > x0 && y1 > y0) {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::BadValue,
                    "the framed extents have zero width or height; give a margin more than 0",
                    "margin",
                ));
            }
            let mut resolved_keys = keys.clone();
            resolved_keys.insert(
                "rectangle".into(),
                json!({ "from": [x0, y0, 0.0], "to": [x1, y1, 0.0] }),
            );
            if write_arc_length {
                resolved_keys.insert("arc_length".into(), json!(arc_length));
            }
            let tables = session.tables_for_parsing();
            let parsed = parse_added_object_json(
                "revision_cloud",
                &resolved_keys,
                units,
                &tables.layers,
                &tables.linetypes,
                &tables.styles,
                &tables.blocks,
                &tables.layouts,
                &tables.dimension_styles,
                &tables.mline_styles,
            )
            .map_err(|e| {
                let field = e.field.as_deref().unwrap_or("");
                let trimmed = field.strip_prefix("revision_cloud.").unwrap_or(field);
                op_error(index, op_name, e.code, e.message, trimmed)
            })?;
            if let Some(explicit_id) = &parsed.id {
                let full = format!("{sheet_name}/{explicit_id}");
                if record
                    .record
                    .objects
                    .iter()
                    .any(|o| o.id.eq_ignore_ascii_case(&full))
                {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::DuplicateId,
                        format!("an object with ID `{full}` already exists on `{sheet_name}`"),
                        "id",
                    ));
                }
            }
            if let Some(explicit_name) = &parsed.name {
                if record.record.objects.iter().any(|o| {
                    o.name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case(explicit_name))
                }) {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::DuplicateName,
                        format!(
                            "an object named `{explicit_name}` already exists on `{sheet_name}`"
                        ),
                        "name",
                    ));
                }
            }
            let space = match parsed.space {
                None => TargetSpace::Model,
                Some(idx) => TargetSpace::Layout(tables.layouts[idx].clone()),
            };
            let resolved_order = match draw_order {
                Some(place) => Some(resolve_revise_draw_order(
                    session,
                    record,
                    place,
                    &space,
                    sheet_name,
                    op,
                    "draw_order",
                )?),
                None => None,
            };
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let (hex, handles) = session.add_object(
                &parsed.object,
                &space,
                sheet_name,
                parsed.id.as_deref(),
                parsed.name.as_deref(),
                &start_of,
            )?;
            if let Some(order) = resolved_order {
                apply_revise_draw_order(session, &space, std::slice::from_ref(&hex), &order)?;
            }
            let id = match &parsed.id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &hex),
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("added revision_cloud `{id}` (handle `{hex}`)"),
                id: Some(id),
                handle: Some(hex),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::DrawOrder { targets, order } => {
            let record = ensure_record(session, cached, source, sheet_name)?;
            let mut resolved = Vec::with_capacity(targets.len());
            let mut seen_handles: HashSet<u64> = HashSet::with_capacity(targets.len());
            for (target, field) in targets {
                let from_group = matches!(target, Target::Group { .. });
                for found in resolve_target_all(session, record, target, sheet_name, op, field)? {
                    let Some(handle_num) = canonical_handle_value(&found.handle) else {
                        return Err(op_error(
                            index,
                            op_name,
                            ErrorCode::BadValue,
                            format!("invalid handle `{}` on `{}`", found.handle, found.id),
                            field,
                        ));
                    };
                    if !seen_handles.insert(handle_num) {
                        if from_group {
                            continue;
                        }
                        return Err(op_error(
                            index,
                            op_name,
                            ErrorCode::BadValue,
                            format!(
                                "object `{}` (handle `{:X}`) is listed more than once in the \
                                 target list",
                                found.id, handle_num
                            ),
                            field,
                        ));
                    }
                    resolved.push((found, field.as_str()));
                }
            }
            let first_space = session.space_of_handle(&resolved[0].0.handle)?;
            for (found, field) in &resolved[1..] {
                let sp = session.space_of_handle(&found.handle)?;
                if sp != first_space {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::BadValue,
                        format!(
                            "`{}` and `{}` live in different spaces; draw_order reorders objects \
                             within one space",
                            resolved[0].0.id, found.id
                        ),
                        field,
                    ));
                }
            }
            let resolved_order = resolve_revise_draw_order(
                session,
                record,
                order,
                &first_space,
                sheet_name,
                op,
                "order",
            )?;
            if let ResolvedDrawOrder::Above(ref_h) | ResolvedDrawOrder::Under(ref_h) =
                &resolved_order
            {
                if let Some(ref_num) = canonical_handle_value(ref_h) {
                    if seen_handles.contains(&ref_num) {
                        return Err(op_error(
                            index,
                            op_name,
                            ErrorCode::BadValue,
                            "the reference object in `order` cannot also be one of the objects \
                             being reordered",
                            "order",
                        ));
                    }
                }
            }
            let moving: Vec<String> = resolved.iter().map(|(f, _)| f.handle.clone()).collect();
            apply_revise_draw_order(session, &first_space, &moving, &resolved_order)?;
            let first = &resolved[0].0;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "moved {} object{} {} in draw order",
                    resolved.len(),
                    plural(resolved.len()),
                    order.words()
                ),
                id: Some(first.id.clone()),
                handle: Some(first.handle.clone()),
                handles_touched: Vec::new(),
                handles_added: Vec::new(),
                handles_deleted: Vec::new(),
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::TextToFront { to, which } => {
            check_add_target_sheet(to, sheet_name, op)?;
            let record = ensure_record(session, cached, source, sheet_name)?;
            let dwg_origin = record
                .record
                .dwg
                .as_ref()
                .expect("DWG record has DwgOrigin");
            let mut by_space: Vec<(TargetSpace, Vec<String>)> = Vec::new();
            let mut total_matched = 0usize;
            for (i, obj) in record.record.sheet.objects.iter().enumerate() {
                if !which.matches_object(obj) {
                    continue;
                }
                let handle = &dwg_origin.handles[i];
                let Ok(space) = session.space_of_handle(handle) else {
                    continue;
                };
                total_matched += 1;
                if let Some((_, list)) = by_space.iter_mut().find(|(s, _)| *s == space) {
                    list.push(handle.clone());
                } else {
                    by_space.push((space, vec![handle.clone()]));
                }
            }
            for (space, moving) in &by_space {
                apply_revise_draw_order(session, space, moving, &ResolvedDrawOrder::Front)?;
            }
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "brought {total_matched} {} object{} to the front",
                    which.name(),
                    plural(total_matched)
                ),
                id: None,
                handle: None,
                handles_touched: Vec::new(),
                handles_added: Vec::new(),
                handles_deleted: Vec::new(),
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::SetAttribute {
            target,
            tag,
            occurrence,
            value,
        } => {
            if op.row_value.is_some() {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::Usage,
                    "`row_value` belongs to a drawing set template and cannot be used on an \
                     original DWG",
                    "row_value",
                ));
            }
            let record = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, record, target, sheet_name, op, "target")?;
            check_unlocked(session, &found.handle, &found.id, op, "target")?;
            let (msg, handles) = session
                .set_attribute(&found.handle, tag, *occurrence, value)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "tag"))?;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("{msg} on `{}` (handle `{}`)", found.id, found.handle),
                id: Some(found.id),
                handle: Some(found.handle),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::SetBlockState { target, state } => {
            let record = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, record, target, sheet_name, op, "target")?;
            check_unlocked(session, &found.handle, &found.id, op, "target")?;
            let mut dyn_state = std::collections::BTreeMap::new();
            for (k, v) in state {
                let parsed = match v {
                    Value::Bool(b) => crate::model::DynamicValue::Bool(*b),
                    Value::Number(n) => {
                        let Some(f) = n.as_f64().filter(|f| f.is_finite()) else {
                            return Err(op_error(
                                index,
                                op_name,
                                ErrorCode::BadValue,
                                format!("dynamic parameter `{k}` must be a finite number"),
                                &format!("state.{k}"),
                            ));
                        };
                        crate::model::DynamicValue::Number(f)
                    }
                    Value::String(s) => crate::model::DynamicValue::Text(s.clone()),
                    Value::Null => {
                        return Err(op_error(
                            index,
                            op_name,
                            ErrorCode::BadValue,
                            format!(
                                "resetting `{k}` with null is not supported on an original DWG; \
                                 pass the explicit state name"
                            ),
                            &format!("state.{k}"),
                        ))
                    }
                    _ => {
                        return Err(op_error(
                            index,
                            op_name,
                            ErrorCode::WrongType,
                            format!("dynamic parameter `{k}` takes a string, number, or boolean"),
                            &format!("state.{k}"),
                        ))
                    }
                };
                dyn_state.insert(k.clone(), parsed);
            }
            let (msg, handles) = session
                .set_block_state(&found.handle, &dyn_state)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "state"))?;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("{msg} on `{}` (handle `{}`)", found.id, found.handle),
                id: Some(found.id),
                handle: Some(found.handle),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::SetField {
            target,
            tag,
            occurrence,
            field,
            value,
        } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, loaded, target, sheet_name, op, "target")?;
            check_unlocked(session, &found.handle, &found.id, op, "target")?;

            // Build local-ID -> handle map and evaluate display slots for any
            // new field against the drawing as it currently stands.
            let record = &loaded.record;
            let dwg_origin = record.dwg.as_ref().expect("DWG record has DwgOrigin");
            let mut id_to_hex: std::collections::HashMap<String, String> =
                std::collections::HashMap::with_capacity(record.sheet.objects.len());
            let mut local_ids: Vec<String> = Vec::with_capacity(record.sheet.objects.len());
            for (i, h_hex) in dwg_origin.handles.iter().enumerate() {
                let full_id = &dwg_origin.ids[i];
                let local = full_id
                    .split_once(crate::description::ID_SEPARATOR)
                    .map_or(full_id.as_str(), |(_, tail)| tail)
                    .to_owned();
                id_to_hex.insert(full_id.to_lowercase(), h_hex.clone());
                id_to_hex.insert(local.to_lowercase(), h_hex.clone());
                id_to_hex.insert(format!("#{}", h_hex.to_lowercase()), h_hex.clone());
                local_ids.push(local);
            }
            let layout_name = found
                .model_index
                .and_then(|idx| record.sheet.layout_of(idx).map(|l| l.name.as_str()))
                .or_else(|| record.sheet.layout.as_ref().map(|l| l.name.as_str()));
            let display_slots = match field {
                Some(tf) => {
                    let ctx = crate::model::field::EvalContext::from_sheet(
                        &record.sheet,
                        Some(sheet_name),
                        layout_name,
                        &local_ids,
                        &dwg_origin.handles,
                    );
                    let (mut slots, _) = crate::model::field::evaluate_text_field(tf, &ctx);
                    if let Some(explicit) = value.as_deref() {
                        if tf.items.len() == 1
                            && crate::model::field::is_default_field_template(&tf.template)
                            && !explicit.is_empty()
                        {
                            slots[0] = explicit.to_owned();
                        }
                    }
                    slots
                }
                None => Vec::new(),
            };
            let (msg, handles) = session
                .set_field(
                    &found.handle,
                    tag.as_deref(),
                    *occurrence,
                    field.as_ref(),
                    &display_slots,
                    value.as_deref(),
                    &id_to_hex,
                )
                .map_err(|e| {
                    let f = e.field.as_deref().unwrap_or("field");
                    op_error(index, op_name, e.code, e.message, f)
                })?;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("{msg} on `{}` (handle `{}`)", found.id, found.handle),
                id: Some(found.id),
                handle: Some(found.handle),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::ReplaceText {
            find,
            replace,
            matching,
            case_sensitive,
            sheets,
            scopes,
            tag,
            block,
        } => {
            if let Some(names) = sheets {
                if !names.iter().any(|s| s.eq_ignore_ascii_case(sheet_name)) {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!(
                            "`sheets` does not include this drawing `{sheet_name}`; omit `sheets` \
                             or include `{sheet_name}`"
                        ),
                        "sheets",
                    ));
                }
            }
            let matcher = TextMatcher::compile(find, *matching, *case_sensitive)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "find"))?;
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let record = &loaded.record;
            enum PendingReplace {
                Contents {
                    handle: String,
                    id: String,
                    kind: &'static str,
                    before: String,
                    after: String,
                    count: usize,
                },
                Attribute {
                    handle: String,
                    id: String,
                    tag: String,
                    occurrence: Option<usize>,
                    before: String,
                    after: String,
                    count: usize,
                },
            }
            let mut pending = Vec::new();
            for (obj_idx, obj) in record.sheet.objects.iter().enumerate() {
                let Some(handle) = record.handle_of(obj_idx).map(str::to_owned) else {
                    continue;
                };
                let id = record.id(obj_idx);
                match obj {
                    Object::Text(t) if scopes.contains(&TextScope::Text) => {
                        if let Some((after, count)) = matcher.replace(&t.contents, replace) {
                            pending.push(PendingReplace::Contents {
                                handle,
                                id,
                                kind: "text",
                                before: t.contents.clone(),
                                after,
                                count,
                            });
                        }
                    }
                    Object::MText(t) if scopes.contains(&TextScope::MText) => {
                        if let Some((after, count)) = matcher.replace(&t.contents, replace) {
                            pending.push(PendingReplace::Contents {
                                handle,
                                id,
                                kind: "mtext",
                                before: t.contents.clone(),
                                after,
                                count,
                            });
                        }
                    }
                    Object::Insert(ins) if scopes.contains(&TextScope::Attribute) => {
                        if let Some(b) = block {
                            if !ins.block.eq_ignore_ascii_case(b) {
                                continue;
                            }
                        }
                        let mut totals: std::collections::BTreeMap<String, usize> =
                            std::collections::BTreeMap::new();
                        for a in &ins.attributes {
                            *totals.entry(a.tag.to_ascii_uppercase()).or_default() += 1;
                        }
                        let mut seen: std::collections::BTreeMap<String, usize> =
                            std::collections::BTreeMap::new();
                        for a in &ins.attributes {
                            let cap_tag = a.tag.to_ascii_uppercase();
                            let occ_0 = {
                                let e = seen.entry(cap_tag.clone()).or_default();
                                let cur = *e;
                                *e += 1;
                                cur
                            };
                            let repeated = totals.get(&cap_tag).copied().unwrap_or(0) > 1;
                            let occ_1 = repeated.then_some(occ_0 + 1);
                            if let Some(wanted_tag) = tag {
                                if !cap_tag.eq_ignore_ascii_case(wanted_tag) {
                                    continue;
                                }
                            }
                            if let Some((after, count)) = matcher.replace(&a.value, replace) {
                                pending.push(PendingReplace::Attribute {
                                    handle: handle.clone(),
                                    id: id.clone(),
                                    tag: cap_tag,
                                    occurrence: occ_1,
                                    before: a.value.clone(),
                                    after,
                                    count,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            let existing_names: HashSet<String> = record
                .objects
                .iter()
                .filter_map(|o| o.name.as_ref().map(|n| n.to_lowercase()))
                .collect();
            let fonts = SheetFonts::find(&record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let mut handles_touched = Vec::new();
            let mut replacements = Vec::new();
            for item in pending {
                match item {
                    PendingReplace::Contents {
                        handle,
                        id,
                        kind,
                        before,
                        after,
                        count,
                    } => {
                        check_unlocked(session, &handle, &id, op, "find")?;
                        let mut set = serde_json::Map::new();
                        set.insert("contents".to_owned(), Value::String(after.clone()));
                        let (_, handles) = session
                            .change_object(
                                &handle,
                                &set,
                                units,
                                sheet_name,
                                &existing_names,
                                &start_of,
                            )
                            .map_err(|e| op_error(index, op_name, e.code, e.message, "replace"))?;
                        handles_touched.extend(handles.touched);
                        replacements.push(TextReplaced {
                            sheet: sheet_name.to_owned(),
                            id,
                            kind: kind.to_owned(),
                            tag: None,
                            occurrence: None,
                            before,
                            after,
                            count,
                            row_value: None,
                        });
                    }
                    PendingReplace::Attribute {
                        handle,
                        id,
                        tag: cap_tag,
                        occurrence,
                        before,
                        after,
                        count,
                    } => {
                        check_unlocked(session, &handle, &id, op, "find")?;
                        let (_, handles) = session
                            .set_attribute(&handle, &cap_tag, occurrence, &after)
                            .map_err(|e| op_error(index, op_name, e.code, e.message, "replace"))?;
                        handles_touched.extend(handles.touched);
                        replacements.push(TextReplaced {
                            sheet: sheet_name.to_owned(),
                            id,
                            kind: "attribute".to_owned(),
                            tag: Some(cap_tag),
                            occurrence,
                            before,
                            after,
                            count,
                            row_value: None,
                        });
                    }
                }
            }
            let total_matches: usize = replacements.iter().map(|r| r.count).sum();
            let message = if replacements.is_empty() {
                format!("found no {} matches for `{find}`", matching.name())
            } else {
                format!(
                    "replaces {total_matches} match{} of `{find}` with `{replace}` in {} place{}",
                    if total_matches == 1 { "" } else { "es" },
                    replacements.len(),
                    if replacements.len() == 1 { "" } else { "s" },
                )
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id: None,
                handle: None,
                handles_touched,
                handles_added: Vec::new(),
                handles_deleted: Vec::new(),
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements,
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::SetAttributes {
            selection,
            sheets,
            tag,
            occurrence,
            value,
        } => {
            if op.row_value.is_some() {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::Usage,
                    "`row_value` belongs to a drawing set template and cannot be used on an \
                     original DWG",
                    "row_value",
                ));
            }
            if let Some(names) = sheets {
                if !names.iter().any(|s| s.eq_ignore_ascii_case(sheet_name)) {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!(
                            "`sheets` does not include this drawing `{sheet_name}`; omit `sheets` \
                             or include `{sheet_name}`"
                        ),
                        "sheets",
                    ));
                }
            }
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let record = &loaded.record;
            let selected = crate::query::select_on_sheet(selection, record, settings)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "where"))?;
            let cap_tag = tag.to_ascii_uppercase();
            let mut pending: Vec<(String, String, String, Option<usize>, String)> = Vec::new();
            for obj_idx in selected {
                let Object::Insert(ins) = &record.sheet.objects[obj_idx] else {
                    continue;
                };
                let Some(handle) = record.handle_of(obj_idx).map(str::to_owned) else {
                    continue;
                };
                let id = record.id(obj_idx);
                let editable: Vec<&str> = ins
                    .attributes
                    .iter()
                    .filter(|a| a.tag.eq_ignore_ascii_case(&cap_tag))
                    .map(|a| a.value.as_str())
                    .collect();
                let occ_idx = occurrence.unwrap_or(1).saturating_sub(1);
                let before = editable.get(occ_idx).copied().unwrap_or("").to_owned();
                let reported_occ = (editable.len() > 1).then_some(occurrence.unwrap_or(1));
                pending.push((handle, id, ins.block.clone(), reported_occ, before));
            }
            let mut handles_touched = Vec::new();
            let mut attributes_set = Vec::new();
            for (handle, id, block, reported_occ, before) in pending {
                check_unlocked(session, &handle, &id, op, "where")?;
                let (_, handles) = session
                    .set_attribute(&handle, &cap_tag, *occurrence, value)
                    .map_err(|e| op_error(index, op_name, e.code, e.message, "tag"))?;
                handles_touched.extend(handles.touched);
                attributes_set.push(AttributeUpdated {
                    sheet: sheet_name.to_owned(),
                    id,
                    block,
                    tag: cap_tag.clone(),
                    occurrence: reported_occ,
                    before,
                    after: value.clone(),
                    row_value: None,
                });
            }
            let message = if attributes_set.is_empty() {
                format!(
                    "no insert matches {}",
                    crate::query::selection_words(selection)
                )
            } else {
                format!(
                    "sets `{cap_tag}` to `{value}` on {} insert{}",
                    attributes_set.len(),
                    if attributes_set.len() == 1 { "" } else { "s" },
                )
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id: None,
                handle: None,
                handles_touched,
                handles_added: Vec::new(),
                handles_deleted: Vec::new(),
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set,
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Offset {
            target,
            by,
            id,
            name,
            layer,
            erase_source,
        } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, loaded, target, sheet_name, op, "target")?;
            if *erase_source {
                check_unlocked(session, &found.handle, &found.id, op, "target")?;
            }
            let obj_idx = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(found.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}` to offset it", found.id),
                        "target",
                    )
                })?;
            let base = loaded.record.sheet.objects[obj_idx].clone();
            let mut snapped = Vec::new();
            let resolved_by = match by {
                OpOffsetBy::Distance { distance, side } => {
                    let side = match side {
                        OpOffsetSide::Left => OffsetSide::Left,
                        OpOffsetSide::Right => OffsetSide::Right,
                        OpOffsetSide::Inside => OffsetSide::Inside,
                        OpOffsetSide::Outside => OffsetSide::Outside,
                        OpOffsetSide::Point(pt) => OffsetSide::Point(resolve_op_point(
                            pt,
                            loaded,
                            settings,
                            op,
                            &mut snapped,
                        )?),
                    };
                    OffsetBy::Distance {
                        distance: *distance,
                        side,
                    }
                }
                OpOffsetBy::Through(pt) => {
                    OffsetBy::Through(resolve_op_point(pt, loaded, settings, op, &mut snapped)?)
                }
            };
            let (new_object, eff_distance) = compute_offset(&base, &resolved_by, layer.as_deref())
                .map_err(|e| op_error(index, op_name, e.code, e.message, "distance"))?;
            let space = session.space_of_handle(&found.handle)?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let (hex, mut handles) = session.add_object(
                &new_object,
                &space,
                sheet_name,
                id.as_deref(),
                name.as_deref(),
                &start_of,
            )?;
            if *erase_source {
                let del = session.delete_object(&found.handle)?;
                handles.touched.extend(del.touched);
                handles.deleted.extend(del.deleted);
            }
            let new_id = match id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &hex),
            };
            let dist_str = crate::description::number_text(eff_distance);
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "offset `{}` by {dist_str} to {} `{new_id}` (handle `{hex}`)",
                    found.id,
                    new_object.type_name()
                ),
                id: Some(new_id),
                handle: Some(hex),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped,
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Trim {
            target,
            boundaries,
            keep,
            new_id,
        } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, loaded, target, sheet_name, op, "target")?;
            check_unlocked(session, &found.handle, &found.id, op, "target")?;
            let obj_idx = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(found.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}` to trim it", found.id),
                        "target",
                    )
                })?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let reader = Reader {
                sheet: &loaded.record.sheet,
                fonts: &fonts,
            };
            let mut b_curves = Vec::new();
            for (bt, bf) in boundaries {
                let bf_found = resolve_target(session, loaded, bt, sheet_name, op, bf)?;
                let b_idx = (0..loaded.record.sheet.objects.len())
                    .find(|&i| loaded.record.handle_of(i) == Some(bf_found.handle.as_str()))
                    .ok_or_else(|| {
                        op_error(
                            index,
                            op_name,
                            ErrorCode::UnknownName,
                            format!("could not read boundary `{}`", bf_found.id),
                            bf,
                        )
                    })?;
                if !loaded.record.sheet.same_space(obj_idx, b_idx) {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::BadValue,
                        format!(
                            "boundary `{}` is in a different space from `target`",
                            bf_found.id
                        ),
                        bf,
                    ));
                }
                for part in reader.parts(&loaded.record.sheet.objects[b_idx]) {
                    b_curves.push(part.curve);
                }
            }
            let base = loaded.record.sheet.objects[obj_idx].clone();
            let obj_name = loaded.record.objects[obj_idx].name.clone();
            let trimmed = compute_trim(&base, &b_curves, *keep)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "boundary"))?;
            let space = session.space_of_handle(&found.handle)?;
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let local_id = found
                .id
                .split_once('/')
                .map(|(_, l)| l)
                .filter(|l| !l.starts_with(HANDLE_ID));
            let del = session.delete_object(&found.handle)?;
            let (new_hex, added) = session.add_object(
                &trimmed.kept,
                &space,
                sheet_name,
                local_id,
                obj_name.as_deref(),
                &start_of,
            )?;
            let mut handles_touched = del.touched;
            handles_touched.extend(added.touched);
            let mut handles_added = added.added;
            let handles_deleted = del.deleted;
            let mut split_full_id = None;
            if let Some(split_obj) = &trimmed.split {
                let (shex, sadd) = session.add_object(
                    split_obj,
                    &space,
                    sheet_name,
                    new_id.as_deref(),
                    None,
                    &start_of,
                )?;
                handles_touched.extend(sadd.touched);
                handles_added.extend(sadd.added);
                split_full_id = Some(match new_id {
                    Some(explicit) => format!("{sheet_name}/{explicit}"),
                    None => handle_id(sheet_name, &shex),
                });
            }
            let kept_id = match local_id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &new_hex),
            };
            let message = match &split_full_id {
                Some(sid) => format!(
                    "trimmed `{}` (keep `{}`) and split off `{sid}`",
                    found.id,
                    keep.name()
                ),
                None => format!("trimmed `{}` (keep `{}`)", found.id, keep.name()),
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id: Some(kept_id),
                handle: Some(new_hex),
                handles_touched,
                handles_added,
                handles_deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: split_full_id,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Extend {
            target,
            boundaries,
            end,
        } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, loaded, target, sheet_name, op, "target")?;
            check_unlocked(session, &found.handle, &found.id, op, "target")?;
            let obj_idx = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(found.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}` to extend it", found.id),
                        "target",
                    )
                })?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let reader = Reader {
                sheet: &loaded.record.sheet,
                fonts: &fonts,
            };
            let mut b_curves = Vec::new();
            for (bt, bf) in boundaries {
                let bf_found = resolve_target(session, loaded, bt, sheet_name, op, bf)?;
                let b_idx = (0..loaded.record.sheet.objects.len())
                    .find(|&i| loaded.record.handle_of(i) == Some(bf_found.handle.as_str()))
                    .ok_or_else(|| {
                        op_error(
                            index,
                            op_name,
                            ErrorCode::UnknownName,
                            format!("could not read boundary `{}`", bf_found.id),
                            bf,
                        )
                    })?;
                if !loaded.record.sheet.same_space(obj_idx, b_idx) {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::BadValue,
                        format!(
                            "boundary `{}` is in a different space from `target`",
                            bf_found.id
                        ),
                        bf,
                    ));
                }
                for part in reader.parts(&loaded.record.sheet.objects[b_idx]) {
                    b_curves.push(part.curve);
                }
            }
            let base = loaded.record.sheet.objects[obj_idx].clone();
            let obj_name = loaded.record.objects[obj_idx].name.clone();
            let extended = compute_extend(&base, &b_curves, *end)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "boundary"))?;
            let space = session.space_of_handle(&found.handle)?;
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let local_id = found
                .id
                .split_once('/')
                .map(|(_, l)| l)
                .filter(|l| !l.starts_with(HANDLE_ID));
            let del = session.delete_object(&found.handle)?;
            let (new_hex, added) = session.add_object(
                &extended,
                &space,
                sheet_name,
                local_id,
                obj_name.as_deref(),
                &start_of,
            )?;
            let mut handles_touched = del.touched;
            handles_touched.extend(added.touched);
            let kept_id = match local_id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &new_hex),
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("extended `{}` ({} end)", found.id, end.name()),
                id: Some(kept_id),
                handle: Some(new_hex),
                handles_touched,
                handles_added: added.added,
                handles_deleted: del.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Fillet { mode, radius } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            apply_original_corner(
                session,
                loaded,
                sheet_name,
                op,
                mode,
                OriginalCornerOp::Fillet(*radius),
                settings,
            )
        }
        Body::Chamfer { mode, spec } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            apply_original_corner(
                session,
                loaded,
                sheet_name,
                op,
                mode,
                OriginalCornerOp::Chamfer(*spec),
                settings,
            )
        }
        Body::Break { target, by, new_id } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, loaded, target, sheet_name, op, "target")?;
            check_unlocked(session, &found.handle, &found.id, op, "target")?;
            let obj_idx = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(found.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}` to break it", found.id),
                        "target",
                    )
                })?;
            let mut snapped = Vec::new();
            let spec = match by {
                OpBreakBy::AtPoint { at, gap } => BreakSpec::AtPoint {
                    at: resolve_op_point(at, loaded, settings, op, &mut snapped)?,
                    gap: *gap,
                },
                OpBreakBy::Between { from, to } => BreakSpec::Between {
                    from: resolve_op_point(from, loaded, settings, op, &mut snapped)?,
                    to: resolve_op_point(to, loaded, settings, op, &mut snapped)?,
                },
            };
            let base = loaded.record.sheet.objects[obj_idx].clone();
            let obj_name = loaded.record.objects[obj_idx].name.clone();
            let broken = compute_break(&base, spec)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "at"))?;
            let space = session.space_of_handle(&found.handle)?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let local_id = found
                .id
                .split_once('/')
                .map(|(_, l)| l)
                .filter(|l| !l.starts_with(HANDLE_ID));
            let del = session.delete_object(&found.handle)?;
            let (new_hex, added) = session.add_object(
                &broken.kept,
                &space,
                sheet_name,
                local_id,
                obj_name.as_deref(),
                &start_of,
            )?;
            let mut handles_touched = del.touched;
            handles_touched.extend(added.touched);
            let mut handles_added = added.added;
            let handles_deleted = del.deleted;
            let mut split_full_id = None;
            if let Some(split_obj) = &broken.split {
                let (shex, sadd) = session.add_object(
                    split_obj,
                    &space,
                    sheet_name,
                    new_id.as_deref(),
                    None,
                    &start_of,
                )?;
                handles_touched.extend(sadd.touched);
                handles_added.extend(sadd.added);
                split_full_id = Some(match new_id {
                    Some(explicit) => format!("{sheet_name}/{explicit}"),
                    None => handle_id(sheet_name, &shex),
                });
            }
            let kept_id = match local_id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &new_hex),
            };
            let message = match &split_full_id {
                Some(sid) => format!(
                    "broke `{}`; `{kept_id}` carries on and `{sid}` is new",
                    found.id
                ),
                None => format!(
                    "broke `{}` (`{kept_id}` carries on as {})",
                    found.id,
                    broken.kept.type_name()
                ),
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id: Some(kept_id),
                handle: Some(new_hex),
                handles_touched,
                handles_added,
                handles_deleted,
                copies: Vec::new(),
                snapped,
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: split_full_id,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Join { targets, tolerance } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let mut resolved = Vec::with_capacity(targets.len());
            let mut indices = Vec::with_capacity(targets.len());
            for (i, (t, key)) in targets.iter().enumerate() {
                let f = resolve_target(session, loaded, t, sheet_name, op, key)?;
                check_unlocked(session, &f.handle, &f.id, op, key)?;
                if resolved
                    .iter()
                    .any(|prev: &ResolvedTarget| prev.handle == f.handle)
                {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::BadValue,
                        format!("`{}` is named twice in `targets`", f.id),
                        key,
                    ));
                }
                let obj_idx = (0..loaded.record.sheet.objects.len())
                    .find(|&j| loaded.record.handle_of(j) == Some(f.handle.as_str()))
                    .ok_or_else(|| {
                        op_error(
                            index,
                            op_name,
                            ErrorCode::UnknownName,
                            format!("could not read `{}` to join it", f.id),
                            key,
                        )
                    })?;
                if i > 0 && !loaded.record.sheet.same_space(indices[0], obj_idx) {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::BadValue,
                        format!(
                            "`{}` is in a different space from `{}`",
                            f.id, resolved[0].id
                        ),
                        key,
                    ));
                }
                resolved.push(f);
                indices.push(obj_idx);
            }
            let objs: Vec<&Object> = indices
                .iter()
                .map(|&idx| &loaded.record.sheet.objects[idx])
                .collect();
            let joined = compute_join(&objs, *tolerance)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "targets"))?;
            let space = session.space_of_handle(&resolved[0].handle)?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let local0 = resolved[0]
                .id
                .split_once('/')
                .map(|(_, l)| l)
                .filter(|l| !l.starts_with(HANDLE_ID));
            let name0 = loaded.record.objects[indices[0]].name.clone();
            let mut handles_touched = Vec::new();
            let mut handles_added = Vec::new();
            let mut handles_deleted = Vec::new();
            for f in &resolved {
                let del = session.delete_object(&f.handle)?;
                handles_touched.extend(del.touched);
                handles_deleted.extend(del.deleted);
            }
            let (new_hex, added) = session.add_object(
                &joined,
                &space,
                sheet_name,
                local0,
                name0.as_deref(),
                &start_of,
            )?;
            handles_touched.extend(added.touched);
            handles_added.extend(added.added);
            let kept_id = match local0 {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &new_hex),
            };
            let deleted_ids: Vec<String> = resolved[1..].iter().map(|f| f.id.clone()).collect();
            let removed_text: Vec<String> =
                deleted_ids.iter().map(|id| format!("`{id}`")).collect();
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "joined {} objects into {} `{kept_id}` (carries on; removed {})",
                    resolved.len(),
                    joined.type_name(),
                    removed_text.join(", ")
                ),
                id: Some(kept_id),
                handle: Some(new_hex),
                handles_touched,
                handles_added,
                handles_deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: deleted_ids,
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Extrude {
            target,
            height,
            direction,
            taper_angle,
            erase_source,
            layer,
            id,
            name,
        } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, loaded, target, sheet_name, op, "target")?;
            if *erase_source {
                check_unlocked(session, &found.handle, &found.id, op, "target")?;
            }
            let obj_idx = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(found.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}` to extrude it", found.id),
                        "target",
                    )
                })?;
            let base = loaded.record.sheet.objects[obj_idx].clone();
            let profile = extract_profile(&base, op_name)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "target"))?;
            let solid = build_extrude_solid(
                profile.points,
                profile.bulges,
                profile.exact_circle,
                *height,
                *direction,
                *taper_angle,
                layer.clone().unwrap_or(profile.layer),
                profile.properties,
            )
            .map_err(|e| op_error(index, op_name, e.code, e.message, "height"))?;
            let new_object = Object::Solid3D(solid);
            let space = session.space_of_handle(&found.handle)?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let (hex, mut handles) = session.add_object(
                &new_object,
                &space,
                sheet_name,
                id.as_deref(),
                name.as_deref(),
                &start_of,
            )?;
            let mut deleted = Vec::new();
            if *erase_source {
                let del = session.delete_object(&found.handle)?;
                handles.touched.extend(del.touched);
                handles.deleted.extend(del.deleted);
                deleted.push(found.id.clone());
            }
            let new_id = match id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &hex),
            };
            let h_str = crate::description::number_text(*height);
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "extruded `{}` by {h_str} to solid3d `{new_id}` (handle `{hex}`)",
                    found.id
                ),
                id: Some(new_id),
                handle: Some(hex),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted,
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Revolve {
            target,
            axis_origin,
            axis_direction,
            angle,
            erase_source,
            layer,
            id,
            name,
        } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, loaded, target, sheet_name, op, "target")?;
            if *erase_source {
                check_unlocked(session, &found.handle, &found.id, op, "target")?;
            }
            let obj_idx = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(found.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}` to revolve it", found.id),
                        "target",
                    )
                })?;
            let base = loaded.record.sheet.objects[obj_idx].clone();
            let profile = extract_profile(&base, op_name)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "target"))?;
            let mut snapped = Vec::new();
            let origin_xyz = resolve_op_point(axis_origin, loaded, settings, op, &mut snapped)?;
            let solid = build_revolve_solid(
                profile.points,
                profile.bulges,
                Point3::new(origin_xyz[0], origin_xyz[1], origin_xyz[2]),
                *axis_direction,
                *angle,
                layer.clone().unwrap_or(profile.layer),
                profile.properties,
            )
            .map_err(|e| op_error(index, op_name, e.code, e.message, "angle"))?;
            let new_object = Object::Solid3D(solid);
            let space = session.space_of_handle(&found.handle)?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let (hex, mut handles) = session.add_object(
                &new_object,
                &space,
                sheet_name,
                id.as_deref(),
                name.as_deref(),
                &start_of,
            )?;
            let mut deleted = Vec::new();
            if *erase_source {
                let del = session.delete_object(&found.handle)?;
                handles.touched.extend(del.touched);
                handles.deleted.extend(del.deleted);
                deleted.push(found.id.clone());
            }
            let new_id = match id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &hex),
            };
            let ang_str = crate::description::number_text(*angle);
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "revolved `{}` by {ang_str}° to solid3d `{new_id}` (handle `{hex}`)",
                    found.id
                ),
                id: Some(new_id),
                handle: Some(hex),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped,
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted,
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::SolidBoolean {
            op: bool_op,
            targets,
            layer,
            id,
            name,
        } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let mut resolved = Vec::with_capacity(targets.len());
            let mut indices = Vec::with_capacity(targets.len());
            for (i, (t, key)) in targets.iter().enumerate() {
                let f = resolve_target(session, loaded, t, sheet_name, op, key)?;
                check_unlocked(session, &f.handle, &f.id, op, key)?;
                if resolved
                    .iter()
                    .any(|prev: &ResolvedTarget| prev.handle == f.handle)
                {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::BadValue,
                        format!("`{}` is named twice", f.id),
                        key,
                    ));
                }
                let obj_idx = (0..loaded.record.sheet.objects.len())
                    .find(|&j| loaded.record.handle_of(j) == Some(f.handle.as_str()))
                    .ok_or_else(|| {
                        op_error(
                            index,
                            op_name,
                            ErrorCode::UnknownName,
                            format!("could not read `{}` for `{}`", f.id, bool_op.name()),
                            key,
                        )
                    })?;
                if i > 0 && !loaded.record.sheet.same_space(indices[0], obj_idx) {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::BadValue,
                        format!(
                            "`{}` is in a different space from `{}`",
                            f.id, resolved[0].id
                        ),
                        key,
                    ));
                }
                resolved.push(f);
                indices.push(obj_idx);
            }
            let mut solids: Vec<&Solid3D> = Vec::with_capacity(indices.len());
            for (&obj_idx, (f, (_, key))) in indices.iter().zip(resolved.iter().zip(targets)) {
                match &loaded.record.sheet.objects[obj_idx] {
                    Object::Solid3D(s) => solids.push(s),
                    other => {
                        return Err(op_error(
                            index,
                            op_name,
                            ErrorCode::BadValue,
                            format!(
                                "`{}` expects 3D solids (`solid3d`), but `{}` is a {}",
                                bool_op.name(),
                                f.id,
                                other.type_name()
                            ),
                            key,
                        ));
                    }
                }
            }
            let combined = Object::Solid3D(
                build_boolean_solid(*bool_op, &solids, layer.clone())
                    .map_err(|e| op_error(index, op_name, e.code, e.message, ""))?,
            );
            let space = session.space_of_handle(&resolved[0].handle)?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let local0 = id.as_deref().or_else(|| {
                resolved[0]
                    .id
                    .split_once('/')
                    .map(|(_, l)| l)
                    .filter(|l| !l.starts_with(HANDLE_ID))
            });
            let eff_name = name
                .as_deref()
                .or(loaded.record.objects[indices[0]].name.as_deref());
            let mut handles_touched = Vec::new();
            let mut handles_added = Vec::new();
            let mut handles_deleted = Vec::new();
            for f in &resolved {
                let del = session.delete_object(&f.handle)?;
                handles_touched.extend(del.touched);
                handles_deleted.extend(del.deleted);
            }
            let (new_hex, added) =
                session.add_object(&combined, &space, sheet_name, local0, eff_name, &start_of)?;
            handles_touched.extend(added.touched);
            handles_added.extend(added.added);
            let kept_id = match local0 {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &new_hex),
            };
            let deleted_ids: Vec<String> = resolved[1..].iter().map(|f| f.id.clone()).collect();
            let removed_text: Vec<String> =
                deleted_ids.iter().map(|id| format!("`{id}`")).collect();
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "applied `{}` to {} solids into solid3d `{kept_id}` (removed {})",
                    bool_op.name(),
                    resolved.len(),
                    removed_text.join(", ")
                ),
                id: Some(kept_id),
                handle: Some(new_hex),
                handles_touched,
                handles_added,
                handles_deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: deleted_ids,
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Stretch {
            to,
            space,
            targets,
            corners,
            motion,
        } => {
            if let AddTo::Template(_) = to {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::Usage,
                    "`template` belongs to a drawing set and cannot be used on an original DWG",
                    "template",
                ));
            }
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let mut snapped = Vec::new();
            let c0 = resolve_op_point(&corners[0], loaded, settings, op, &mut snapped)?;
            let c1 = resolve_op_point(&corners[1], loaded, settings, op, &mut snapped)?;
            let stretch_box = StretchBox::from_corners(c0, c1)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "box"))?;
            let delta = resolve_motion(motion, loaded, settings, op, &mut snapped)?;

            let wanted_space = space
                .as_deref()
                .filter(|s| !s.eq_ignore_ascii_case("model"));
            if let Some(layout) = wanted_space {
                if !loaded
                    .record
                    .sheet
                    .layouts()
                    .any(|l| l.name.eq_ignore_ascii_case(layout))
                {
                    return Err(op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("drawing `{sheet_name}` has no layout `{layout}`"),
                        "space",
                    ));
                }
            }
            let explicit_handles: Option<HashSet<String>> = if let Some(ts) = targets {
                let mut set = HashSet::new();
                for (t, key) in ts {
                    let f = resolve_target(session, loaded, t, sheet_name, op, key)?;
                    check_unlocked_with_children(session, &f.handle, &f.id, op, key)?;
                    set.insert(f.handle);
                }
                Some(set)
            } else {
                None
            };

            struct PendingOrig {
                handle: String,
                id: String,
                name: Option<String>,
                action: StretchAction,
            }
            let mut pending = Vec::new();
            let mut locked_skipped = 0usize;
            for (obj_idx, obj) in loaded.record.sheet.objects.iter().enumerate() {
                let Some(handle) = loaded.record.handle_of(obj_idx).map(str::to_owned) else {
                    continue;
                };
                if let Some(ref filter) = explicit_handles {
                    if !filter.contains(&handle) {
                        continue;
                    }
                }
                let obj_space = crate::query::space_of(&loaded.record.sheet, obj_idx);
                let in_space = match (wanted_space, obj_space.as_deref()) {
                    (None, None) => true,
                    (Some(w), Some(o)) => w.eq_ignore_ascii_case(o),
                    _ => false,
                };
                if !in_space {
                    continue;
                }
                let action = compute_stretch(obj, stretch_box, delta)
                    .map_err(|e| op_error(index, op_name, e.code, e.message, "by"))?;
                if matches!(action, StretchAction::Untouched) {
                    continue;
                }
                let id = loaded.record.id(obj_idx);
                if explicit_handles.is_none()
                    && check_unlocked_with_children(session, &handle, &id, op, "box").is_err()
                {
                    locked_skipped += 1;
                    continue;
                }
                pending.push(PendingOrig {
                    handle,
                    id,
                    name: loaded.record.objects[obj_idx].name.clone(),
                    action,
                });
            }

            let fonts = SheetFonts::find(&loaded.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let mut handles_touched = Vec::new();
            let mut handles_added = Vec::new();
            let mut handles_deleted = Vec::new();
            let mut transformed = Vec::new();
            let mut stretched_count = 0usize;
            let mut moved_count = 0usize;

            for item in pending {
                match item.action {
                    StretchAction::MoveWhole => {
                        let h = session
                            .move_object(&item.handle, delta)
                            .map_err(|e| op_error(index, op_name, e.code, e.message, "by"))?;
                        handles_touched.extend(h.touched);
                        handles_added.extend(h.added);
                        handles_deleted.extend(h.deleted);
                        transformed.push(item.id);
                        moved_count += 1;
                    }
                    StretchAction::Replace(new_obj) => {
                        let sp = session.space_of_handle(&item.handle)?;
                        let local = item
                            .id
                            .split_once('/')
                            .map(|(_, l)| l)
                            .filter(|l| !l.starts_with(HANDLE_ID));
                        let del = session.delete_object(&item.handle)?;
                        let (new_hex, add) = session.add_object(
                            &new_obj,
                            &sp,
                            sheet_name,
                            local,
                            item.name.as_deref(),
                            &start_of,
                        )?;
                        handles_touched.extend(del.touched);
                        handles_touched.extend(add.touched);
                        handles_added.extend(add.added);
                        handles_deleted.extend(del.deleted);
                        let new_id = match local {
                            Some(explicit) => format!("{sheet_name}/{explicit}"),
                            None => handle_id(sheet_name, &new_hex),
                        };
                        transformed.push(new_id);
                        stretched_count += 1;
                    }
                    StretchAction::Untouched => unreachable!(),
                }
            }

            let total = transformed.len();
            let locked_note = if locked_skipped > 0 {
                format!(" ({locked_skipped} on locked layers skipped)")
            } else {
                String::new()
            };
            let message = if total == 0 {
                format!("no unlocked object lies inside the stretch box{locked_note}")
            } else {
                format!(
                    "stretched {stretched_count} and moved {moved_count} object{} ({total} \
                     touched){locked_note}",
                    plural(total)
                )
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id: None,
                handle: None,
                handles_touched,
                handles_added,
                handles_deleted,
                copies: Vec::new(),
                snapped,
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed,
                created: Vec::new(),
            })
        }
        Body::Explode { targets, id_prefix } => {
            let loaded = ensure_record(session, cached, source, sheet_name)?;
            let fonts = SheetFonts::find(&loaded.record.sheet, settings);

            struct PendingExplodeOrig {
                handle: String,
                id: String,
                space: TargetSpace,
                pieces: Vec<Object>,
            }

            let mut seen: HashSet<String> = HashSet::new();
            let mut plan: Vec<PendingExplodeOrig> = Vec::with_capacity(targets.len());
            for (target, key) in targets {
                let from_group = matches!(target, Target::Group { .. });
                for found in resolve_target_all(session, loaded, target, sheet_name, op, key)? {
                    if !seen.insert(found.handle.clone()) {
                        if from_group {
                            continue;
                        }
                        return Err(op_error(
                            index,
                            op_name,
                            ErrorCode::BadValue,
                            format!("`{}` is listed more than once to explode", found.id),
                            key,
                        ));
                    }
                    check_unlocked_with_children(session, &found.handle, &found.id, op, key)?;
                    let obj_idx = (0..loaded.record.sheet.objects.len())
                        .find(|&i| loaded.record.handle_of(i) == Some(found.handle.as_str()))
                        .ok_or_else(|| {
                            op_error(
                                index,
                                op_name,
                                ErrorCode::UnknownName,
                                format!("could not read `{}` to explode it", found.id),
                                key,
                            )
                        })?;
                    let obj = &loaded.record.sheet.objects[obj_idx];
                    let pieces = compute_explode(obj, &loaded.record.sheet, &fonts)
                        .map_err(|e| op_error(index, op_name, e.code, e.message, key))?;
                    let space = session.space_of_handle(&found.handle)?;
                    plan.push(PendingExplodeOrig {
                        handle: found.handle,
                        id: found.id,
                        space,
                        pieces,
                    });
                }
            }

            let start_of = |t: &crate::text::Text| fonts.start(t);
            let mut handles_touched = Vec::new();
            let mut handles_added = Vec::new();
            let mut handles_deleted = Vec::new();
            let mut created = Vec::new();
            let mut deleted = Vec::new();
            let mut seq = 1usize;

            for item in plan {
                let del = session.delete_object(&item.handle)?;
                handles_touched.extend(del.touched);
                handles_deleted.extend(del.deleted);
                deleted.push(item.id);
                for piece in &item.pieces {
                    let explicit_local = id_prefix.as_ref().map(|p| {
                        let s = format!("{p}-{seq}");
                        seq += 1;
                        s
                    });
                    let (new_hex, added) = session.add_object(
                        piece,
                        &item.space,
                        sheet_name,
                        explicit_local.as_deref(),
                        None,
                        &start_of,
                    )?;
                    handles_touched.extend(added.touched);
                    handles_added.extend(added.added);
                    let new_id = match explicit_local {
                        Some(explicit) => format!("{sheet_name}/{explicit}"),
                        None => handle_id(sheet_name, &new_hex),
                    };
                    created.push(new_id);
                }
            }

            let first_deleted = (deleted.len() == 1).then(|| deleted[0].clone());
            let message = format!(
                "explodes {} object{} into {} object{}",
                deleted.len(),
                if deleted.len() == 1 { "" } else { "s" },
                created.len(),
                if created.len() == 1 { "" } else { "s" }
            );
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id: first_deleted,
                handle: None,
                handles_touched,
                handles_added,
                handles_deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted,
                transformed: Vec::new(),
                created,
            })
        }
        Body::Change {
            target,
            set,
            viewport,
            clip,
            ..
        } => {
            if *viewport || clip.is_some() {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::Usage,
                    format!(
                        "operation `{op_name}` is not supported when revising an original DWG yet"
                    ),
                    "",
                ));
            }
            if op.row_value.is_some() {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::Usage,
                    "`row_value` belongs to a drawing set template and cannot be used on an \
                     original DWG",
                    "row_value",
                ));
            }
            let record = ensure_record(session, cached, source, sheet_name)?;
            let resolved = resolve_target_all(session, record, target, sheet_name, op, "target")?;
            let moves_children = set.contains_key("position")
                || set.contains_key("rotation")
                || set.contains_key("scale");
            for found in &resolved {
                if moves_children {
                    check_unlocked_with_children(session, &found.handle, &found.id, op, "target")?;
                } else {
                    check_unlocked(session, &found.handle, &found.id, op, "target")?;
                }
            }
            let existing_names: HashSet<String> = record
                .record
                .objects
                .iter()
                .filter_map(|o| o.name.as_ref().map(|n| n.to_lowercase()))
                .collect();
            let fonts = SheetFonts::find(&record.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
            let mut all_handles = MutationHandles::default();
            let mut last_msg = String::new();
            for found in &resolved {
                let (msg, handles) = session
                    .change_object(
                        &found.handle,
                        set,
                        units,
                        sheet_name,
                        &existing_names,
                        &start_of,
                    )
                    .map_err(|e| {
                        let field = e.field.as_deref().unwrap_or("set");
                        op_error(index, op_name, e.code, e.message, field)
                    })?;
                last_msg = msg;
                extend_handles(&mut all_handles, handles);
            }
            let (message, id, handle, transformed) =
                if resolved.len() == 1 && !matches!(target, Target::Group { .. }) {
                    let only = &resolved[0];
                    (
                        format!("{last_msg} on `{}` (handle `{}`)", only.id, only.handle),
                        Some(only.id.clone()),
                        Some(only.handle.clone()),
                        Vec::new(),
                    )
                } else {
                    (
                        format!(
                            "{last_msg} on {} ({} object{})",
                            target.words(),
                            resolved.len(),
                            plural(resolved.len())
                        ),
                        resolved.first().map(|f| f.id.clone()),
                        resolved.first().map(|f| f.handle.clone()),
                        resolved.iter().map(|f| f.id.clone()).collect(),
                    )
                };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id,
                handle,
                handles_touched: all_handles.touched,
                handles_added: all_handles.added,
                handles_deleted: all_handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed,
                created: Vec::new(),
            })
        }
        Body::Move { target, motion } => {
            let record = ensure_record(session, cached, source, sheet_name)?;
            let resolved = resolve_target_all(session, record, target, sheet_name, op, "target")?;
            for found in &resolved {
                check_unlocked_with_children(session, &found.handle, &found.id, op, "target")?;
            }
            let mut snapped = Vec::new();
            let delta = resolve_motion(motion, record, settings, op, &mut snapped)?;
            let mut all_handles = MutationHandles::default();
            for found in &resolved {
                let handles = session
                    .move_object(&found.handle, delta)
                    .map_err(|e| op_error(index, op_name, e.code, e.message, "target"))?;
                extend_handles(&mut all_handles, handles);
            }
            let (message, id, handle, transformed) =
                if resolved.len() == 1 && !matches!(target, Target::Group { .. }) {
                    let only = &resolved[0];
                    (
                        format!(
                            "moved `{}` (handle `{}`) by [{}, {}, {}]",
                            only.id, only.handle, delta[0], delta[1], delta[2]
                        ),
                        Some(only.id.clone()),
                        Some(only.handle.clone()),
                        Vec::new(),
                    )
                } else {
                    (
                        format!(
                            "moved {} ({} object{}) by [{}, {}, {}]",
                            target.words(),
                            resolved.len(),
                            plural(resolved.len()),
                            delta[0],
                            delta[1],
                            delta[2]
                        ),
                        resolved.first().map(|f| f.id.clone()),
                        resolved.first().map(|f| f.handle.clone()),
                        resolved.iter().map(|f| f.id.clone()).collect(),
                    )
                };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id,
                handle,
                handles_touched: all_handles.touched,
                handles_added: all_handles.added,
                handles_deleted: all_handles.deleted,
                copies: Vec::new(),
                snapped,
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed,
                created: Vec::new(),
            })
        }
        Body::Delete { target } => {
            let record = ensure_record(session, cached, source, sheet_name)?;
            let resolved = resolve_target_all(session, record, target, sheet_name, op, "target")?;
            for found in &resolved {
                check_unlocked_with_children(session, &found.handle, &found.id, op, "target")?;
            }
            let mut all_handles = MutationHandles::default();
            for found in &resolved {
                let handles = session
                    .delete_object(&found.handle)
                    .map_err(|e| op_error(index, op_name, e.code, e.message, "target"))?;
                extend_handles(&mut all_handles, handles);
            }
            let (message, id, handle, deleted) =
                if resolved.len() == 1 && !matches!(target, Target::Group { .. }) {
                    let only = &resolved[0];
                    (
                        format!("deleted `{}` (handle `{}`)", only.id, only.handle),
                        Some(only.id.clone()),
                        Some(only.handle.clone()),
                        Vec::new(),
                    )
                } else {
                    (
                        format!(
                            "deleted {} ({} object{})",
                            target.words(),
                            resolved.len(),
                            plural(resolved.len())
                        ),
                        resolved.first().map(|f| f.id.clone()),
                        resolved.first().map(|f| f.handle.clone()),
                        resolved.iter().map(|f| f.id.clone()).collect(),
                    )
                };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id,
                handle,
                handles_touched: all_handles.touched,
                handles_added: all_handles.added,
                handles_deleted: all_handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted,
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Transform { targets, by } => {
            let record = ensure_record(session, cached, source, sheet_name)?;
            let resolved = resolve_unique_targets(session, record, targets, sheet_name, op)?;
            let mut snapped = Vec::new();
            let mut all_handles = MutationHandles::default();
            let summary = match by {
                TransformBy::Rotate { base_point, angle } => {
                    let about = resolve_op_point(base_point, record, settings, op, &mut snapped)?;
                    for found in &resolved {
                        let h = session
                            .rotate_object(&found.handle, about, *angle)
                            .map_err(|e| op_error(index, op_name, e.code, e.message, ""))?;
                        extend_handles(&mut all_handles, h);
                    }
                    format!(
                        "rotated {} object{} by {angle} deg",
                        resolved.len(),
                        plural(resolved.len())
                    )
                }
                TransformBy::Scale { base_point, factor } => {
                    let about = resolve_op_point(base_point, record, settings, op, &mut snapped)?;
                    for found in &resolved {
                        let h = session
                            .scale_object(&found.handle, about, *factor)
                            .map_err(|e| op_error(index, op_name, e.code, e.message, ""))?;
                        extend_handles(&mut all_handles, h);
                    }
                    format!(
                        "scaled {} object{} by {factor}",
                        resolved.len(),
                        plural(resolved.len())
                    )
                }
                TransformBy::Mirror { first, second } => {
                    let a = resolve_op_point(first, record, settings, op, &mut snapped)?;
                    let b = resolve_op_point(second, record, settings, op, &mut snapped)?;
                    for found in &resolved {
                        let (_, h) = session
                            .mirror_object(&found.handle, a, b, true)
                            .map_err(|e| op_error(index, op_name, e.code, e.message, ""))?;
                        extend_handles(&mut all_handles, h);
                    }
                    format!(
                        "mirrored {} object{} in place",
                        resolved.len(),
                        plural(resolved.len())
                    )
                }
            };
            let first_target = resolved.first();
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: summary,
                id: first_target.map(|f| f.id.clone()),
                handle: first_target.map(|f| f.handle.clone()),
                handles_touched: all_handles.touched,
                handles_added: all_handles.added,
                handles_deleted: all_handles.deleted,
                copies: Vec::new(),
                snapped,
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Copy { targets, pattern } => {
            let record = ensure_record(session, cached, source, sheet_name)?;
            let resolved = resolve_unique_targets(session, record, targets, sheet_name, op)?;
            let mut snapped = Vec::new();
            let mut copies = Vec::new();
            let mut all_handles = MutationHandles::default();

            match pattern {
                Pattern::Mirror { first, second } => {
                    let a = resolve_op_point(first, record, settings, op, &mut snapped)?;
                    let b = resolve_op_point(second, record, settings, op, &mut snapped)?;
                    for found in &resolved {
                        let (new_hex, h) = session
                            .mirror_object(&found.handle, a, b, false)
                            .map_err(|e| op_error(index, op_name, e.code, e.message, ""))?;
                        extend_handles(&mut all_handles, h);
                        copies.push(RevisionCopy {
                            id: handle_id(sheet_name, &new_hex),
                            handle: new_hex,
                            source: found.id.clone(),
                            offset: None,
                            row: None,
                            column: None,
                            item: None,
                            along: None,
                            by: [0.0; 3],
                            turned: None,
                            about: None,
                            mirrored_across: Some([a, b]),
                        });
                    }
                }
                other => {
                    let places: Vec<Place> = match other {
                        Pattern::Offsets(motions) => {
                            let mut deltas = Vec::with_capacity(motions.len());
                            for motion in motions {
                                deltas.push(resolve_motion(
                                    motion,
                                    record,
                                    settings,
                                    op,
                                    &mut snapped,
                                )?);
                            }
                            array::offsets(&deltas)
                        }
                        Pattern::Rectangular {
                            rows,
                            columns,
                            row_spacing,
                            column_spacing,
                        } => array::rectangular(*rows, *columns, *row_spacing, *column_spacing),
                        Pattern::Polar {
                            center,
                            count,
                            angle,
                            rotate,
                            base_point,
                        } => {
                            let c = resolve_op_point(center, record, settings, op, &mut snapped)?;
                            let base = match base_point {
                                Some(bp) => {
                                    resolve_op_point(bp, record, settings, op, &mut snapped)?
                                }
                                None if *rotate => [0.0; 3],
                                None => targets_center(record, &resolved, settings).ok_or_else(
                                    || {
                                        op_error(
                                        index,
                                        op_name,
                                        ErrorCode::BadValue,
                                        "could not determine a base point for the unturned polar \
                                         array; pass `base_point` explicitly",
                                        "base_point",
                                    )
                                    },
                                )?,
                            };
                            array::polar(c, *count, *angle, *rotate, base)
                        }
                        Pattern::Path {
                            path,
                            count,
                            spacing,
                            align,
                        } => {
                            let path_found =
                                resolve_target(session, record, path, sheet_name, op, "path")?;
                            let path_idx = path_found.model_index.ok_or_else(|| {
                                op_error(
                                    index,
                                    op_name,
                                    ErrorCode::BadValue,
                                    format!(
                                        "path `{}` is not a modeled curve object",
                                        path_found.id
                                    ),
                                    "path",
                                )
                            })?;
                            let path_obj = &record.record.sheet.objects[path_idx];
                            if !matches!(
                                path_obj,
                                Object::Line(_)
                                    | Object::Polyline(_)
                                    | Object::Arc(_)
                                    | Object::Circle(_)
                                    | Object::Ellipse(_)
                                    | Object::Spline(_)
                            ) {
                                return Err(op_error(
                                    index,
                                    op_name,
                                    ErrorCode::BadValue,
                                    format!(
                                        "path `{}` is not a curve (only line, polyline, arc, \
                                         circle, ellipse, and spline can be array paths)",
                                        path_found.id
                                    ),
                                    "path",
                                ));
                            }
                            let fonts = SheetFonts::find(&record.record.sheet, settings);
                            let reader = Reader {
                                sheet: &record.record.sheet,
                                fonts: &fonts,
                            };
                            let curves: Vec<Curve> = reader
                                .parts(path_obj)
                                .into_iter()
                                .map(|p| p.curve)
                                .collect();
                            array::along(&curves, *count, *spacing, *align).map_err(|why| {
                                op_error(index, op_name, ErrorCode::BadValue, why, "path")
                            })?
                        }
                        Pattern::Mirror { .. } => unreachable!(),
                    };
                    for place in &places {
                        for found in &resolved {
                            let (new_hex, h) = session
                                .copy_object(&found.handle, place.by, place.turn, place.about)
                                .map_err(|e| op_error(index, op_name, e.code, e.message, ""))?;
                            extend_handles(&mut all_handles, h);
                            let (offset, row, column, item, along) = match place.at {
                                At::Offset(i) => (Some(i), None, None, None, None),
                                At::Grid { row, column } => {
                                    (None, Some(row), Some(column), None, None)
                                }
                                At::Item(i) => (None, None, None, Some(i), None),
                                At::Along { item, distance } => {
                                    (None, None, None, Some(item), Some(distance))
                                }
                            };
                            copies.push(RevisionCopy {
                                id: handle_id(sheet_name, &new_hex),
                                handle: new_hex,
                                source: found.id.clone(),
                                offset,
                                row,
                                column,
                                item,
                                along,
                                by: place.by,
                                turned: (place.turn != 0.0).then_some(place.turn),
                                about: (place.turn != 0.0).then_some(place.about),
                                mirrored_across: None,
                            });
                        }
                    }
                }
            }
            let count = copies.len();
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "created {count} cop{}",
                    if count == 1 { "y" } else { "ies" }
                ),
                id: copies.first().map(|c| c.id.clone()),
                handle: copies.first().map(|c| c.handle.clone()),
                handles_touched: all_handles.touched,
                handles_added: all_handles.added,
                handles_deleted: all_handles.deleted,
                copies,
                snapped,
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::SetPageSetup { to, layout, set } => {
            check_add_target_sheet(to, sheet_name, op)?;
            let (msg, handles) = session
                .set_page_setup(layout.as_deref(), set)
                .map_err(|e| {
                    let f = e.field.as_deref().unwrap_or("set");
                    op_error(index, op_name, e.code, e.message, f)
                })?;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: msg,
                id: None,
                handle: handles.touched.first().cloned(),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::AttachXref { .. }
        | Body::DetachXref { .. }
        | Body::BindXref { .. }
        | Body::AddGroup { .. }
        | Body::Ungroup { .. }
        | Body::EditGroup { .. }
        | Body::SetAnnotationScale { .. }
        | Body::SetObjectScales { .. }
        | Body::SetPointStyle { .. }
        | Body::AddMLineStyle { .. } => {
            unreachable!("handled before apply_revision_op")
        }
        Body::Table {
            op: TableOp::Add,
            table: Table::Layer,
            to,
            name,
            keys,
            ..
        } => {
            check_add_target_sheet(to, sheet_name, op)?;
            let tables = session.tables_for_parsing();
            let layer = parse_new_layer_json(name, keys, &tables.linetypes).map_err(|e| {
                op_error(
                    index,
                    op_name,
                    e.code,
                    e.message,
                    e.field.as_deref().unwrap_or(""),
                )
            })?;
            let handles = session
                .add_layer(&layer)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "name"))?;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("added layer `{}`", layer.name),
                id: None,
                handle: handles.added.first().cloned(),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        Body::Table {
            op: TableOp::Change,
            table: Table::Layer,
            to,
            name,
            keys,
            ..
        } => {
            check_add_target_sheet(to, sheet_name, op)?;
            let tables = session.tables_for_parsing();
            let existing = session
                .existing_layer(name)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "name"))?;
            let updated =
                parse_layer_patch_json(&existing, keys, &tables.linetypes).map_err(|e| {
                    op_error(
                        index,
                        op_name,
                        e.code,
                        e.message,
                        e.field.as_deref().unwrap_or("set"),
                    )
                })?;
            let handles = session
                .change_layer_fields(&updated, keys)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "name"))?;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("changed layer `{}`", updated.name),
                id: None,
                handle: handles.touched.first().cloned(),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        _ => Err(op_error(
            index,
            op_name,
            ErrorCode::Usage,
            format!("operation `{op_name}` is not supported when revising an original DWG yet"),
            "",
        )),
    }
}

enum ResolvedDrawOrder {
    Front,
    Back,
    Above(String),
    Under(String),
}

fn resolve_revise_draw_order(
    session: &RevisionSession,
    record: &DwgLoaded,
    place: &DrawOrderPlace,
    space: &TargetSpace,
    sheet_name: &str,
    op: &Operation,
    field: &str,
) -> Result<ResolvedDrawOrder, Error> {
    match place {
        DrawOrderPlace::Front => Ok(ResolvedDrawOrder::Front),
        DrawOrderPlace::Back => Ok(ResolvedDrawOrder::Back),
        DrawOrderPlace::Above(target) | DrawOrderPlace::Under(target) => {
            let sub = if matches!(place, DrawOrderPlace::Above(_)) {
                "above"
            } else {
                "under"
            };
            let full_field = format!("{field}.{sub}");
            let found = resolve_target(session, record, target, sheet_name, op, &full_field)?;
            let ref_space = session.space_of_handle(&found.handle)?;
            if &ref_space != space {
                return Err(op_error(
                    op.index,
                    op.spec.name,
                    ErrorCode::BadValue,
                    format!(
                        "reference object `{}` is in a different space; draw order applies within \
                         one space",
                        found.id
                    ),
                    &full_field,
                ));
            }
            if matches!(place, DrawOrderPlace::Above(_)) {
                Ok(ResolvedDrawOrder::Above(found.handle))
            } else {
                Ok(ResolvedDrawOrder::Under(found.handle))
            }
        }
    }
}

fn apply_revise_draw_order(
    session: &mut RevisionSession,
    space: &TargetSpace,
    moving: &[String],
    order: &ResolvedDrawOrder,
) -> Result<(), Error> {
    if moving.is_empty() {
        return Ok(());
    }
    let current = session.space_handles_in_draw_order(space)?;
    let is_moving = |h: &str| moving.iter().any(|m| m.eq_ignore_ascii_case(h));
    let mut stay: Vec<String> = Vec::with_capacity(current.len());
    let mut moved: Vec<String> = Vec::with_capacity(moving.len());
    for h in &current {
        if is_moving(h) {
            moved.push(h.clone());
        } else {
            stay.push(h.clone());
        }
    }
    if moved.is_empty() {
        return Ok(());
    }
    let insert_pos = match order {
        ResolvedDrawOrder::Front => stay.len(),
        ResolvedDrawOrder::Back => 0,
        ResolvedDrawOrder::Above(ref_h) => stay
            .iter()
            .position(|h| h.eq_ignore_ascii_case(ref_h))
            .map_or(stay.len(), |p| p + 1),
        ResolvedDrawOrder::Under(ref_h) => stay
            .iter()
            .position(|h| h.eq_ignore_ascii_case(ref_h))
            .unwrap_or(0),
    };
    let mut reordered = Vec::with_capacity(current.len());
    reordered.extend_from_slice(&stay[..insert_pos]);
    reordered.extend(moved);
    reordered.extend_from_slice(&stay[insert_pos..]);
    if reordered != current {
        session.set_draw_order(space, &reordered)?;
    }
    Ok(())
}

fn extend_handles(dst: &mut MutationHandles, src: MutationHandles) {
    dst.touched.extend(src.touched);
    dst.added.extend(src.added);
    dst.deleted.extend(src.deleted);
}

fn targets_center(
    record: &DwgLoaded,
    resolved: &[ResolvedTarget],
    settings: &Settings,
) -> Option<[f64; 3]> {
    let fonts = SheetFonts::find(&record.record.sheet, settings);
    let reader = Reader {
        sheet: &record.record.sheet,
        fonts: &fonts,
    };
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut any = false;
    for found in resolved {
        let Some(idx) = found.model_index else {
            continue;
        };
        let obj = &record.record.sheet.objects[idx];
        for snap in reader.own_snaps(obj) {
            let p = snap.at;
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
            any = true;
        }
    }
    any.then(|| {
        [
            (min[0] + max[0]) / 2.0,
            (min[1] + max[1]) / 2.0,
            (min[2] + max[2]) / 2.0,
        ]
    })
}

fn decode_refusal_message(source: &Path, decode: &DecodeReport) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());
    let mut parts = Vec::new();
    if let Some(reason) = &decode.read_past_damage {
        parts.push(format!("read past damage ({reason})"));
    }
    if !decode.not_decoded.is_empty() {
        let details: Vec<String> = decode
            .not_decoded
            .iter()
            .map(|k| {
                if k.handles.is_empty() {
                    format!("{} {}", k.count, k.kind)
                } else {
                    let preview: Vec<&str> = k.handles.iter().take(5).map(String::as_str).collect();
                    let extra = if k.handles.len() > 5 { ", ..." } else { "" };
                    format!(
                        "{} {} (handles {}{extra})",
                        k.count,
                        k.kind,
                        preview.join(", ")
                    )
                }
            })
            .collect();
        parts.push(format!("could not decode {}", details.join("; ")));
    }
    if decode.skipped > 0 {
        parts.push(format!(
            "skipped {} record{}",
            decode.skipped,
            plural(decode.skipped)
        ));
    }
    if decode.recovered_errors > 0 {
        parts.push(format!(
            "recovered from {} error{}",
            decode.recovered_errors,
            plural(decode.recovered_errors)
        ));
    }
    if decode.diagnostics_truncated {
        parts.push("hit its diagnostic limit".to_owned());
    }
    if parts.is_empty() {
        parts.push("did not decode every record whole".to_owned());
    }
    format!(
        "refused to revise {name} because the DWG library {}; nothing was written. Pass \
         `--allow-loss` (or `allow_loss: true`) to allow revising this drawing and record the \
         accepted loss in the revision report (ADR-0004)",
        parts.join(", ")
    )
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn enforce_verification_gates(
    source: &Path,
    sheet_name: &str,
    dry_run: bool,
    allow_loss: bool,
    original_meta: &RevisionFile,
    parent_revision: Option<&ParentRevision>,
    units: Option<Unit>,
    decode: &DecodeReport,
    outcomes: &[RevisionOperationOutcome],
    verified: &RevisionVerification,
) -> Option<(Box<RevisionReport>, Error)> {
    // 1. Requested-edit integrity gate (ADR-0005): NEVER bypassable with
    //    `--allow-loss`. If any requested addition vanished or any requested
    //    deletion survived, refuse with zero files written.
    if !verified.requested_failures.is_empty() {
        let detail = requested_failures_refusal_message(source, &verified.requested_failures);
        let report = Box::new(RevisionReport {
            source: source.display().to_string(),
            sheet: sheet_name.to_owned(),
            dry_run,
            journal_entry: None,
            message: detail.clone(),
            original: original_meta.clone(),
            parent_revision: parent_revision.cloned(),
            revision: None,
            revision_dwg: None,
            change_list: None,
            pdf: None,
            pdf_error: None,
            plot: None,
            units,
            decode: decode.clone(),
            allowed_loss: allow_loss,
            operations: outcomes.to_vec(),
            requested_failures: verified.requested_failures.clone(),
            unexpected_changes: verified.unexpected_changes.clone(),
            checks: None,
        });
        let error = Error::new(ErrorCode::DwgWrite, detail).in_file(source);
        return Some((report, error));
    }

    // 2. Untouched-means-unchanged gate (ADR-0004): refuse collateral changes
    //    outside the requested operations unless `allow_loss: true`.
    if !verified.unexpected_changes.is_empty() && !allow_loss {
        let detail = unexpected_changes_refusal_message(source, &verified.unexpected_changes);
        let report = Box::new(RevisionReport {
            source: source.display().to_string(),
            sheet: sheet_name.to_owned(),
            dry_run,
            journal_entry: None,
            message: detail.clone(),
            original: original_meta.clone(),
            parent_revision: parent_revision.cloned(),
            revision: None,
            revision_dwg: None,
            change_list: None,
            pdf: None,
            pdf_error: None,
            plot: None,
            units,
            decode: decode.clone(),
            allowed_loss: false,
            operations: outcomes.to_vec(),
            requested_failures: Vec::new(),
            unexpected_changes: verified.unexpected_changes.clone(),
            checks: None,
        });
        let error = Error::new(ErrorCode::DwgWrite, detail).in_file(source);
        return Some((report, error));
    }
    None
}

fn requested_failures_refusal_message(source: &Path, failures: &[UnexpectedChange]) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());
    let preview: Vec<String> = failures
        .iter()
        .take(5)
        .map(|u| format!("{} {} ({})", u.kind, u.handle, u.detail))
        .collect();
    let extra = if failures.len() > 5 { ", ..." } else { "" };
    format!(
        "refused to revise {name} because {} requested edit{} did not verify after writing \
         ({}{extra}); nothing was written (ADR-0005)",
        failures.len(),
        plural(failures.len()),
        preview.join("; ")
    )
}

fn unexpected_changes_refusal_message(source: &Path, unexpected: &[UnexpectedChange]) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());
    let preview: Vec<String> = unexpected
        .iter()
        .take(5)
        .map(|u| format!("{} {} ({})", u.kind, u.handle, u.detail))
        .collect();
    let extra = if unexpected.len() > 5 { ", ..." } else { "" };
    format!(
        "refused to revise {name} because writing the revision introduced {} unexpected \
         change{} outside the requested operations ({}{extra}); nothing was written. Pass \
         `--allow-loss` (or `allow_loss: true`) to allow saving this revision and record these \
         changes in the report (ADR-0004)",
        unexpected.len(),
        plural(unexpected.len()),
        preview.join("; ")
    )
}

#[allow(clippy::too_many_arguments)]
fn revision_message(
    source: &Path,
    revision_dwg: &Path,
    dwg_version: &str,
    ops: usize,
    dry_run: bool,
    allowed_loss: bool,
    unexpected: usize,
    pdf: Option<&str>,
    pdf_error: Option<&Error>,
) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut msg = if dry_run {
        format!(
            "a dry run, which wrote nothing: would revise {name} ({dwg_version}) with {ops} \
             operation{} into {}",
            plural(ops),
            revision_dwg.display()
        )
    } else {
        format!(
            "revised {name} ({dwg_version}) with {ops} operation{} into {}",
            plural(ops),
            revision_dwg.display()
        )
    };
    if allowed_loss {
        msg.push_str(" (allow_loss accepted; decode losses recorded in the report)");
    }
    if unexpected > 0 {
        msg.push_str(&format!(
            "; {unexpected} unexpected change{} outside the requested operations",
            plural(unexpected)
        ));
    }
    if let Some(pdf) = pdf {
        msg.push_str(&format!("; plotted PDF to {pdf}"));
    } else if let Some(err) = pdf_error {
        msg.push_str(&format!("; PDF not plotted ({})", err.message));
    }
    msg
}

#[derive(Clone, Copy)]
enum OriginalCornerOp {
    Fillet(f64),
    Chamfer(ChamferSpec),
}

impl OriginalCornerOp {
    fn verb(self) -> &'static str {
        match self {
            Self::Fillet(_) => "filleted",
            Self::Chamfer(_) => "chamfered",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Fillet(_) => "fillet",
            Self::Chamfer(_) => "chamfer",
        }
    }

    fn size_field(self) -> &'static str {
        match self {
            Self::Fillet(_) => "radius",
            Self::Chamfer(_) => "distance",
        }
    }
}

fn apply_original_corner(
    session: &mut RevisionSession,
    loaded: &DwgLoaded,
    sheet_name: &str,
    op: &Operation,
    mode: &CornerMode,
    kind: OriginalCornerOp,
    settings: &Settings,
) -> Result<RevisionOperationOutcome, Error> {
    let index = op.index;
    let op_name = op.spec.name;
    let fonts = SheetFonts::find(&loaded.record.sheet, settings);
    let start_of = |t: &crate::text::Text| fonts.start(t);

    match mode {
        CornerMode::Polyline { target } => {
            let found = resolve_target(session, loaded, target, sheet_name, op, "target")?;
            check_unlocked(session, &found.handle, &found.id, op, "target")?;
            let obj_idx = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(found.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}` to {} it", found.id, kind.noun()),
                        "target",
                    )
                })?;
            let base = loaded.record.sheet.objects[obj_idx].clone();
            let obj_name = loaded.record.objects[obj_idx].name.clone();
            let res = match kind {
                OriginalCornerOp::Fillet(r) => compute_fillet_polyline(&base, r),
                OriginalCornerOp::Chamfer(spec) => compute_chamfer_polyline(&base, spec),
            }
            .map_err(|e| op_error(index, op_name, e.code, e.message, kind.size_field()))?;

            let space = session.space_of_handle(&found.handle)?;
            let local_id = found
                .id
                .split_once('/')
                .map(|(_, l)| l)
                .filter(|l| !l.starts_with(HANDLE_ID));
            let del = session.delete_object(&found.handle)?;
            let (new_hex, added) = session.add_object(
                &res.polyline,
                &space,
                sheet_name,
                local_id,
                obj_name.as_deref(),
                &start_of,
            )?;
            let mut handles_touched = del.touched;
            handles_touched.extend(added.touched);
            let kept_id = match local_id {
                Some(explicit) => format!("{sheet_name}/{explicit}"),
                None => handle_id(sheet_name, &new_hex),
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "{} {} corner{} of `{}`",
                    kind.verb(),
                    res.corners_modified,
                    if res.corners_modified == 1 { "" } else { "s" },
                    found.id
                ),
                id: Some(kept_id),
                handle: Some(new_hex),
                handles_touched,
                handles_added: added.added,
                handles_deleted: del.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
        CornerMode::TwoObjects {
            first,
            second,
            trim,
            first_end,
            second_end,
            new_id,
            layer,
        } => {
            let f1 = resolve_target(session, loaded, first, sheet_name, op, "first")?;
            let f2 = resolve_target(session, loaded, second, sheet_name, op, "second")?;
            if f1.handle == f2.handle {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::BadValue,
                    format!(
                        "`first` and `second` both name `{}`; to {} a single polyline's \
                         corners, pass it under `target`",
                        f1.id,
                        kind.noun()
                    ),
                    "second",
                ));
            }
            if *trim {
                check_unlocked(session, &f1.handle, &f1.id, op, "first")?;
                check_unlocked(session, &f2.handle, &f2.id, op, "second")?;
            }
            let idx1 = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(f1.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}`", f1.id),
                        "first",
                    )
                })?;
            let idx2 = (0..loaded.record.sheet.objects.len())
                .find(|&i| loaded.record.handle_of(i) == Some(f2.handle.as_str()))
                .ok_or_else(|| {
                    op_error(
                        index,
                        op_name,
                        ErrorCode::UnknownName,
                        format!("could not read `{}`", f2.id),
                        "second",
                    )
                })?;
            if !loaded.record.sheet.same_space(idx1, idx2) {
                return Err(op_error(
                    index,
                    op_name,
                    ErrorCode::BadValue,
                    format!("`{}` is in a different space from `{}`", f2.id, f1.id),
                    "second",
                ));
            }
            let obj1 = loaded.record.sheet.objects[idx1].clone();
            let obj2 = loaded.record.sheet.objects[idx2].clone();
            let name1 = loaded.record.objects[idx1].name.clone();
            let name2 = loaded.record.objects[idx2].name.clone();

            let res = match kind {
                OriginalCornerOp::Fillet(r) => compute_fillet_two(
                    &obj1,
                    &obj2,
                    r,
                    *trim,
                    *first_end,
                    *second_end,
                    layer.as_deref(),
                ),
                OriginalCornerOp::Chamfer(spec) => compute_chamfer_two(
                    &obj1,
                    &obj2,
                    spec,
                    *trim,
                    *first_end,
                    *second_end,
                    layer.as_deref(),
                ),
            }
            .map_err(|e| op_error(index, op_name, e.code, e.message, kind.size_field()))?;

            let space = session.space_of_handle(&f1.handle)?;
            let mut handles_touched = Vec::new();
            let mut handles_added = Vec::new();
            let mut handles_deleted = Vec::new();
            let mut last_hex = None;

            for (updated_opt, found, obj_name) in
                [(&res.first, &f1, name1), (&res.second, &f2, name2)]
            {
                if let Some(updated) = updated_opt {
                    let local = found
                        .id
                        .split_once('/')
                        .map(|(_, l)| l)
                        .filter(|l| !l.starts_with(HANDLE_ID));
                    let del = session.delete_object(&found.handle)?;
                    let (hex, added) = session.add_object(
                        updated,
                        &space,
                        sheet_name,
                        local,
                        obj_name.as_deref(),
                        &start_of,
                    )?;
                    handles_touched.extend(del.touched);
                    handles_touched.extend(added.touched);
                    handles_added.extend(added.added);
                    handles_deleted.extend(del.deleted);
                    last_hex = Some(hex);
                }
            }

            let mut created_id = None;
            let mut created_hex = None;
            if let Some(added_obj) = &res.added {
                let (ahex, aadd) = session.add_object(
                    added_obj,
                    &space,
                    sheet_name,
                    new_id.as_deref(),
                    None,
                    &start_of,
                )?;
                handles_touched.extend(aadd.touched);
                handles_added.extend(aadd.added);
                created_id = Some(match new_id {
                    Some(explicit) => format!("{sheet_name}/{explicit}"),
                    None => handle_id(sheet_name, &ahex),
                });
                created_hex = Some(ahex);
            }

            let message = match &created_id {
                Some(cid) => format!("{} `{}` and `{}` with `{cid}`", kind.verb(), f1.id, f2.id),
                None => format!(
                    "{} `{}` and `{}` to a sharp corner",
                    kind.verb(),
                    f1.id,
                    f2.id
                ),
            };
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message,
                id: created_id,
                handle: created_hex.or(last_hex),
                handles_touched,
                handles_added,
                handles_deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                transformed: Vec::new(),
                created: Vec::new(),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn revise_add_group(
    session: &mut RevisionSession,
    cached: &mut Option<DwgLoaded>,
    source: &Path,
    sheet_name: &str,
    op: &Operation,
    to: &AddTo,
    name: &str,
    targets: &[(Target, String)],
    description: &str,
    selectable: bool,
) -> Result<RevisionOperationOutcome, Error> {
    check_add_target_sheet(to, sheet_name, op)?;
    let record = ensure_record(session, cached, source, sheet_name)?;
    let mut member_hexes: Vec<String> = Vec::with_capacity(targets.len());
    for (target, field) in targets {
        for found in resolve_target_all(session, record, target, sheet_name, op, field)? {
            if !member_hexes
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&found.handle))
            {
                member_hexes.push(found.handle);
            }
        }
    }
    let handles = session
        .create_group(name, description, selectable, &member_hexes)
        .map_err(|e| op_error(op.index, op.spec.name, e.code, e.message, "name"))?;
    Ok(RevisionOperationOutcome {
        index: op.index,
        op: op.spec.name.to_owned(),
        ok: true,
        message: format!(
            "added group `{name}` with {} object{}",
            member_hexes.len(),
            plural(member_hexes.len())
        ),
        id: None,
        handle: handles.added.first().cloned(),
        handles_touched: handles.touched,
        handles_added: handles.added,
        handles_deleted: handles.deleted,
        copies: Vec::new(),
        snapped: Vec::new(),
        replacements: Vec::new(),
        attributes_set: Vec::new(),
        split_id: None,
        deleted: Vec::new(),
        transformed: Vec::new(),
        created: Vec::new(),
    })
}

fn revise_ungroup(
    session: &mut RevisionSession,
    sheet_name: &str,
    op: &Operation,
    to: &AddTo,
    name: &str,
) -> Result<RevisionOperationOutcome, Error> {
    check_add_target_sheet(to, sheet_name, op)?;
    let (canonical, handles) = session
        .ungroup(name)
        .map_err(|e| op_error(op.index, op.spec.name, e.code, e.message, "name"))?;
    Ok(RevisionOperationOutcome {
        index: op.index,
        op: op.spec.name.to_owned(),
        ok: true,
        message: format!("ungrouped `{canonical}`"),
        id: None,
        handle: handles.deleted.first().cloned(),
        handles_touched: handles.touched,
        handles_added: handles.added,
        handles_deleted: handles.deleted,
        copies: Vec::new(),
        snapped: Vec::new(),
        replacements: Vec::new(),
        attributes_set: Vec::new(),
        split_id: None,
        deleted: Vec::new(),
        transformed: Vec::new(),
        created: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn revise_edit_group(
    session: &mut RevisionSession,
    cached: &mut Option<DwgLoaded>,
    source: &Path,
    sheet_name: &str,
    op: &Operation,
    to: &AddTo,
    name: &str,
    add: &[(Target, String)],
    remove: &[(Target, String)],
    rename: Option<&str>,
    description: Option<&str>,
    selectable: Option<bool>,
) -> Result<RevisionOperationOutcome, Error> {
    check_add_target_sheet(to, sheet_name, op)?;
    let mut add_hexes: Vec<String> = Vec::new();
    let mut remove_hexes: Vec<String> = Vec::new();
    if !add.is_empty() || !remove.is_empty() {
        let record = ensure_record(session, cached, source, sheet_name)?;
        for (target, field) in add {
            for found in resolve_target_all(session, record, target, sheet_name, op, field)? {
                if !add_hexes
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case(&found.handle))
                {
                    add_hexes.push(found.handle);
                }
            }
        }
        for (target, field) in remove {
            for found in resolve_target_all(session, record, target, sheet_name, op, field)? {
                if !remove_hexes
                    .iter()
                    .any(|h| h.eq_ignore_ascii_case(&found.handle))
                {
                    remove_hexes.push(found.handle);
                }
            }
        }
    }
    let (final_name, handles) = session
        .edit_group(
            name,
            &add_hexes,
            &remove_hexes,
            rename,
            description,
            selectable,
        )
        .map_err(|e| op_error(op.index, op.spec.name, e.code, e.message, "name"))?;
    Ok(RevisionOperationOutcome {
        index: op.index,
        op: op.spec.name.to_owned(),
        ok: true,
        message: format!("edited group `{final_name}`"),
        id: None,
        handle: handles.touched.first().cloned(),
        handles_touched: handles.touched,
        handles_added: handles.added,
        handles_deleted: handles.deleted,
        copies: Vec::new(),
        snapped: Vec::new(),
        replacements: Vec::new(),
        attributes_set: Vec::new(),
        split_id: None,
        deleted: Vec::new(),
        transformed: Vec::new(),
        created: Vec::new(),
    })
}

#[cfg(test)]
#[path = "revise_tests.rs"]
mod tests;
