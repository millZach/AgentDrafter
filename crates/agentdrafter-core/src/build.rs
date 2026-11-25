//! Build: producing a DWG from a sheet description.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::convert::{ConversionLoss, DwgVersion};
use crate::description::{read_sheet_record_noting, SheetRecord};
use crate::dwg::{self, BuildMark};
use crate::error::{Error, ErrorCode};
use crate::settings::Settings;
use crate::text::fonts::FontSubstitution;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// The CAD drawing file format a build writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingFormat {
    /// DWG in the 2018 format (`AC1032`) — the default.
    #[default]
    Dwg,
    /// ASCII DXF in the 2018 format (`AC1032`).
    Dxf,
    /// Binary DXF in the 2018 format (`AC1032`).
    DxfBinary,
}

impl DrawingFormat {
    /// Every format a build can write, in order.
    pub const ALL: [DrawingFormat; 3] = [
        DrawingFormat::Dwg,
        DrawingFormat::Dxf,
        DrawingFormat::DxfBinary,
    ];

    /// The format's name in JSON, YAML and the command line.
    pub const fn name(self) -> &'static str {
        match self {
            DrawingFormat::Dwg => "dwg",
            DrawingFormat::Dxf => "dxf",
            DrawingFormat::DxfBinary => "dxf_binary",
        }
    }

    /// Parses a format name (`dwg`, `dxf`, `dxf_binary`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "dwg" => Some(DrawingFormat::Dwg),
            "dxf" => Some(DrawingFormat::Dxf),
            "dxf_binary" => Some(DrawingFormat::DxfBinary),
            _ => None,
        }
    }

    /// The file extension without its leading dot: `dwg` or `dxf`.
    pub const fn extension(self) -> &'static str {
        match self {
            DrawingFormat::Dwg => "dwg",
            DrawingFormat::Dxf | DrawingFormat::DxfBinary => "dxf",
        }
    }

    /// Whether this format is DWG.
    pub const fn is_dwg(&self) -> bool {
        matches!(self, DrawingFormat::Dwg)
    }

    /// Whether this format is DXF (ASCII or binary).
    pub const fn is_dxf(self) -> bool {
        matches!(self, DrawingFormat::Dxf | DrawingFormat::DxfBinary)
    }

    /// Whether this format is binary DXF.
    pub const fn is_binary_dxf(self) -> bool {
        matches!(self, DrawingFormat::DxfBinary)
    }
}

/// A request to build one sheet description into a DWG or DXF.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRequest {
    /// The sheet description to build.
    pub description: PathBuf,
    /// Where to write the drawing. Without it, the drawing goes beside the
    /// sheet description with the same name and `.dwg` (or `.dxf` when built
    /// as DXF). An output path ending `.dxf` builds ASCII DXF unless another
    /// format is asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    /// Replace the file at the output path, which is allowed only when it is
    /// a drawing AgentDrafter built. Without it, an existing file is never
    /// touched.
    #[serde(default)]
    pub overwrite: bool,
}

/// What a build wrote.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BuildReport {
    /// The sheet description that was built, or for a sheet of a drawing
    /// set, its template.
    pub description: String,
    /// The drawing that was written (a `.dwg` or `.dxf` path; when a DXF is
    /// written, `dxf` also names it).
    pub dwg: String,
    /// The DXF that was written, when the build wrote a DXF (`dxf` or
    /// `dxf_binary`). Left out when the build wrote a DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dxf: Option<String>,
    /// The drawing format written (`dwg`, `dxf`, or `dxf_binary`), left out
    /// when `dwg` (the default).
    #[serde(skip_serializing_if = "DrawingFormat::is_dwg")]
    pub format: DrawingFormat,
    /// The drawing version it was written in: `AC1032`, the 2018 format.
    pub dwg_version: String,
    /// The sheet's declared unit, which is the drawing's unit.
    pub units: Unit,
    /// The block library the sheet draws on, where it was read from, if the
    /// sheet names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_library: Option<String>,
    /// The blocks the DWG defines, as the block library defines them: the
    /// ones the sheet inserts.
    pub blocks: Vec<String>,
    /// How many objects of each type the DWG holds in model space, such as
    /// `{"line": 1}`.
    pub objects: BTreeMap<String, usize>,
    /// The size of the DWG in bytes.
    pub bytes: usize,
    /// The text styles whose fonts are not in the font folders, and the
    /// look-alike each is measured with. The DWG names the styles' own
    /// fonts all the same. Empty when every font was found.
    pub font_substitutions: Vec<FontSubstitution>,
    /// What reading a sheet description built on its own noticed that did
    /// not stop it, such as an attribute tag written in lowercase and taken
    /// in capitals. Left out when there is nothing to say; a sheet of a
    /// drawing set gives its notices beside its result.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notices: Vec<crate::set::Notice>,
    /// Version-conversion losses reported when building directly into a DWG
    /// version older than 2018 (`AC1032`). Left out when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conversion_losses: Vec<ConversionLoss>,
}

/// Builds a sheet description into a DWG (or ASCII DXF when `output` ends
/// `.dxf`) in the 2018 format, reading fonts from the font folders the
/// environment names ([`Settings::from_env`]).
///
/// The drawing is written whole or not at all: it is written beside its final
/// path and moved into place once complete.
pub fn build(request: &BuildRequest) -> Result<BuildReport, Error> {
    build_with(request, &Settings::from_env())
}

/// Builds a sheet description into a DWG (or ASCII DXF when `output` ends
/// `.dxf`) in the 2018 format, reading fonts from the font folders `settings`
/// names. A block library the sheet description names is read from beside it.
pub fn build_with(request: &BuildRequest, settings: &Settings) -> Result<BuildReport, Error> {
    build_with_format(request, None, settings)
}

/// Builds a sheet description into `format` (`dwg`, ASCII `dxf`, or
/// `dxf_binary`), or infers ASCII `dxf` when `format` is `None` and
/// `request.output` ends `.dxf` (otherwise `dwg`).
pub fn build_with_format(
    request: &BuildRequest,
    format: Option<DrawingFormat>,
    settings: &Settings,
) -> Result<BuildReport, Error> {
    build_with_options(request, format, None, settings)
}

/// Builds a sheet description into `format` (`dwg`, ASCII `dxf`, or
/// `dxf_binary`) and `dwg_version` (defaults to `AC1032` / 2018 when `None`).
pub fn build_with_options(
    request: &BuildRequest,
    format: Option<DrawingFormat>,
    dwg_version: Option<DwgVersion>,
    settings: &Settings,
) -> Result<BuildReport, Error> {
    let description = request.description.as_path();
    let effective = resolve_build_format(format, request.output.as_deref())?;
    let version = dwg_version.unwrap_or_default();
    let mut notices = Vec::new();
    let record = read_sheet_record_noting(description, &mut notices)?;
    let output = request
        .output
        .clone()
        .unwrap_or_else(|| description.with_extension(effective.extension()));
    let alone = BuildMark {
        set: None,
        rows: None,
    };
    let mut report = build_sheet_with_version(
        &record,
        description,
        &output,
        &alone,
        request.overwrite,
        effective,
        version,
        settings,
    )?;
    report.notices = notices;
    Ok(report)
}

fn resolve_build_format(
    format: Option<DrawingFormat>,
    output: Option<&Path>,
) -> Result<DrawingFormat, Error> {
    let ext = output
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match (format, ext.as_deref()) {
        (None, Some("dxf")) => Ok(DrawingFormat::Dxf),
        (None, _) => Ok(DrawingFormat::Dwg),
        (Some(DrawingFormat::Dwg), Some("dxf")) => Err(Error::usage(
            "the output path ends `.dxf` while `format` is `dwg`; choose a `.dwg` path or ask \
             for `dxf` / `dxf_binary`",
        )
        .at_field("format")),
        (Some(f @ (DrawingFormat::Dxf | DrawingFormat::DxfBinary)), Some("dwg")) => {
            Err(Error::usage(format!(
                "the output path ends `.dwg` while `format` is `{}`; choose a `.dxf` path or ask \
                 for `dwg`",
                f.name()
            ))
            .at_field("format"))
        }
        (Some(f), _) => Ok(f),
    }
}

/// Builds the sheet `record` holds, read from the sheet description or
/// template at `description`, into a DWG or DXF at `output`, written whole or
/// not at all, each object carrying the ID and name its record gives it, and
/// carrying the build mark `mark`: naming its drawing set, for a sheet of
/// a set, and no set for a sheet built on its own. An existing file there
/// is replaced only when `overwrite` is asked for and it carries
/// AgentDrafter's build mark.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_sheet_with_version(
    record: &SheetRecord,
    description: &Path,
    output: &Path,
    mark: &BuildMark,
    overwrite: bool,
    format: DrawingFormat,
    version: DwgVersion,
    settings: &Settings,
) -> Result<BuildReport, Error> {
    let prepared = prepare_sheet_with_version(
        record,
        description,
        output,
        mark,
        overwrite,
        format,
        version,
        settings,
    )?;
    write_whole(output, &prepared.bytes)?;
    Ok(prepared.report)
}

/// A sheet built in memory, ready to write: its drawing's bytes and what the
/// build will report once they are written.
pub(crate) struct PreparedSheet {
    pub report: BuildReport,
    pub bytes: Vec<u8>,
}

/// Builds the sheet `record` holds into the bytes of a DWG or DXF for
/// `output`, as [`build_sheet_with_version`] does, and refuses as it does, but
/// writes nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_sheet_with_version(
    record: &SheetRecord,
    description: &Path,
    output: &Path,
    mark: &BuildMark,
    overwrite: bool,
    format: DrawingFormat,
    version: DwgVersion,
    settings: &Settings,
) -> Result<PreparedSheet, Error> {
    if fs::symlink_metadata(output).is_ok() {
        if is_same_file(description, output) {
            return Err(Error::usage(format!(
                "the output path is the sheet description itself; choose another path for the {}",
                if format.is_dxf() { "DXF" } else { "DWG" }
            ))
            .in_file(output));
        }
        if !overwrite {
            return Err(Error::new(
                ErrorCode::OutputExists,
                "a file is already at the output path; ask to overwrite it to replace a previous build",
            )
            .in_file(output));
        }
        refuse_unless_built(output)?;
    }

    let has_shapes = record
        .sheet
        .objects
        .iter()
        .chain(record.sheet.blocks.iter().flat_map(|b| b.objects.iter()))
        .any(|o| matches!(o, crate::model::Object::Shape(_)));
    let resolved_record;
    let effective_record = if has_shapes && record.sheet.resolved_shapes.is_empty() {
        let mut cloned = record.clone();
        if let Some(dir) = description.parent() {
            crate::text::fonts::resolve_sheet_shapes(&mut cloned.sheet, settings, &[dir]);
        } else {
            crate::text::fonts::resolve_sheet_shapes(&mut cloned.sheet, settings, &[]);
        }
        resolved_record = cloned;
        &resolved_record
    } else {
        record
    };
    let sheet = &effective_record.sheet;
    let fonts = SheetFonts::find(sheet, settings);
    let (bytes, conversion_losses) = dwg::write_new_sheet_in_version(
        effective_record,
        &|text| fonts.start(text),
        &|text| crate::draw::mtext_box(text, &fonts),
        mark,
        format,
        version,
    )
    .map_err(|e| e.in_file(output))?;

    let mut objects = BTreeMap::new();
    for object in &sheet.objects {
        *objects.entry(object.type_name().to_owned()).or_insert(0) += 1;
    }
    let block_library = sheet.block_library.as_ref().map(|written| {
        description
            .parent()
            .unwrap_or(Path::new(""))
            .join(written)
            .display()
            .to_string()
    });
    let written_path = output.display().to_string();
    let report = BuildReport {
        description: description.display().to_string(),
        dwg: written_path.clone(),
        dxf: format.is_dxf().then_some(written_path),
        format,
        dwg_version: version.code().to_owned(),
        units: sheet.units,
        block_library,
        blocks: sheet.blocks.iter().map(|b| b.name.clone()).collect(),
        objects,
        bytes: bytes.len(),
        font_substitutions: fonts.substitutions(),
        notices: Vec::new(),
        conversion_losses,
    };
    Ok(PreparedSheet { report, bytes })
}

/// Refuses to replace the file at `output` unless it is a DWG or DXF
/// AgentDrafter built, as its build mark shows. Anything else may be an
/// original, and an original is never overwritten.
fn refuse_unless_built(output: &Path) -> Result<(), Error> {
    let kind = if dwg::is_dxf_path(output) {
        "DXF"
    } else {
        "DWG"
    };
    let why = match dwg::carries_build_mark(output) {
        Ok(true) => return Ok(()),
        Ok(false) => format!(
            "the {kind} at the output path was not built by AgentDrafter: it does not carry \
             AgentDrafter's build mark"
        ),
        Err(e) => format!(
            "the file at the output path cannot be read as a {kind}, so nothing shows AgentDrafter \
             built it ({})",
            e.message
        ),
    };
    Err(Error::new(
        ErrorCode::NotABuild,
        format!(
            "{why}; it may be an original, and an original is never overwritten, so choose \
             another output path"
        ),
    )
    .in_file(output))
}

/// Platform file identity: `(dev, ino)` on Unix, `(volume, index_high,
/// index_low)` via `GetFileInformationByHandle` on Windows.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileId {
    dev: u64,
    ino: u64,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileId {
    volume: u32,
    high: u32,
    low: u32,
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileId;

#[cfg(windows)]
pub(crate) mod win_file {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

    pub(crate) const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    pub(crate) const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
    pub(crate) const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(crate) struct ByHandleFileInformation {
        pub file_attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        pub volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        pub file_index_high: u32,
        pub file_index_low: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(file: *mut c_void, info: *mut ByHandleFileInformation)
            -> i32;
    }

    pub(crate) fn file_info(file: &File) -> Option<ByHandleFileInformation> {
        let mut info = ByHandleFileInformation::default();
        // SAFETY: `file.as_raw_handle()` is a valid open Windows handle for the
        // duration of the call, and `info` points to an initialized struct.
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
        (ok != 0).then_some(info)
    }
}

/// The identity of an already-opened file handle.
#[cfg(unix)]
pub(crate) fn file_id_of_file(file: &File) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata().ok()?;
    Some(FileId {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

#[cfg(windows)]
pub(crate) fn file_id_of_file(file: &File) -> Option<FileId> {
    let info = win_file::file_info(file)?;
    Some(FileId {
        volume: info.volume_serial_number,
        high: info.file_index_high,
        low: info.file_index_low,
    })
}

/// The identity of the file at `path` (following links).
#[cfg(unix)]
pub(crate) fn file_id_of_path(path: &Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path).ok()?;
    Some(FileId {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

#[cfg(windows)]
pub(crate) fn file_id_of_path(path: &Path) -> Option<FileId> {
    let file = File::open(path).ok()?;
    file_id_of_file(&file)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn file_id_of_path(_path: &Path) -> Option<FileId> {
    None
}

/// Whether `candidate` names the same file as `target`, using `target_id`
/// captured from an already-opened handle of `target` when available so
/// `target` is never reopened.
pub(crate) fn is_same_file_id(candidate: &Path, target: &Path, target_id: Option<FileId>) -> bool {
    #[cfg(any(unix, windows))]
    if let Some(target_id) = target_id {
        return file_id_of_path(candidate) == Some(target_id);
    }
    let _ = target_id;
    is_same_file(candidate, target)
}

/// The identity of the entry at `path` without following a trailing link.
#[cfg(unix)]
pub(crate) fn symlink_file_id(meta: &fs::Metadata) -> FileId {
    use std::os::unix::fs::MetadataExt;
    FileId {
        dev: meta.dev(),
        ino: meta.ino(),
    }
}

/// Whether `a` and `b` name one file that is there, as the file system
/// sees it: through links (including hard links), `..`, and case where the
/// file system ignores case. On Unix `(dev, ino)` says; on Windows
/// `GetFileInformationByHandle` says for files (with `canonicalize` fallback
/// for directories); elsewhere `canonicalize` says.
pub(crate) fn is_same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        match (file_id_of_path(a), file_id_of_path(b)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
    #[cfg(windows)]
    {
        match (file_id_of_path(a), file_id_of_path(b)) {
            (Some(a), Some(b)) => a == b,
            _ => match (fs::canonicalize(a), fs::canonicalize(b)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            },
        }
    }
    #[cfg(not(any(unix, windows)))]
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Writes `bytes` to a temporary file beside `path`, then moves it onto
/// `path`, so a reader never sees a half-written DWG.
///
/// The temporary file's name is short and does not hold the DWG's own, so
/// a DWG whose name is as long as the system allows can still be written.
/// It holds the process ID and a count, so no two writes share one.
pub(crate) fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let temporary = write_beside(path, bytes, "the DWG")?;
    fs::rename(&temporary, path).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not write the DWG: {e}"),
        )
        .in_file(path)
    })
}

/// Writes `bytes` to a new temporary file beside `path` and gives its path,
/// for the caller to move onto `path` once everything it writes together is
/// written. `what` names the file in the error, such as "the DWG".
pub(crate) fn write_beside(path: &Path, bytes: &[u8], what: &str) -> Result<PathBuf, Error> {
    if path.file_name().is_none() {
        return Err(Error::usage("the output path does not name a file").in_file(path));
    }
    let temporary = beside(path, "tmp");
    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()
    });
    match written {
        Ok(()) => Ok(temporary),
        Err(e) => {
            let _ = fs::remove_file(&temporary);
            Err(Error::new(
                ErrorCode::WriteFailed,
                format!("could not write {what}: {e}"),
            )
            .in_file(path))
        }
    }
}

/// A new name for a file of AgentDrafter's own beside `path`, ending in
/// `ending`, such as `.agentdrafter-1234-7.tmp`.
///
/// The name is short and does not hold the file's own, so a file whose
/// name is as long as the system allows can still have one. It holds the
/// process ID and a count, so no two share one.
pub(crate) fn beside(path: &Path, ending: &str) -> PathBuf {
    static NAMES: AtomicU64 = AtomicU64::new(0);
    path.with_file_name(format!(
        ".agentdrafter-{}-{}.{ending}",
        std::process::id(),
        NAMES.fetch_add(1, Ordering::Relaxed)
    ))
}
