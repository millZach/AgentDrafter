//! Build: producing a DWG from a sheet description.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::description::{read_sheet_record, SheetRecord};
use crate::dwg;
use crate::error::{Error, ErrorCode};
use crate::settings::Settings;
use crate::text::fonts::FontSubstitution;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// A request to build one sheet description into a DWG.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRequest {
    /// The sheet description to build.
    pub description: PathBuf,
    /// Where to write the DWG. Without it, the DWG goes beside the sheet
    /// description with the same name and a `.dwg` extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    /// Replace the file at the output path, which is allowed only when it is
    /// a DWG AgentDrafter built. Without it, an existing file is never
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
    /// The DWG that was written.
    pub dwg: String,
    /// The DWG version it was written in: `AC1032`, the 2018 format.
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
}

/// Builds a sheet description into a DWG in the 2018 format, reading fonts
/// from the font folders the environment names ([`Settings::from_env`]).
///
/// The DWG is written whole or not at all: it is written beside its final
/// path and moved into place once complete.
pub fn build(request: &BuildRequest) -> Result<BuildReport, Error> {
    build_with(request, &Settings::from_env())
}

/// Builds a sheet description into a DWG in the 2018 format, reading fonts
/// from the font folders `settings` names. A block library the sheet
/// description names is read from beside it.
pub fn build_with(request: &BuildRequest, settings: &Settings) -> Result<BuildReport, Error> {
    let description = request.description.as_path();
    let record = read_sheet_record(description)?;
    let output = request
        .output
        .clone()
        .unwrap_or_else(|| description.with_extension("dwg"));
    build_sheet(&record, description, &output, request.overwrite, settings)
}

/// Builds the sheet `record` holds, read from the sheet description or
/// template at `description`, into a DWG at `output`, written whole or not
/// at all, each object carrying the ID and name its record gives it. An
/// existing file there is replaced only when `overwrite` is asked for and
/// it carries AgentDrafter's build mark.
pub(crate) fn build_sheet(
    record: &SheetRecord,
    description: &Path,
    output: &Path,
    overwrite: bool,
    settings: &Settings,
) -> Result<BuildReport, Error> {
    let prepared = prepare_sheet(record, description, output, overwrite, settings)?;
    write_whole(output, &prepared.bytes)?;
    Ok(prepared.report)
}

/// A sheet built in memory, ready to write: its DWG's bytes and what the
/// build will report once they are written.
pub(crate) struct PreparedSheet {
    pub report: BuildReport,
    pub bytes: Vec<u8>,
}

/// Builds the sheet `record` holds into the bytes of a DWG for `output`,
/// as [`build_sheet`] does, and refuses as it does, but writes nothing.
pub(crate) fn prepare_sheet(
    record: &SheetRecord,
    description: &Path,
    output: &Path,
    overwrite: bool,
    settings: &Settings,
) -> Result<PreparedSheet, Error> {
    if fs::symlink_metadata(output).is_ok() {
        if is_same_file(description, output) {
            return Err(Error::usage(
                "the output path is the sheet description itself; choose another path for the DWG",
            )
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

    let sheet = &record.sheet;
    let fonts = SheetFonts::find(sheet, settings);
    let bytes =
        dwg::write_new_sheet(record, &|text| fonts.start(text)).map_err(|e| e.in_file(output))?;

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
    let report = BuildReport {
        description: description.display().to_string(),
        dwg: output.display().to_string(),
        dwg_version: dwg::new_sheet_version(),
        units: sheet.units,
        block_library,
        blocks: sheet.blocks.iter().map(|b| b.name.clone()).collect(),
        objects,
        bytes: bytes.len(),
        font_substitutions: fonts.substitutions(),
    };
    Ok(PreparedSheet { report, bytes })
}

/// Refuses to replace the file at `output` unless it is a DWG AgentDrafter
/// built, as its build mark shows. Anything else may be an original, and an
/// original is never overwritten.
fn refuse_unless_built(output: &Path) -> Result<(), Error> {
    let why = match dwg::carries_build_mark(output) {
        Ok(true) => return Ok(()),
        Ok(false) => "the DWG at the output path was not built by AgentDrafter: it does not carry \
                      AgentDrafter's build mark"
            .to_owned(),
        Err(e) => format!(
            "the file at the output path cannot be read as a DWG, so nothing shows AgentDrafter \
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

fn is_same_file(a: &Path, b: &Path) -> bool {
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
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Error> {
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
