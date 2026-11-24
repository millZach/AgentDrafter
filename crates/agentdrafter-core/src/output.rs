//! What every output shares: reading the record it is drawn from, with
//! every file read for its stamp, and writing it safely.
//!
//! An output is written whole or not at all, and an existing file is
//! replaced only when asked and only when it is an output AgentDrafter
//! produced, as its stamp shows: anything else may be someone's own file.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(any(unix, windows))]
use crate::build::file_id_of_file;
#[cfg(not(any(unix, windows)))]
use crate::build::FileId;
use crate::description::sheet_description_at;
use crate::dwg;
use crate::error::{Error, ErrorCode};
use crate::model::{Paper, RecordsOfKind, Sheet, UnresolvedMedia, UnresolvedXref};
use crate::stamp::{self, SourceKind};
use crate::units::Unit;

/// The kind of output being written, for what errors say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputKind {
    Png,
    Pdf,
}

impl OutputKind {
    /// What the output is called: `PNG` or `PDF`.
    fn name(self) -> &'static str {
        match self {
            OutputKind::Png => "PNG",
            OutputKind::Pdf => "PDF",
        }
    }

    /// What makes it: a render or a plot.
    fn made_by(self) -> &'static str {
        match self {
            OutputKind::Png => "render",
            OutputKind::Pdf => "plot",
        }
    }

    /// How AgentDrafter made one: rendered or plotted.
    fn made(self) -> &'static str {
        match self {
            OutputKind::Png => "rendered",
            OutputKind::Pdf => "plotted",
        }
    }

    /// Whether `bytes` are a file of this kind, by how every such file
    /// starts.
    fn is_kind_of(self, bytes: &[u8]) -> bool {
        bytes.starts_with(match self {
            OutputKind::Png => b"\x89PNG\r\n\x1a\n".as_slice(),
            OutputKind::Pdf => b"%PDF-".as_slice(),
        })
    }
}

/// A record read to draw an output, with every file read for it.
pub(crate) struct Record {
    pub sheet: Sheet,
    /// The drawing's unit; none when a DWG has none the core knows. The
    /// sheet is then drawn as millimetres.
    pub units: Option<Unit>,
    /// How many objects of a DWG the core has no model for yet.
    pub objects_not_drawn: usize,
    /// Those objects by kind, with the handle of each.
    pub not_drawn: Vec<RecordsOfKind>,
    /// The ID and name of each of the sheet's objects, in order, for a DWG:
    /// the ID AgentDrafter wrote on it, or the drawing's name joined to its
    /// handle (`crate::original`). Empty for a sheet description.
    pub ids: Vec<(String, Option<String>)>,
    /// Every file read, the record first, each with its kind and the bytes
    /// read.
    pub files: Vec<(SourceKind, PathBuf, Vec<u8>)>,
    /// External references that could not be found or loaded.
    pub unresolved_xrefs: Vec<UnresolvedXref>,
    /// Raster images or PDF underlays that could not be found or decoded.
    pub unresolved_media: Vec<UnresolvedMedia>,
}

impl Record {
    /// The files read, as a stamp takes them.
    pub(crate) fn read(&self) -> Vec<(SourceKind, &Path, &[u8])> {
        self.files
            .iter()
            .map(|(kind, path, bytes)| (*kind, path.as_path(), bytes.as_slice()))
            .collect()
    }
}

/// Reads the record at `source`: a DWG when its name ends `.dwg`, and
/// otherwise a sheet description, with the block library it names.
pub(crate) fn read_record(source: &Path) -> Result<Record, Error> {
    read_record_with_recovery(source, false)
}

/// Reads the record at `source`, optionally falling back to read-past-damage
/// recovery on a damaged DWG when `read_past_damage` is true (used only when
/// explicitly requested by `display-list` for view-only inspection of a
/// recoverable damaged original under ADR-0004).
pub(crate) fn read_record_with_recovery(
    source: &Path,
    read_past_damage: bool,
) -> Result<Record, Error> {
    let kind = SourceKind::of(source);
    let mut file = File::open(source).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the {}: {e}", kind.words()),
        )
        .in_file(source)
    })?;
    #[cfg(any(unix, windows))]
    let source_id = file_id_of_file(&file);
    #[cfg(not(any(unix, windows)))]
    let source_id: Option<FileId> = None;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the {}: {e}", kind.words()),
        )
        .in_file(source)
    })?;
    drop(file);
    if kind.is_drawing() {
        let drawing = if read_past_damage {
            dwg::read_bytes_past_damage(&bytes, source)?
        } else {
            dwg::read_bytes(&bytes, source)?
        };
        let ids = crate::original::ids_of_with_id(source, source_id, &drawing);
        // Zach decided on 2026-11-13 that a DWG plots on the paper its
        // sheet declares, read from the set its build mark names; the DWG
        // itself holds no paper. Read the build mark from the exact bytes
        // already in memory and match rows against `source_id` so `source`
        // is never reopened.
        let units = drawing.units.unwrap_or(Unit::Mm);
        let found_paper = crate::set::paper_of_dwg_bytes(source, source_id, &bytes)?;
        let mut files = vec![(kind, source.to_path_buf(), bytes)];
        let paper = found_paper.and_then(|found| {
            files.extend(found.read);
            // The paper in the DWG's own unit, should the set's have moved on.
            found.paper.map(|paper| Paper {
                width: found.units.convert(paper.width, units),
                height: found.units.convert(paper.height, units),
                ..paper
            })
        });
        let mut sheet = Sheet {
            units: drawing.units.unwrap_or(Unit::Mm),
            block_library: None,
            linetypes: drawing.linetypes,
            layers: drawing.layers,
            text_styles: drawing.text_styles,
            table_styles: drawing.table_styles,
            mleader_styles: drawing.mleader_styles,
            dimension_styles: drawing.dimension_styles,
            blocks: drawing.blocks,
            xrefs: drawing.xrefs,
            resolved_xrefs: Vec::new(),
            resolved_media: Vec::new(),
            objects: drawing.objects,
            // A DWG that opens on its layout plots on its layout's paper.
            paper: drawing.paper.or(paper),
            page_setup: drawing.page_setup,
            ltscale: drawing.ltscale,
            layout: drawing.layout,
            other_layouts: drawing.other_layouts,
            wipeout_frame: drawing.wipeout_frame,
            image_frame: drawing.image_frame,
            custom_properties: drawing.custom_properties,
        };
        let resolved_xrefs = crate::xref::resolve_sheet_xrefs(&mut sheet, source);
        files.extend(resolved_xrefs.files);
        let resolved_media = crate::media::resolve_sheet_media_for_host(&mut sheet, source);
        files.extend(resolved_media.files);
        return Ok(Record {
            sheet,
            units: drawing.units,
            objects_not_drawn: drawing.objects_not_read,
            not_drawn: drawing.not_read,
            ids,
            files,
            unresolved_xrefs: resolved_xrefs.unresolved,
            unresolved_media: resolved_media.unresolved,
        });
    }
    let text = String::from_utf8(bytes.clone()).map_err(|_| {
        Error::new(
            ErrorCode::ReadFailed,
            "could not read the sheet description: it is not UTF-8 text",
        )
        .in_file(source)
    })?;
    let (mut sheet, library) = sheet_description_at(&text, source)?;
    let mut files = vec![(kind, source.to_path_buf(), bytes)];
    if let Some(library) = library {
        files.push((SourceKind::BlockLibrary, library.path, library.bytes));
    }
    let resolved_xrefs = crate::xref::resolve_sheet_xrefs(&mut sheet, source);
    files.extend(resolved_xrefs.files);
    let resolved_media = crate::media::resolve_sheet_media_for_host(&mut sheet, source);
    files.extend(resolved_media.files);
    Ok(Record {
        units: Some(sheet.units),
        sheet,
        objects_not_drawn: 0,
        not_drawn: Vec::new(),
        ids: Vec::new(),
        files,
        unresolved_xrefs: resolved_xrefs.unresolved,
        unresolved_media: resolved_media.unresolved,
    })
}

/// How many objects of each type the sheet holds, such as `{"line": 1}`.
pub(crate) fn object_counts(sheet: &Sheet) -> BTreeMap<String, usize> {
    let mut objects = BTreeMap::new();
    for object in &sheet.objects {
        *objects.entry(object.type_name().to_owned()).or_insert(0) += 1;
    }
    objects
}

/// Refuses to write over the file at `output` unless `overwrite` asks to
/// and it is an output of the same kind AgentDrafter produced, as its stamp
/// shows: a render replaces only a PNG it rendered, and a plot only a PDF
/// it plotted. Anything else may be someone's own file, and is never
/// overwritten. The output may never be the record it is drawn from.
pub(crate) fn refuse_to_replace(
    source: &Path,
    output: &Path,
    overwrite: bool,
    kind: OutputKind,
) -> Result<(), Error> {
    if fs::symlink_metadata(output).is_err() {
        return Ok(());
    }
    if let (Ok(a), Ok(b)) = (fs::canonicalize(source), fs::canonicalize(output)) {
        if a == b {
            return Err(Error::usage(format!(
                "the output path is the source itself; choose another path for the {}",
                kind.name()
            ))
            .in_file(output));
        }
    }
    if !overwrite {
        return Err(Error::new(
            ErrorCode::OutputExists,
            format!(
                "a file is already at the output path; ask to overwrite it to replace a previous \
                 {}",
                kind.made_by()
            ),
        )
        .in_file(output));
    }
    let replaceable = fs::read(output)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            if kind.is_kind_of(&bytes) {
                stamp::stamp_of(&bytes)
            } else {
                Err(format!(
                    "the file is not a {0} AgentDrafter {1}: it is not a {0}",
                    kind.name(),
                    kind.made()
                ))
            }
        });
    match replaceable {
        Ok(_) => Ok(()),
        Err(why) => Err(Error::new(
            ErrorCode::NotAnOutput,
            format!(
                "{why}; it may be someone's own file, which is never overwritten, so choose another \
                 output path"
            ),
        )
        .in_file(output)),
    }
}

/// Writes `bytes` to a temporary file beside `path`, then moves it onto
/// `path`, so a reader never sees a half-written output.
///
/// The temporary file's name is short and does not hold the output's own,
/// so an output whose name is as long as the system allows, as a drawing
/// set's sheet may have, can still be written. It holds the process ID and
/// a count, so no two writes share one.
pub(crate) fn write_whole(path: &Path, bytes: &[u8], kind: OutputKind) -> Result<(), Error> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    if path.file_name().is_none() {
        return Err(Error::usage("the output path does not name a file").in_file(path));
    }
    let temporary = path.with_file_name(format!(
        ".agentdrafter-{}-{}.{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed),
        kind.name().to_ascii_lowercase()
    ));
    let written = File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    written.map_err(|e| {
        let _ = fs::remove_file(&temporary);
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not write the {}: {e}", kind.name()),
        )
        .in_file(path)
    })
}
