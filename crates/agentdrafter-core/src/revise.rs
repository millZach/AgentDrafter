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
use crate::change::operations::{
    op_error, read_change_set_for_original, AddTo, Body, Motion, OpPoint, Operation, Pattern,
    Table, TableOp, Target, TransformBy,
};
use crate::change::original_journal::OriginalJournal;
use crate::change::{HowChanged, ObjectChanged, OriginalRevisionMeta, SnapResolved};
use crate::check::{check_dwg_bytes, CheckReport};
use crate::description::{parse_added_object_json, parse_layer_patch_json, parse_new_layer_json};
use crate::dwg::{
    LockedLayerHit, MutationHandles, RevisionSession, RevisionVerification, TargetSpace,
    UnexpectedChange,
};
use crate::error::{Error, ErrorCode};
use crate::geometry::curve::Curve;
use crate::geometry::Reader;
use crate::model::{DecodeReport, Object};
#[cfg(any(unix, windows))]
use crate::original::regular_file_id_no_follow;
use crate::original::{
    dwg_loaded_from_drawing, handle_id, is_dwg, is_dwg_file, on_disk_path, plural, DwgLoaded,
    HANDLE_ID,
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
    if !is_dwg_file(source) {
        return Reply::from(Err(Error::usage(
            "revising an original requires a `.dwg` file, not a folder",
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
            Ok(PreparedRevision::Ready(ready)) => Reply::from(Ok(ready.report)),
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
            };
            match oj.commit_revision(&locked, &request.change_set, objects, meta, &new_files) {
                Ok(entry_num) => {
                    ready.report.journal_entry = Some(entry_num);
                    Reply::from(Ok(ready.report))
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
    report: RevisionReport,
}

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
    let sheet_name = oj.sheet.clone();
    let original_file_name = resolved_source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());

    // Validate/choose output paths before touching anything on disk.
    let paths = resolve_output_paths(source, source_id, request.output.as_deref(), request.no_pdf)?;

    // Strictly parse the original DWG into an in-place revision session.
    let mut session = RevisionSession::open(original_bytes, source)?;
    let original_meta = RevisionFile {
        file: original_file_name,
        sha256: original_sha256,
        dwg_version: session.dwg_version().to_owned(),
    };
    let units = session.initial_contents().units;
    let effective_units = units.unwrap_or(Unit::Mm);
    let decode = session.decode_report().clone();

    // Refuse DWG versions acadrust cannot write back without converting (ADR-0006).
    session
        .check_writable_version()
        .map_err(|e| e.or_in_file(source))?;

    // Decode-loss gate (ADR-0004): refuse unless `allow_loss: true`.
    if !decode.decoded && !request.allow_loss {
        let detail = decode_refusal_message(source, &decode);
        let report = RevisionReport {
            source: source.display().to_string(),
            sheet: sheet_name,
            dry_run: request.dry_run,
            journal_entry: None,
            message: detail.clone(),
            original: original_meta,
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
        };
        let error = Error::new(ErrorCode::IncompleteDecode, detail).in_file(source);
        return Ok(PreparedRevision::Refused(Box::new(report), error));
    }

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
    let mut cached_record: Option<DwgLoaded> = Some(dwg_loaded_from_drawing(
        source,
        sheet_name.clone(),
        session.initial_contents().clone(),
        None,
    ));

    for op in &parsed.operations {
        let outcome = apply_revision_op(
            &mut session,
            &mut cached_record,
            source,
            &sheet_name,
            effective_units,
            op,
            settings,
        )?;
        outcomes.push(outcome);
        // Invalidate cached SheetRecord so subsequent ops see updated geometry/IDs.
        cached_record = None;
    }

    // Serialize revision bytes in the original's DWG version, re-read strictly,
    // and run the structural diff self-check.
    let verified = session.serialize_and_verify(&paths.dwg)?;
    if let Some((report, error)) = enforce_verification_gates(
        source,
        &sheet_name,
        request.dry_run,
        request.allow_loss,
        &original_meta,
        units,
        &decode,
        &outcomes,
        &verified,
    ) {
        return Ok(PreparedRevision::Refused(Box::new(report), error));
    }
    let revision_bytes = verified.bytes;
    let unexpected_changes = verified.unexpected_changes;

    let revision_sha256 = fingerprint(&revision_bytes);
    let revision_file_name = paths
        .dwg
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| paths.dwg.display().to_string());
    let revision_meta = RevisionFile {
        file: revision_file_name,
        sha256: revision_sha256,
        dwg_version: session.dwg_version().to_owned(),
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

    let change_list_file = RevisionChangeListFile {
        format: 1,
        original: original_meta.clone(),
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

    let report = RevisionReport {
        source: source.display().to_string(),
        sheet: sheet_name,
        dry_run: request.dry_run,
        journal_entry: None,
        message,
        original: original_meta,
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
    };
    Ok(PreparedRevision::Ready(Box::new(ReadyRevision {
        paths,
        revision_bytes,
        change_list_bytes,
        pdf_bytes,
        report,
    })))
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
    if let Some(output) = explicit_output {
        if output.file_name().is_none() {
            return Err(
                Error::usage("the revision's output path does not name a file")
                    .at_field("output")
                    .in_file(output),
            );
        }
        if !is_dwg(output) {
            return Err(Error::usage("the revision's output path must end `.dwg`")
                .at_field("output")
                .in_file(output));
        }
        let changes = with_dwg_replaced(output, REVISION_CHANGES_EXTENSION);
        let pdf = (!no_pdf).then(|| with_dwg_replaced(output, "pdf"));
        refuse_existing_or_original(source, source_id, output, "revision DWG")?;
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

    // Default beside the original: `<stem>.rev1.dwg`, `<stem>.rev2.dwg`, ...
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    for n in 1u64..=100_000 {
        let dwg = source.with_file_name(format!("{stem}.rev{n}.dwg"));
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
    }
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
        let found = resolve_target(session, record, target, sheet_name, op, field)?;
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
        Body::Add { to, keys, snaps } => {
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
                            | "insert"
                    ) =>
                {
                    object
                }
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
            let found = resolve_target(session, record, target, sheet_name, op, "target")?;
            let moves_children = set.contains_key("position")
                || set.contains_key("rotation")
                || set.contains_key("scale");
            if moves_children {
                check_unlocked_with_children(session, &found.handle, &found.id, op, "target")?;
            } else {
                check_unlocked(session, &found.handle, &found.id, op, "target")?;
            }
            let existing_names: HashSet<String> = record
                .record
                .objects
                .iter()
                .filter_map(|o| o.name.as_ref().map(|n| n.to_lowercase()))
                .collect();
            let fonts = SheetFonts::find(&record.record.sheet, settings);
            let start_of = |t: &crate::text::Text| fonts.start(t);
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
            })
        }
        Body::Move { target, motion } => {
            let record = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, record, target, sheet_name, op, "target")?;
            check_unlocked_with_children(session, &found.handle, &found.id, op, "target")?;
            let mut snapped = Vec::new();
            let delta = resolve_motion(motion, record, settings, op, &mut snapped)?;
            let handles = session
                .move_object(&found.handle, delta)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "target"))?;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!(
                    "moved `{}` (handle `{}`) by [{}, {}, {}]",
                    found.id, found.handle, delta[0], delta[1], delta[2]
                ),
                id: Some(found.id),
                handle: Some(found.handle),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped,
            })
        }
        Body::Delete { target } => {
            let record = ensure_record(session, cached, source, sheet_name)?;
            let found = resolve_target(session, record, target, sheet_name, op, "target")?;
            check_unlocked_with_children(session, &found.handle, &found.id, op, "target")?;
            let handles = session
                .delete_object(&found.handle)
                .map_err(|e| op_error(index, op_name, e.code, e.message, "target"))?;
            Ok(RevisionOperationOutcome {
                index,
                op: op_name.to_owned(),
                ok: true,
                message: format!("deleted `{}` (handle `{}`)", found.id, found.handle),
                id: Some(found.id),
                handle: Some(found.handle),
                handles_touched: handles.touched,
                handles_added: handles.added,
                handles_deleted: handles.deleted,
                copies: Vec::new(),
                snapped: Vec::new(),
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
            })
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

#[allow(clippy::too_many_arguments)]
fn enforce_verification_gates(
    source: &Path,
    sheet_name: &str,
    dry_run: bool,
    allow_loss: bool,
    original_meta: &RevisionFile,
    units: Option<Unit>,
    decode: &DecodeReport,
    outcomes: &[RevisionOperationOutcome],
    verified: &RevisionVerification,
) -> Option<(RevisionReport, Error)> {
    // 1. Requested-edit integrity gate (ADR-0005): NEVER bypassable with
    //    `--allow-loss`. If any requested addition vanished or any requested
    //    deletion survived, refuse with zero files written.
    if !verified.requested_failures.is_empty() {
        let detail = requested_failures_refusal_message(source, &verified.requested_failures);
        let report = RevisionReport {
            source: source.display().to_string(),
            sheet: sheet_name.to_owned(),
            dry_run,
            journal_entry: None,
            message: detail.clone(),
            original: original_meta.clone(),
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
        };
        let error = Error::new(ErrorCode::DwgWrite, detail).in_file(source);
        return Some((report, error));
    }

    // 2. Untouched-means-unchanged gate (ADR-0004): refuse collateral changes
    //    outside the requested operations unless `allow_loss: true`.
    if !verified.unexpected_changes.is_empty() && !allow_loss {
        let detail = unexpected_changes_refusal_message(source, &verified.unexpected_changes);
        let report = RevisionReport {
            source: source.display().to_string(),
            sheet: sheet_name.to_owned(),
            dry_run,
            journal_entry: None,
            message: detail.clone(),
            original: original_meta.clone(),
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
        };
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

#[cfg(test)]
#[path = "revise_tests.rs"]
mod tests;
