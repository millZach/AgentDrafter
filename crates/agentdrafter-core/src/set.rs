//! Building a drawing set: one DWG for each of its rows, each a template
//! plus that row of data.
//!
//! A drawing set is a folder. Its settings, in [`SET_SETTINGS_FILE`],
//! declare its unit, the folder its DWGs go to and the file its rows are
//! in. Each row names a sheet, its template and the values the template
//! takes from it, and builds the DWG `<output>/<sheet>.dwg`.
//!
//! Each sheet is built fully or not at all (ADR-0005): a sheet that fails
//! writes nothing, and a DWG an earlier build left there stays as it was.
//! The other sheets are still built, so that one bad row does not stop a
//! long run, and the reply says for each sheet what was built and what
//! failed. Settings or rows that cannot be read stop the whole set before
//! anything is written.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::build::{build_sheet, BuildReport};
use crate::description::{read_rows, read_set_settings, read_template};
use crate::error::{Error, ErrorCode};
use crate::reply::Reply;
use crate::settings::Settings;
use crate::units::Unit;

/// The file at the top of a drawing set's folder that holds the set's
/// settings.
pub const SET_SETTINGS_FILE: &str = "set.yaml";

/// A request to build every sheet of a drawing set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetBuildRequest {
    /// The drawing set's folder, which holds its settings in
    /// [`SET_SETTINGS_FILE`].
    pub set: PathBuf,
    /// Replace each sheet's DWG that is already there, which is allowed
    /// only when it is a DWG AgentDrafter built. Without it, a sheet whose
    /// DWG is already there fails and its file is never touched.
    #[serde(default)]
    pub overwrite: bool,
}

/// What a drawing set's build did.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SetReport {
    /// The drawing set's folder.
    pub set: String,
    /// The set's declared unit, which every DWG it builds is in.
    pub units: Unit,
    /// The folder the set's DWGs go to.
    pub output: String,
    /// How many sheets were built.
    pub built: usize,
    /// How many sheets failed. Nothing was written for them.
    pub failed: usize,
    /// Each sheet, in the order its rows give them.
    pub sheets: Vec<SheetOutcome>,
}

/// What became of one sheet of a drawing set.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetOutcome {
    /// The sheet's name, as its row gives it.
    pub sheet: String,
    /// The sheet's template, as its row writes its path.
    pub template: String,
    /// Whether the sheet was built.
    pub ok: bool,
    /// What the sheet's build wrote, when it was built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<BuildReport>,
    /// Why the sheet was not built, when it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
    /// What the build noticed about the sheet that did not stop it, such as
    /// a value of its row its template does not use. Empty when there is
    /// nothing to say.
    pub notices: Vec<Notice>,
}

/// Something a build noticed about a sheet that did not stop it, placed in
/// the file it is about.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Notice {
    pub code: NoticeCode,
    pub message: String,
    /// The sheet it is about.
    pub sheet: String,
    /// The sheet's template, as its row writes its path.
    pub template: String,
    /// The path to what it is about in `file`, such as `rows[1].values.DATE`.
    pub field: String,
    /// The line in `file`, counted from 1.
    pub line: usize,
    /// The column in the line, counted from 1 in characters.
    pub column: usize,
    /// The file it is about.
    pub file: String,
}

/// What kind of notice it is. Written in `snake_case` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeCode {
    /// A row gives a value its template has no place for. The sheet is
    /// built without it, as Zach decided on 2026-11-11, and the notice keeps
    /// a stray or misspelt value in sight.
    UnusedValue,
}

/// Builds every sheet of a drawing set, reading fonts from the font folders
/// the environment names ([`Settings::from_env`]). See [`build_set_with`].
pub fn build_set(request: &SetBuildRequest) -> Reply<SetReport> {
    build_set_with(request, &Settings::from_env())
}

/// Builds every sheet of a drawing set into a DWG in the 2018 format,
/// reading fonts from the font folders `settings` names.
///
/// A set can be built in part, so this returns a reply rather than a
/// result: `ok` with the report when every sheet was built; not `ok`, with
/// the report and a `sheets_failed` error, when some failed; and not `ok`,
/// with only an error, when the set's settings or rows could not be read
/// and nothing was built.
pub fn build_set_with(request: &SetBuildRequest, settings: &Settings) -> Reply<SetReport> {
    match build_sheets(request, settings) {
        Ok(report) if report.failed > 0 => {
            let error = Error::new(
                ErrorCode::SheetsFailed,
                format!(
                    "{} of the set's {} sheets failed, and nothing was written for them; {} \
                     built. Each sheet's outcome is under `sheets`",
                    report.failed,
                    report.sheets.len(),
                    report.built
                ),
            );
            Reply::partly(report, error)
        }
        outcome => Reply::from(outcome),
    }
}

fn build_sheets(request: &SetBuildRequest, settings: &Settings) -> Result<SetReport, Error> {
    let folder = request.set.as_path();
    let set = read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let rows_file = folder.join(&set.rows);
    let rows = read_rows(&rows_file)?;
    let output = folder.join(&set.output);
    fs::create_dir_all(&output).map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not make the drawing set's output folder: {e}"),
        )
        .in_file(&output)
    })?;
    let sheets: Vec<SheetOutcome> = rows
        .iter()
        .map(|row| {
            let template = folder.join(&row.template);
            let dwg = output.join(format!("{}.dwg", row.sheet));
            let mut notices = Vec::new();
            let built = read_template(&template, row, &rows_file, set.units, &mut notices)
                .and_then(|record| {
                    let ids = record.ids();
                    build_sheet(
                        &record.sheet,
                        &ids,
                        &template,
                        &dwg,
                        request.overwrite,
                        settings,
                    )
                });
            let (result, error) = match built {
                Ok(report) => (Some(report), None),
                Err(error) => (None, Some(error)),
            };
            SheetOutcome {
                sheet: row.sheet.clone(),
                template: row.template.clone(),
                ok: result.is_some(),
                result,
                error,
                notices,
            }
        })
        .collect();
    let built = sheets.iter().filter(|s| s.ok).count();
    Ok(SetReport {
        set: folder.display().to_string(),
        units: set.units,
        output: output.display().to_string(),
        built,
        failed: sheets.len() - built,
        sheets,
    })
}
