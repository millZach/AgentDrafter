//! Reads an original whole: a DWG someone else drew, such as a plant's
//! drawing, which AgentDrafter never writes to (ADR-0004).
//!
//! A read says, for the drawing, whether the DWG library decoded every
//! record of the file, and names by kind, count and handle each it did not
//! ([`DecodeReport`]); it names in the same way each object of the sheet
//! the core has no model for, which no output draws. Every object the core
//! reads gets an ID an agent can name it by: the ID AgentDrafter's own
//! extended data on it gives, on a drawing AgentDrafter built, and
//! otherwise the drawing's name joined to its handle, such as
//! `P-101/handle:2F`. A handle is the DWG's own name for a record, so the
//! ID is the same each time the same file is read.
//!
//! The read writes the drawing's text mirror beside it (ADR-0003): a
//! readable text rendering of what the core read, for reading and
//! comparing, which is never the record. Editing it changes nothing. It is
//! written whole or not at all, and replaces only a mirror AgentDrafter
//! wrote, as its first line shows; any other file may be someone's own,
//! and the original itself is never written.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[cfg(any(unix, windows))]
use crate::build::file_id_of_file;
#[cfg(not(any(unix, windows)))]
use crate::build::is_same_file;
use crate::build::{file_id_of_path, FileId};
use crate::description::{
    number_text, DwgOrigin, ObjectRecord, SheetRecord, Written, ID_SEPARATOR,
};
use crate::dwg;
use crate::error::{Error, ErrorCode};
use crate::model::{DrawingContents, IdAndName, Layout, Object, Paper, RecordsOfKind, Sheet};
use crate::stamp::fingerprint;
use crate::units::Unit;

/// How a text mirror's first line starts. A mirror is replaced only when
/// its first line starts so.
pub const MIRROR_MARK: &str = "# AgentDrafter's text mirror of ";

/// What a text mirror's name ends with, after the original's name without
/// its `.dwg`: `P-101.dwg`'s mirror is `P-101.mirror.yaml`.
pub const MIRROR_EXTENSION: &str = "mirror.yaml";

/// What joins an original's name to an object's handle in the object's
/// ID, after [`ID_SEPARATOR`]: `P-101/handle:2F`.
pub const HANDLE_ID: &str = "handle:";

/// What `agentdrafter read` is asked to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    /// The original to read: a DWG.
    pub source: PathBuf,
    /// Where to write the text mirror. Without it, beside the original,
    /// named after it: `P-101.mirror.yaml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<PathBuf>,
    /// Read only, and write no mirror.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_mirror: bool,
}

/// What a read found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadReport {
    /// The original read.
    pub source: String,
    /// The name its objects' IDs start with: the original's name without
    /// its `.dwg`.
    pub sheet: String,
    /// What was read, in words.
    pub message: String,
    /// The DWG version the original is saved in, such as `AC1032` for
    /// 2018, which a revision of it keeps (ADR-0006).
    pub dwg_version: String,
    /// The drawing's unit; none when it states none the core knows.
    pub units: Option<Unit>,
    /// True when the DWG library decoded every record of the file.
    pub decoded: bool,
    /// The records the DWG library read but could not decode, by kind,
    /// with the handle of each.
    pub not_decoded: Vec<RecordsOfKind>,
    /// How many records the DWG library could not read at all.
    pub skipped: usize,
    /// How many errors the DWG library recovered from while reading.
    pub recovered_errors: usize,
    /// How many non-fatal reader warnings the DWG library reported while
    /// reading (such as inconsistent source references or unknown enum
    /// values), left out when 0.
    #[serde(skip_serializing_if = "is_zero_usize")]
    pub warnings: usize,
    /// True when the DWG library hit its diagnostic cap (999 entries) or
    /// notification limit and omitted further diagnostics, left out when
    /// false.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub diagnostics_truncated: bool,
    /// Why the DWG library could not read the file whole, when it could
    /// not: the read then went on past the records it could not read, as
    /// AutoCAD's RECOVER reads a damaged drawing, repairing nothing, and
    /// named each it passed under `not_decoded`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_past_damage: Option<String>,
    /// How many objects of each type the core read, such as `{"line": 12}`.
    pub objects: BTreeMap<String, usize>,
    /// The sheet's objects the core has no model for or that were skipped
    /// unread while reading past damage, by kind, with the handle of each:
    /// what no output draws.
    pub not_read: Vec<RecordsOfKind>,
    /// How many entries of each of the drawing's tables, and how many of
    /// its block definitions and layouts, the core has no model for, such
    /// as `{"layers": 2}`. Only those with any are named.
    pub tables_not_read: BTreeMap<String, usize>,
    /// The text mirror written; none when none was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

/// Reads the original `request.source` and, unless asked not to, writes
/// its text mirror.
pub fn read_original(request: &ReadRequest) -> Result<ReadReport, Error> {
    let source = &request.source;
    if !is_dwg(source) {
        return Err(Error::usage(
            "an original to read is a DWG, whose name ends .dwg; a sheet description is read by \
             query, render and plot",
        )
        .in_file(source));
    }
    if request.no_mirror && request.mirror.is_some() {
        return Err(Error::usage(
            "`mirror` names where to write the text mirror, and `no_mirror` writes none; give one \
             or the other",
        )
        .at_field("mirror"));
    }
    let mut source_file = File::open(source).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the DWG: {e}"),
        )
        .in_file(source)
    })?;
    #[cfg(any(unix, windows))]
    let source_id = file_id_of_file(&source_file);
    #[cfg(not(any(unix, windows)))]
    let source_id: Option<FileId> = None;
    let mut bytes = Vec::new();
    source_file.read_to_end(&mut bytes).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the DWG: {e}"),
        )
        .in_file(source)
    })?;
    drop(source_file);
    let drawing = dwg::read_bytes_past_damage(&bytes, source)?;
    let resolved_source = on_disk_path(source, source_id);
    let sheet = stem_of(&resolved_source);
    let mirror = if request.no_mirror {
        None
    } else {
        let path = request
            .mirror
            .clone()
            .unwrap_or_else(|| source.with_extension(MIRROR_EXTENSION));
        let target = refuse_to_replace(source, source_id, &path)?;
        let text = mirror_text(&resolved_source, &bytes, &sheet, &drawing);
        write_whole(source, source_id, &path, text.as_bytes(), target)?;
        Some(path)
    };
    let mut objects = BTreeMap::new();
    for object in &drawing.objects {
        *objects.entry(object.type_name().to_owned()).or_insert(0) += 1;
    }
    let tables_not_read = tables_not_read(&drawing);
    let message = message(source, &drawing, mirror.as_deref());
    Ok(ReadReport {
        source: source.display().to_string(),
        sheet,
        message,
        dwg_version: drawing.dwg_version,
        units: drawing.units,
        decoded: drawing.decode.decoded,
        not_decoded: drawing.decode.not_decoded,
        skipped: drawing.decode.skipped,
        recovered_errors: drawing.decode.recovered_errors,
        warnings: drawing.decode.warnings,
        diagnostics_truncated: drawing.decode.diagnostics_truncated,
        read_past_damage: drawing.decode.read_past_damage,
        objects,
        not_read: drawing.not_read,
        tables_not_read,
        mirror: mirror.map(|p| p.display().to_string()),
    })
}

/// A DWG loaded once for `query` or `check`, strictly (refusing damaged files)
/// and strictly read-only, with the same sheet name and object IDs as `read`
/// and `display-list`.
pub(crate) struct DwgLoaded {
    pub record: SheetRecord,
    /// The drawing's declared unit; none when the DWG is unitless or uses a
    /// unit the core does not know.
    pub units: Option<Unit>,
    /// How many sheet objects of the DWG the core has no model for yet.
    pub objects_not_read: usize,
    /// Those objects by kind, with the handle of each.
    pub not_read: Vec<RecordsOfKind>,
}

/// Reads the DWG at `source` once in strict mode and turns it into a
/// [`SheetRecord`] for `query` and `check`, using the same canonical sheet
/// name and collision-free handle IDs as `read` and `display-list`. Never
/// writes or modifies any file.
///
/// When `need_set_paper` is true and the DWG declares no layout paper, checks
/// its in-memory build mark for its set's declared paper without reopening
/// `source`.
pub(crate) fn read_dwg_record(source: &Path, need_set_paper: bool) -> Result<DwgLoaded, Error> {
    let mut source_file = File::open(source).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the DWG: {e}"),
        )
        .in_file(source)
    })?;
    #[cfg(any(unix, windows))]
    let source_id = file_id_of_file(&source_file);
    #[cfg(not(any(unix, windows)))]
    let source_id: Option<FileId> = None;
    let mut bytes = Vec::new();
    source_file.read_to_end(&mut bytes).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the DWG: {e}"),
        )
        .in_file(source)
    })?;
    drop(source_file);
    let drawing = dwg::read_bytes(&bytes, source)?;
    let resolved_source = on_disk_path(source, source_id);
    let sheet_name = stem_of(&resolved_source);
    let (resolved_ids, _) = resolve_all_ids(&sheet_name, &drawing);
    let units = drawing.units;
    let effective_units = units.unwrap_or(Unit::Mm);
    // Built DWGs may carry a build mark naming their set's template paper;
    // originals carry no build mark so `paper_of_dwg_bytes` inspects only the
    // bytes already in memory and compares against `source_id` without ever
    // reopening `source`.
    let built_paper = if need_set_paper && drawing.paper.is_none() {
        crate::set::paper_of_dwg_bytes(source, source_id, &bytes).and_then(|found| {
            found.paper.map(|paper| Paper {
                width: found.units.convert(paper.width, effective_units),
                height: found.units.convert(paper.height, effective_units),
                ..paper
            })
        })
    } else {
        None
    };
    let mut ids = Vec::with_capacity(resolved_ids.len());
    let mut carried = Vec::with_capacity(resolved_ids.len());
    let mut objects = Vec::with_capacity(resolved_ids.len());
    for (id, name, is_carried) in resolved_ids {
        ids.push(id.clone());
        carried.push(is_carried);
        objects.push(ObjectRecord {
            id,
            name,
            name_key: None,
            object: Written {
                field: String::new(),
                line: 0,
                column: 0,
            },
            id_key: None,
            in_row: false,
        });
    }
    let sheet = Sheet {
        units: effective_units,
        block_library: None,
        linetypes: drawing.linetypes,
        layers: drawing.layers,
        text_styles: drawing.text_styles,
        table_styles: drawing.table_styles,
        mleader_styles: drawing.mleader_styles,
        dimension_styles: drawing.dimension_styles,
        blocks: drawing.blocks,
        objects: drawing.objects,
        paper: drawing.paper.or(built_paper),
        ltscale: drawing.ltscale,
        layout: drawing.layout,
        other_layouts: drawing.other_layouts,
    };
    let record = SheetRecord {
        name: sheet_name,
        file: source.to_path_buf(),
        sheet,
        objects,
        rows_file: None,
        dwg: Some(DwgOrigin {
            ids,
            carried,
            handles: drawing.handles,
            block_handles: drawing.block_handles,
            layout_setups: drawing.layout_setups,
        }),
    };
    Ok(DwgLoaded {
        record,
        units,
        objects_not_read: drawing.objects_not_read,
        not_read: drawing.not_read,
    })
}

/// One object's resolved unique ID and optional name.
type ResolvedId = (String, Option<String>);

/// One object's resolved unique ID, optional name, and whether the ID came from
/// AgentDrafter's extended data rather than its handle.
type ResolvedEntry = (String, Option<String>, bool);

/// The ID of each object the core read from the DWG at `source`, in the
/// order of `drawing.objects`, with its name: the ID AgentDrafter's
/// extended data on it gives when unique and not colliding with another
/// object's handle ID, and otherwise the drawing's name joined to its
/// handle.
#[cfg(test)]
pub(crate) fn ids_of(source: &Path, drawing: &DrawingContents) -> Vec<ResolvedId> {
    ids_of_with_id(source, file_id_of_path(source), drawing)
}

/// Same as [`ids_of`], reusing `source_id` captured from an already-opened
/// handle so `source` is not reopened to identify it.
pub(crate) fn ids_of_with_id(
    source: &Path,
    source_id: Option<FileId>,
    drawing: &DrawingContents,
) -> Vec<ResolvedId> {
    let sheet = stem_of(&on_disk_path(source, source_id));
    resolve_all_ids(&sheet, drawing)
        .0
        .into_iter()
        .map(|(id, name, _)| (id, name))
        .collect()
}

/// Resolves unique IDs and names for both the sheet's objects and each
/// block definition's objects in `drawing`, preventing a carried ID from
/// colliding with another object's handle ID or duplicating an earlier ID
/// (case-insensitively).
fn resolve_all_ids(
    sheet: &str,
    drawing: &DrawingContents,
) -> (Vec<ResolvedEntry>, Vec<Vec<ResolvedEntry>>) {
    // Flatten every object (sheet first, then each block's objects) so
    // handle-ID reservations and uniqueness hold across the whole drawing.
    let mut flat_handles: Vec<&str> = Vec::with_capacity(
        drawing.handles.len() + drawing.block_handles.iter().map(Vec::len).sum::<usize>(),
    );
    let mut flat_carried: Vec<Option<&IdAndName>> = Vec::with_capacity(flat_handles.capacity());
    for (i, handle) in drawing.handles.iter().enumerate() {
        flat_handles.push(handle.as_str());
        flat_carried.push(drawing.ids_and_names.get(i));
    }
    for (bi, block_handles) in drawing.block_handles.iter().enumerate() {
        let block_ids = drawing.block_ids_and_names.get(bi);
        for (oi, handle) in block_handles.iter().enumerate() {
            flat_handles.push(handle.as_str());
            flat_carried.push(block_ids.and_then(|list| list.get(oi)));
        }
    }

    let handle_ids: Vec<String> = flat_handles.iter().map(|h| handle_id(sheet, h)).collect();
    let mut handle_owner: HashMap<String, usize> = HashMap::with_capacity(handle_ids.len());
    for (i, hid) in handle_ids.iter().enumerate() {
        handle_owner.entry(hid.to_lowercase()).or_insert(i);
    }

    let mut used: HashSet<String> = HashSet::with_capacity(handle_ids.len());
    let mut resolved: Vec<ResolvedEntry> = Vec::with_capacity(handle_ids.len());
    for (i, carried) in flat_carried.iter().enumerate() {
        let candidate = carried
            .and_then(|c| c.id.as_deref())
            .filter(|id| !id.is_empty());
        let accepted = candidate.and_then(|id| {
            let key = id.to_lowercase();
            let steals_other_handle = handle_owner.get(&key).is_some_and(|&owner| owner != i);
            (!used.contains(&key) && !steals_other_handle).then(|| id.to_owned())
        });
        let (id, is_carried) = match accepted {
            Some(id) => (id, true),
            None => {
                let base = &handle_ids[i];
                let id = if !used.contains(&base.to_lowercase()) {
                    base.clone()
                } else {
                    let mut n = 2usize;
                    loop {
                        let next = format!("{base}~{n}");
                        if !used.contains(&next.to_lowercase()) {
                            break next;
                        }
                        n += 1;
                    }
                };
                (id, false)
            }
        };
        used.insert(id.to_lowercase());
        resolved.push((id, carried.and_then(|c| c.name.clone()), is_carried));
    }

    let mut iter = resolved.into_iter();
    let sheet_ids: Vec<ResolvedEntry> = iter.by_ref().take(drawing.handles.len()).collect();
    let block_ids: Vec<Vec<ResolvedEntry>> = drawing
        .block_handles
        .iter()
        .map(|handles| iter.by_ref().take(handles.len()).collect())
        .collect();
    (sheet_ids, block_ids)
}

/// The ID of the object of handle `handle` on the original named `sheet`,
/// such as `P-101/handle:2F`.
pub(crate) fn handle_id(sheet: &str, handle: &str) -> String {
    format!("{sheet}{ID_SEPARATOR}{HANDLE_ID}{handle}")
}

/// The name an original's objects' IDs start with: its on-disk file's name
/// without its `.dwg`, resolved through the file system and its parent
/// directory's own entry when the file exists so the same file gives the
/// same IDs regardless of the case used to name its path (including on
/// case-insensitive volumes mounted on Linux, where `realpath` keeps the
/// caller's spelling).
pub(crate) fn sheet_name(source: &Path) -> String {
    stem_of(&on_disk_path(source, file_id_of_path(source)))
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The file identity of a regular, non-symlink file at `path`, without
/// following symlinks or Windows reparse points.
#[cfg(unix)]
fn regular_file_id_no_follow(path: &Path) -> Option<FileId> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    Some(crate::build::symlink_file_id(&meta))
}

#[cfg(windows)]
fn regular_file_id_no_follow(path: &Path) -> Option<FileId> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .access_mode(0)
        .custom_flags(crate::build::win_file::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let info = crate::build::win_file::file_info(&file)?;
    let bad = crate::build::win_file::FILE_ATTRIBUTE_DIRECTORY
        | crate::build::win_file::FILE_ATTRIBUTE_REPARSE_POINT;
    if (info.file_attributes & bad) != 0 {
        return None;
    }
    file_id_of_file(&file)
}

#[cfg(not(any(unix, windows)))]
fn regular_file_id_no_follow(_path: &Path) -> Option<FileId> {
    None
}

/// Resolves `source` to its canonical on-disk filename, scanning the parent
/// directory for regular non-symlink entries matching `target_id`, sorted
/// deterministically by filename before preferring:
/// 1. exact filename match,
/// 2. Unicode case-folded match (`to_lowercase` or `to_uppercase`),
/// 3. the lexicographically first same-inode entry.
fn on_disk_path(source: &Path, target_id: Option<FileId>) -> PathBuf {
    let resolved = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    let (Some(parent), Some(wanted_name)) = (resolved.parent(), resolved.file_name()) else {
        return resolved;
    };
    let Some(target_id) = target_id else {
        return resolved;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return resolved;
    };
    let mut matches: Vec<std::ffi::OsString> = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || file_type.is_dir() || !file_type.is_file() {
            continue;
        }
        if regular_file_id_no_follow(&entry.path()) == Some(target_id) {
            matches.push(entry.file_name());
        }
    }
    if matches.is_empty() {
        return resolved;
    }
    matches.sort_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
    if let Some(exact) = matches.iter().find(|name| name.as_os_str() == wanted_name) {
        return parent.join(exact);
    }
    let wanted_str = wanted_name.to_string_lossy();
    let wanted_lower = wanted_str.to_lowercase();
    let wanted_upper = wanted_str.to_uppercase();
    if let Some(folded) = matches.iter().find(|name| {
        let s = name.to_string_lossy();
        s.to_lowercase() == wanted_lower || s.to_uppercase() == wanted_upper
    }) {
        return parent.join(folded);
    }
    parent.join(&matches[0])
}

/// Whether `source` has a `.dwg` extension, in any case.
pub(crate) fn is_dwg(source: &Path) -> bool {
    source
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dwg"))
}

/// Whether `source` should be read as a DWG file rather than a drawing-set
/// folder: its name ends `.dwg` (any case) and it is not a directory.
pub(crate) fn is_dwg_file(source: &Path) -> bool {
    is_dwg(source) && !source.is_dir()
}

/// Whether the mirror path is brand new or an existing AgentDrafter mirror
/// to replace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MirrorTarget {
    CreateNew,
    ReplaceExisting,
}

/// Outcome of probing an existing candidate mirror file without following
/// symlinks or reparse points.
enum MirrorProbe {
    Prefix(Vec<u8>),
    #[cfg_attr(not(any(unix, windows)), allow(dead_code))]
    SameAsSource,
    InvalidOrChanged,
}

/// Refuses to write the mirror at `mirror` when it is the original itself,
/// a symbolic link, a folder, a special file, or a file other than a
/// mirror AgentDrafter wrote.
fn refuse_to_replace(
    source: &Path,
    source_id: Option<FileId>,
    mirror: &Path,
) -> Result<MirrorTarget, Error> {
    if mirror.file_name().is_none() {
        return Err(Error::usage("the mirror's path does not name a file").at_field("mirror"));
    }
    if is_dwg(mirror) {
        return Err(Error::usage(
            "a text mirror is YAML, not a DWG: choose a path for it that does not end .dwg",
        )
        .at_field("mirror"));
    }
    let meta = match fs::symlink_metadata(mirror) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(MirrorTarget::CreateNew),
        Err(e) => {
            return Err(Error::new(
                ErrorCode::WriteFailed,
                format!("could not inspect the mirror's path: {e}"),
            )
            .in_file(mirror));
        }
    };
    if meta.file_type().is_symlink() {
        return Err(Error::new(
            ErrorCode::NotAnOutput,
            "the mirror's path is a symbolic link; AgentDrafter never follows or replaces a \
             symbolic link as a text mirror, so choose another path",
        )
        .in_file(mirror));
    }
    if meta.is_dir() {
        return Err(Error::new(
            ErrorCode::NotAnOutput,
            "the mirror's path is a folder, not a text mirror AgentDrafter wrote; choose a file \
             path for the mirror",
        )
        .in_file(mirror));
    }
    if !meta.file_type().is_file() {
        return Err(Error::new(
            ErrorCode::NotAnOutput,
            "the mirror's path is not a regular file; choose another path for the mirror",
        )
        .in_file(mirror));
    }
    #[cfg(not(any(unix, windows)))]
    if is_same_file(source, mirror) {
        return Err(Error::usage(
            "the mirror's path is the original itself, which is never written; choose another \
             path for the mirror",
        )
        .in_file(mirror));
    }
    #[cfg(any(unix, windows))]
    let _ = source;
    let prefix = match read_mirror_prefix(mirror, &meta, source_id) {
        Ok(MirrorProbe::Prefix(prefix)) => prefix,
        Ok(MirrorProbe::SameAsSource) => {
            return Err(Error::usage(
                "the mirror's path is the original itself, which is never written; choose another \
                 path for the mirror",
            )
            .in_file(mirror));
        }
        Ok(MirrorProbe::InvalidOrChanged) => {
            return Err(Error::new(
                ErrorCode::NotAnOutput,
                "the mirror's path changed while inspecting it (or is a link); choose another \
                 path for the mirror",
            )
            .in_file(mirror));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(MirrorTarget::CreateNew),
        Err(_) => Vec::new(),
    };
    if prefix.starts_with(MIRROR_MARK.as_bytes()) {
        Ok(MirrorTarget::ReplaceExisting)
    } else {
        Err(Error::new(
            ErrorCode::NotAnOutput,
            "a file that is not a text mirror AgentDrafter wrote is at the mirror's path; it may \
             be someone's own file, which is never overwritten, so choose another path for the \
             mirror",
        )
        .in_file(mirror))
    }
}

#[cfg(unix)]
const O_NOFOLLOW: i32 = {
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        0x0100
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(target_arch = "aarch64", target_arch = "arm")
    ))]
    {
        0x8000
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        not(any(target_arch = "aarch64", target_arch = "arm"))
    ))]
    {
        0x20000
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "linux",
        target_os = "android"
    )))]
    {
        0
    }
};

/// Opens `mirror` without following a trailing symlink or reparse point,
/// verifies on the opened handle that it is a regular non-symlink file that
/// matches `expected` and is not `source_id`, and reads at most the first
/// `MIRROR_MARK.len()` bytes.
fn read_mirror_prefix(
    mirror: &Path,
    expected: &fs::Metadata,
    source_id: Option<FileId>,
) -> io::Result<MirrorProbe> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if O_NOFOLLOW != 0 {
            opts.custom_flags(O_NOFOLLOW);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(crate::build::win_file::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = opts.open(mirror)?;
    let opened = file.metadata()?;
    if !opened.file_type().is_file() {
        return Ok(MirrorProbe::InvalidOrChanged);
    }
    #[cfg(unix)]
    {
        let expected_id = crate::build::symlink_file_id(expected);
        let opened_id = file_id_of_file(&file);
        if source_id.is_some() && opened_id == source_id {
            return Ok(MirrorProbe::SameAsSource);
        }
        if opened_id != Some(expected_id) {
            return Ok(MirrorProbe::InvalidOrChanged);
        }
    }
    #[cfg(windows)]
    {
        let _ = expected;
        let Some(info) = crate::build::win_file::file_info(&file) else {
            return Ok(MirrorProbe::InvalidOrChanged);
        };
        let bad = crate::build::win_file::FILE_ATTRIBUTE_DIRECTORY
            | crate::build::win_file::FILE_ATTRIBUTE_REPARSE_POINT;
        if (info.file_attributes & bad) != 0 {
            return Ok(MirrorProbe::InvalidOrChanged);
        }
        let opened_id = file_id_of_file(&file);
        if source_id.is_some() && opened_id == source_id {
            return Ok(MirrorProbe::SameAsSource);
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (expected, source_id);
    }
    let mut prefix = Vec::with_capacity(MIRROR_MARK.len());
    file.take(MIRROR_MARK.len() as u64)
        .read_to_end(&mut prefix)?;
    Ok(MirrorProbe::Prefix(prefix))
}

/// Creates a brand-new temporary file beside `path` with an unpredictable
/// name using `create_new(true)` so an existing path or symlink is never
/// opened or followed.
fn create_temp_beside(path: &Path) -> io::Result<(PathBuf, File)> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    for _ in 0..64 {
        let count = WRITES.fetch_add(1, Ordering::Relaxed);
        let hi = random_u64(path, count);
        let lo = random_u64(path, count ^ 0x9e37_79b9_7f4a_7c15);
        let candidate = path.with_file_name(format!(".agentdrafter-{hi:016x}{lo:016x}.mirror.tmp"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary mirror file",
    ))
}

fn random_u64(path: &Path, salt: u64) -> u64 {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.write_u64(salt);
    hasher.write(path.as_os_str().as_encoded_bytes());
    hasher.finish()
}

/// Atomically moves `temporary` into place at `path` only when `path` does
/// not already exist, never overwriting a file that appeared meanwhile and
/// never streaming partially into `path`. Always cleans up `temporary`.
fn place_new(temporary: &Path, path: &Path) -> Result<(), Error> {
    let appeared = || {
        Error::new(
            ErrorCode::NotAnOutput,
            "a file appeared at the mirror's path while reading; it may be someone's own file, \
             which is never overwritten, so choose another path for the mirror",
        )
        .in_file(path)
    };
    let write_err = |e: io::Error| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not write the text mirror: {e}"),
        )
        .in_file(path)
    };

    #[cfg(windows)]
    {
        match rename_no_replace_windows(temporary, path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists || e.raw_os_error() == Some(183) => {
                let _ = fs::remove_file(temporary);
                Err(appeared())
            }
            Err(e) => {
                let _ = fs::remove_file(temporary);
                Err(write_err(e))
            }
        }
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        // Try atomic non-replacing rename first (`renameat2(RENAME_NOREPLACE)`
        // on Linux, `renamex_np(RENAME_EXCL)` on macOS): one syscall that works
        // on ext4/xfs/btrfs/tmpfs/APFS AND on VFAT/exFAT where `hard_link`
        // fails with `EPERM`. Fall back to `hard_link` + unlink only if the
        // filesystem or kernel does not support non-replacing rename.
        match rename_no_replace_unix(temporary, path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(temporary);
                Err(appeared())
            }
            Err(e) if is_rename_noreplace_unsupported(&e) => match fs::hard_link(temporary, path) {
                Ok(()) => {
                    let _ = fs::remove_file(temporary);
                    Ok(())
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(temporary);
                    Err(appeared())
                }
                Err(err) => {
                    let _ = fs::remove_file(temporary);
                    Err(write_err(err))
                }
            },
            Err(e) => {
                let _ = fs::remove_file(temporary);
                Err(write_err(e))
            }
        }
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        match fs::hard_link(temporary, path) {
            Ok(()) => {
                let _ = fs::remove_file(temporary);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(temporary);
                Err(appeared())
            }
            Err(e) => {
                let _ = fs::remove_file(temporary);
                Err(write_err(e))
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn is_rename_noreplace_unsupported(err: &io::Error) -> bool {
    if matches!(
        err.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
    ) {
        return true;
    }
    #[cfg(target_os = "linux")]
    let raw_unsupported = matches!(err.raw_os_error(), Some(22 | 38 | 95)); // EINVAL, ENOSYS, EOPNOTSUPP
    #[cfg(target_os = "macos")]
    let raw_unsupported = matches!(err.raw_os_error(), Some(22 | 45 | 78)); // EINVAL, ENOTSUP, ENOSYS
    raw_unsupported
}

#[cfg(windows)]
fn rename_no_replace_windows(temporary: &Path, path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let src: Vec<u16> = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dst: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `src` and `dst` are valid NUL-terminated UTF-16 paths; flags=0
    // requests an atomic rename without `MOVEFILE_REPLACE_EXISTING`.
    let ok = unsafe { MoveFileExW(src.as_ptr(), dst.as_ptr(), 0) };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn rename_no_replace_unix(temporary: &Path, path: &Path) -> io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint, CString};
    use std::os::unix::ffi::OsStrExt;

    const AT_FDCWD: c_int = -100;
    const RENAME_NOREPLACE: c_uint = 1;

    extern "C" {
        fn renameat2(
            olddirfd: c_int,
            oldpath: *const c_char,
            newdirfd: c_int,
            newpath: *const c_char,
            flags: c_uint,
        ) -> c_int;
    }

    let src = CString::new(temporary.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a nul byte"))?;
    let dst = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a nul byte"))?;
    // SAFETY: `src` and `dst` are valid NUL-terminated C strings.
    let rc = unsafe {
        renameat2(
            AT_FDCWD,
            src.as_ptr(),
            AT_FDCWD,
            dst.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_no_replace_unix(temporary: &Path, path: &Path) -> io::Result<()> {
    use std::ffi::{c_char, c_int, c_uint, CString};
    use std::os::unix::ffi::OsStrExt;

    const RENAME_EXCL: c_uint = 0x0000_0004;

    extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }

    let src = CString::new(temporary.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a nul byte"))?;
    let dst = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a nul byte"))?;
    // SAFETY: `src` and `dst` are valid NUL-terminated C strings.
    let rc = unsafe { renamex_np(src.as_ptr(), dst.as_ptr(), RENAME_EXCL) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Writes `bytes` to a brand-new unpredictable temporary file beside
/// `path`, then places it at `path` whole: without overwriting anything
/// that appeared meanwhile when creating a new mirror (or if an existing
/// mirror disappeared before replacement), and only after re-verifying a
/// regular non-symlink AgentDrafter mirror when replacing one.
fn write_whole(
    source: &Path,
    source_id: Option<FileId>,
    path: &Path,
    bytes: &[u8],
    target: MirrorTarget,
) -> Result<(), Error> {
    let write_err = |e: io::Error| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not write the text mirror: {e}"),
        )
        .in_file(path)
    };
    let (temporary, mut file) = create_temp_beside(path).map_err(write_err)?;
    let wrote = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = wrote {
        let _ = fs::remove_file(&temporary);
        return Err(write_err(e));
    }
    match target {
        MirrorTarget::CreateNew => place_new(&temporary, path),
        MirrorTarget::ReplaceExisting => match refuse_to_replace(source, source_id, path) {
            Err(err) => {
                let _ = fs::remove_file(&temporary);
                Err(err)
            }
            Ok(MirrorTarget::CreateNew) => place_new(&temporary, path),
            Ok(MirrorTarget::ReplaceExisting) => fs::rename(&temporary, path).map_err(|e| {
                let _ = fs::remove_file(&temporary);
                write_err(e)
            }),
        },
    }
}

/// The entries of each table, and the blocks and layouts, the core has no
/// model for, by the name the reply gives each.
fn tables_not_read(drawing: &DrawingContents) -> BTreeMap<String, usize> {
    [
        ("linetypes", drawing.linetypes_not_read),
        ("layers", drawing.layers_not_read),
        ("text_styles", drawing.text_styles_not_read),
        ("table_styles", drawing.table_styles_not_read),
        ("mleader_styles", drawing.mleader_styles_not_read),
        ("dimension_styles", drawing.dimension_styles_not_read),
        ("blocks", drawing.blocks_not_read),
        ("layouts", drawing.layouts_not_read),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .map(|(name, n)| (name.to_owned(), n))
    .collect()
}

/// What a read found, in words.
fn message(source: &Path, drawing: &DrawingContents, mirror: Option<&Path>) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let decode = &drawing.decode;
    let decoded = if decode.decoded {
        "the DWG library decoded every record".to_owned()
    } else {
        let mut parts: Vec<String> = Vec::new();
        if decode.read_past_damage.is_some() {
            parts.push("could not read the file whole, so read past the damage".to_owned());
        }
        if !decode.not_decoded.is_empty() {
            parts.push(format!(
                "could not decode {}",
                kinds_in_words(&decode.not_decoded)
            ));
        }
        if decode.skipped > 0 {
            parts.push(format!("skipped {} it could not read", decode.skipped));
        }
        if decode.recovered_errors > 0 {
            parts.push(format!(
                "recovered from {} error{}",
                decode.recovered_errors,
                plural(decode.recovered_errors)
            ));
        }
        if decode.diagnostics_truncated {
            parts.push("hit its diagnostic limit so some diagnostics may be omitted".to_owned());
        }
        if parts.is_empty() {
            parts.push("did not finish decoding the file whole".to_owned());
        }
        format!("the DWG library {}", parts.join(", "))
    };
    let warnings = if decode.warnings > 0 {
        format!(
            " ({} reader warning{})",
            decode.warnings,
            plural(decode.warnings)
        )
    } else {
        String::new()
    };
    let read = drawing.objects.len();
    let not_read = if drawing.not_read.is_empty() {
        String::new()
    } else {
        format!(
            "; {} the core has no model for or skipped unread, which no output draws: {}",
            drawing.objects_not_read,
            kinds_in_words(&drawing.not_read)
        )
    };
    let written = match mirror {
        Some(path) => format!("; wrote its text mirror to {}", path.display()),
        None => String::new(),
    };
    format!(
        "read {name} ({}): {decoded}{warnings}; {read} object{} read{not_read}{written}",
        drawing.dwg_version,
        plural(read)
    )
}

/// `3 ACAD_PROXY_ENTITY, 1 MLINE`.
pub(crate) fn kinds_in_words(kinds: &[RecordsOfKind]) -> String {
    kinds
        .iter()
        .map(|k| format!("{} {}", k.count, k.kind))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Whether `c` is a YAML 1.2 §5.1 printable non-line-break character
/// (`0x20..=0x7E`, `0xA0..=0xD7FF`, `0xE000..=0xFFFD`, `0x10000..=0x10FFFF`),
/// additionally excluding `U+FEFF` (BOM, forbidden by YAML 1.2 §6.6 comments)
/// and Unicode line/paragraph separators (`U+2028`, `U+2029`) that YAML 1.1
/// treats as line breaks.
fn is_yaml12_safe_single_line(c: char) -> bool {
    !matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}')
        && matches!(
            c as u32,
            0x20..=0x7E | 0xA0..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
        )
}

/// Escapes any character forbidden in YAML 1.2 comments or line breaks so a
/// filename placed in a `#` comment line stays valid single-line YAML 1.2.
fn comment_safe(text: &str) -> String {
    if text.chars().all(is_yaml12_safe_single_line) {
        text.to_owned()
    } else {
        quoted(text)
    }
}

/// The text mirror of the original at `source`, whose bytes are `bytes`,
/// read as `drawing`: YAML, a key to a line, each entry of a table, each
/// object of a block and each object of the sheet on a line of its own,
/// in the order the file holds them, so that two mirrors compare line by
/// line. The same file always gives the same mirror.
fn mirror_text(source: &Path, bytes: &[u8], sheet: &str, drawing: &DrawingContents) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = String::new();
    out.push_str(&format!(
        "{MIRROR_MARK}{}, for reading and comparing. The DWG is the record: this file is \
         never read back, and editing it changes nothing.\n",
        comment_safe(&name)
    ));
    let key = |out: &mut String, key: &str, value: Value| {
        out.push_str(&pair(key, &value));
        out.push('\n');
    };
    key(&mut out, "mirror_of", json!(name));
    key(&mut out, "sha256", json!(fingerprint(bytes)));
    key(&mut out, "dwg_version", json!(drawing.dwg_version));
    key(&mut out, "units", json!(drawing.units.map(Unit::symbol)));
    key(&mut out, "sheet", json!(sheet));
    let decode = &drawing.decode;
    key(&mut out, "decoded", json!(decode.decoded));
    out.push_str(&format!("not_decoded: {}\n", records(&decode.not_decoded)));
    key(&mut out, "skipped", json!(decode.skipped));
    key(&mut out, "recovered_errors", json!(decode.recovered_errors));
    if decode.warnings > 0 {
        key(&mut out, "warnings", json!(decode.warnings));
    }
    if decode.diagnostics_truncated {
        key(&mut out, "diagnostics_truncated", json!(true));
    }
    if let Some(why) = &decode.read_past_damage {
        key(&mut out, "read_past_damage", json!(why));
    }
    out.push_str(&format!("not_read: {}\n", records(&drawing.not_read)));
    key(
        &mut out,
        "tables_not_read",
        to_value(&tables_not_read(drawing)),
    );
    key(&mut out, "ltscale", json!(drawing.ltscale));
    if let Some(opens_on) = &drawing.opens_on {
        key(&mut out, "opens_on", json!(opens_on));
    }
    list(&mut out, "linetypes", &drawing.linetypes);
    list(&mut out, "layers", &drawing.layers);
    list(&mut out, "text_styles", &drawing.text_styles);
    list(&mut out, "table_styles", &drawing.table_styles);
    list(&mut out, "mleader_styles", &drawing.mleader_styles);
    list(&mut out, "dimension_styles", &drawing.dimension_styles);
    let layouts: Vec<&Layout> = drawing
        .layout
        .iter()
        .chain(&drawing.other_layouts)
        .collect();
    let layout_entries: Vec<Value> = layouts
        .iter()
        .map(|l| {
            let mut entry = Map::new();
            entry.insert("name".into(), json!(l.name));
            if let Some(paper) = &l.paper {
                entry.insert("paper".into(), to_value(paper));
            }
            Value::Object(entry)
        })
        .collect();
    list(&mut out, "layouts", &layout_entries);
    let (ids, block_ids) = resolve_all_ids(sheet, drawing);
    if drawing.blocks.is_empty() {
        out.push_str("blocks: []\n");
    } else {
        out.push_str("blocks:\n");
        for (bi, block) in drawing.blocks.iter().enumerate() {
            out.push_str(&format!("  - name: {}\n", flow(&json!(block.name))));
            out.push_str(&format!(
                "    base_point: {}\n",
                flow(&to_value(&block.base_point))
            ));
            if block.objects.is_empty() {
                out.push_str("    objects: []\n");
            } else {
                out.push_str("    objects:\n");
                let handles = drawing.block_handles.get(bi);
                let object_ids = block_ids.get(bi);
                for (oi, object) in block.objects.iter().enumerate() {
                    let mut first = Vec::new();
                    if let Some((id, name, _)) = object_ids.and_then(|list| list.get(oi)) {
                        first.push(("id", json!(id)));
                        if let Some(name) = name {
                            first.push(("name", json!(name)));
                        }
                    }
                    if let Some(handle) = handles.and_then(|list| list.get(oi)) {
                        first.push(("handle", json!(handle)));
                    }
                    out.push_str(&format!("      - {}\n", object_entry(object, first)));
                }
            }
        }
    }
    if drawing.objects.is_empty() {
        out.push_str("objects: []\n");
    } else {
        out.push_str("objects:\n");
        for (i, object) in drawing.objects.iter().enumerate() {
            let (id, name, _) = &ids[i];
            let mut first = vec![("id", json!(id))];
            if let Some(name) = name {
                first.push(("name", json!(name)));
            }
            first.push(("handle", json!(drawing.handles[i])));
            if !layouts.is_empty() {
                let space = layouts
                    .iter()
                    .find(|l| l.paper_space.get(i).copied().unwrap_or(false))
                    .map_or("model", |l| l.name.as_str());
                first.push(("space", json!(space)));
            }
            out.push_str(&format!("  - {}\n", object_entry(object, first)));
        }
    }
    out
}

/// `key: []`, or `key:` and each of `entries` on a line of its own.
fn list<T: Serialize>(out: &mut String, key: &str, entries: &[T]) {
    if entries.is_empty() {
        out.push_str(&format!("{key}: []\n"));
        return;
    }
    out.push_str(&format!("{key}:\n"));
    for entry in entries {
        out.push_str(&format!("  - {}\n", flow(&to_value(entry))));
    }
}

/// Records by kind, as the mirror writes them.
fn records(kinds: &[RecordsOfKind]) -> String {
    let inner: Vec<String> = kinds
        .iter()
        .map(|k| {
            format!(
                "{{{}, {}, {}}}",
                pair("kind", &json!(k.kind)),
                pair("count", &json!(k.count)),
                pair("handles", &json!(k.handles))
            )
        })
        .collect();
    format!("[{}]", inner.join(", "))
}

fn to_value<T: Serialize + ?Sized>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// An object as the mirror writes it, in braces on one line: the keys of
/// `first`, in order, then its `type`, then its own keys.
fn object_entry<'a>(object: &Object, first: impl IntoIterator<Item = (&'a str, Value)>) -> String {
    let mut keys: Vec<(String, Value)> =
        first.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    let mut own = match to_value(object) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Some(kind) = own.remove("type") {
        keys.push(("type".to_owned(), kind));
    }
    keys.extend(own);
    let inner: Vec<String> = keys.iter().map(|(k, v)| pair(k, v)).collect();
    format!("{{{}}}", inner.join(", "))
}

/// `key: value`, a handle or a list of handles, and a fingerprint, always in
/// double quotes, so that one such as `1E3` is never read back as a
/// number.
fn pair(key: &str, value: &Value) -> String {
    let value = match (key, value) {
        ("handle" | "sha256", Value::String(h)) => quoted(h),
        ("handles", Value::Array(hs)) => {
            let inner: Vec<String> = hs
                .iter()
                .map(|h| h.as_str().map_or_else(|| flow(h), quoted))
                .collect();
            format!("[{}]", inner.join(", "))
        }
        _ => flow(value),
    };
    format!("{}: {value}", scalar(key))
}

/// `text` in double quotes, escaped as JSON escapes it plus every character
/// outside YAML 1.2 §5.1 printable non-line-break characters (C0, DEL
/// `U+007F`, C1 `U+0080..=U+009F`, `U+2028`, `U+2029`, `U+FEFF`, `U+FFFE`,
/// `U+FFFF`), so YAML 1.1 and 1.2 always read it back as `text` on one line.
fn quoted(text: &str) -> String {
    let raw = serde_json::to_string(text).unwrap_or_default();
    if raw.chars().all(is_yaml12_safe_single_line) {
        return raw;
    }
    let mut out = String::with_capacity(raw.len() + 8);
    for c in raw.chars() {
        if is_yaml12_safe_single_line(c) {
            out.push(c);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "\\u{:04X}", c as u32);
        }
    }
    out
}

/// `value` in YAML's flow style on one line: mappings in braces, lists in
/// brackets, and a string plain where YAML reads it back as that string,
/// otherwise in double quotes.
fn flow(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => number_text(f),
            _ => n.to_string(),
        },
        Value::String(s) => scalar(s),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(flow).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Object(map) => {
            // A named entry's name first, then its other keys in order.
            let named = map.get("name").map(|name| ("name", name));
            let inner: Vec<String> = named
                .into_iter()
                .chain(
                    map.iter()
                        .filter(|(k, _)| *k != "name")
                        .map(|(k, v)| (k.as_str(), v)),
                )
                .map(|(k, v)| pair(k, v))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

/// A string as the mirror writes it: plain when it is a word YAML reads
/// back as that string, such as `E-WIRE` or `P-101/handle:2F`, and
/// otherwise in double quotes, escaped as JSON escapes it, which YAML reads
/// the same.
fn scalar(text: &str) -> String {
    let word = text
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:".contains(c))
        && !text.ends_with(':')
        && !text.contains(": ")
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
        );
    if word {
        text.to_owned()
    } else {
        quoted(text)
    }
}

#[cfg(test)]
#[path = "original_tests.rs"]
mod tests;
