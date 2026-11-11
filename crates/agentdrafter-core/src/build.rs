//! Build: producing a DWG from a sheet description.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::description::parse_sheet_description;
use crate::dwg;
use crate::error::{Error, ErrorCode};
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
    /// Replace the file at the output path. Without it, an existing file is
    /// never touched.
    #[serde(default)]
    pub overwrite: bool,
}

/// What a build wrote.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BuildReport {
    /// The sheet description that was built.
    pub description: String,
    /// The DWG that was written.
    pub dwg: String,
    /// The DWG version it was written in: `AC1032`, the 2018 format.
    pub dwg_version: String,
    /// The sheet's declared unit, which is the drawing's unit.
    pub units: Unit,
    /// How many objects of each type the DWG holds, such as `{"line": 1}`.
    pub objects: BTreeMap<String, usize>,
    /// The size of the DWG in bytes.
    pub bytes: usize,
}

/// Builds a sheet description into a DWG in the 2018 format.
///
/// The DWG is written whole or not at all: it is written beside its final
/// path and moved into place once complete.
pub fn build(request: &BuildRequest) -> Result<BuildReport, Error> {
    let description = request.description.as_path();
    let text = fs::read_to_string(description).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the sheet description: {e}"),
        )
        .in_file(description)
    })?;
    let sheet = parse_sheet_description(&text).map_err(|e| e.in_file(description))?;

    let output = request
        .output
        .clone()
        .unwrap_or_else(|| description.with_extension("dwg"));
    if fs::symlink_metadata(&output).is_ok() {
        if is_same_file(description, &output) {
            return Err(Error::usage(
                "the output path is the sheet description itself; choose another path for the DWG",
            )
            .in_file(&output));
        }
        if !request.overwrite {
            return Err(Error::new(
                ErrorCode::OutputExists,
                "a file is already at the output path; ask to overwrite it to replace a previous build",
            )
            .in_file(&output));
        }
    }

    let bytes = dwg::write_new_sheet(&sheet).map_err(|e| e.in_file(&output))?;
    write_whole(&output, &bytes)?;

    let mut objects = BTreeMap::new();
    for object in &sheet.objects {
        *objects.entry(object.type_name().to_owned()).or_insert(0) += 1;
    }
    Ok(BuildReport {
        description: description.display().to_string(),
        dwg: output.display().to_string(),
        dwg_version: dwg::new_sheet_version(),
        units: sheet.units,
        objects,
        bytes: bytes.len(),
    })
}

fn is_same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Writes `bytes` to a temporary file beside `path`, then moves it onto
/// `path`, so a reader never sees a half-written DWG.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let Some(name) = path.file_name() else {
        return Err(Error::usage("the output path does not name a file").in_file(path));
    };
    let mut temporary_name = OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(temporary_name);
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
            format!("could not write the DWG: {e}"),
        )
        .in_file(path)
    })
}
