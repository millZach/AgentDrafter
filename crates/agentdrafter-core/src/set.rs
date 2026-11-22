//! Building a drawing set: one DWG for each of its rows, each a template
//! plus that row of data.
//!
//! A drawing set is a folder. Its settings, in [`SET_SETTINGS_FILE`],
//! declare its unit, the folder its DWGs go to and the rows files its rows
//! are in, built in the order given. Each row names a sheet, its template
//! and the values the template takes from it, and builds the DWG
//! `<output>/<sheet>.dwg`.
//!
//! Each sheet is built fully or not at all (ADR-0005): a sheet that fails
//! writes nothing, and a DWG an earlier build left there stays as it was.
//! The other sheets are still built, so that one bad row does not stop a
//! long run, and the reply says for each sheet what was built and what
//! failed. Settings or rows that cannot be read stop the whole set before
//! anything is written.
//!
//! Each DWG names the set it was built from in its build mark, by the
//! set's folder relative to the DWG's, so that a render or a plot of the
//! DWG on its own finds its set wherever the set's output folder is
//! ([`set_of_dwg`]). Zach decided on 2026-11-11 that the mark never holds
//! a path in full, since these DWGs go to plants. A sheet of a set whose
//! rows are in more than one rows file names its own rows file in the mark
//! too ([`sheet_mark`]), so that it is read alone.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::build::{build_sheet, is_same_file, BuildReport};
use crate::change::relative_to;
use crate::description::{
    read_rows_file_in, read_rows_files_in, read_set_settings, read_template, rows_of, rows_paths,
    template_at, template_path, Disk, Row, RowsFile, SetSettings,
};
use crate::dwg::{self, BuildMark};
use crate::error::{Error, ErrorCode};
use crate::leftovers::{leftovers, scan_error, Leftover, SetOutputs};
use crate::model::Paper;
use crate::reply::Reply;
use crate::settings::Settings;
use crate::stamp::SourceKind;
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
    /// Move the set's leftover outputs into `superseded/` in its output
    /// folder (true), or only list them (false). Without it, as the set's
    /// settings say under `clean`, which leave them where they are unless
    /// they say `clean: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clean: Option<bool>,
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
    /// Why the set's DWGs name no set in their build marks, when they
    /// cannot: the way back to the set from its output folder would name a
    /// folder above the set. A render or a plot of one of them on its own
    /// then says it was not issued.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_set_in_marks: Option<String>,
    /// The set's inconsistent mark, when a change set landed on some of its
    /// sheets and failed on others: the build builds the sheets as their
    /// records are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inconsistent: Option<crate::change::Inconsistent>,
    /// The DWGs and PDFs AgentDrafter made for the set in its output folder
    /// that no row writes any more, each moved into `superseded/` when the
    /// run cleaned, and any no row writes that could not be read to tell,
    /// marked `unreadable`. Empty when there are none; left out when the
    /// output folder could not be listed, which `leftovers_error` says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leftovers: Option<Vec<Leftover>>,
    /// Why the output folder could not be listed to look for leftovers,
    /// when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leftovers_error: Option<Error>,
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

/// Something a build or a change noticed about a sheet that did not stop
/// it, placed in the file it is about.
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
    /// An attribute tag is written in lowercase, in a block library's
    /// attribute definition or an insert's `attributes`. It is taken in
    /// capitals, as AutoCAD keeps it, and the notice says so.
    TagInCapitals,
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
        Ok(report) => {
            match scan_error(report.leftovers_error.as_ref(), report.leftovers.as_deref()) {
                Some(error) => Reply::partly(report, error),
                None => Reply::from(Ok(report)),
            }
        }
        Err(error) => Reply::from(Err(error)),
    }
}

fn build_sheets(request: &SetBuildRequest, settings: &Settings) -> Result<SetReport, Error> {
    let folder = request.set.as_path();
    let set = read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let rows_files = read_rows_files_in(&Disk, folder, &set)?;
    let rows = rows_of(&rows_files)?;
    let output = folder.join(&set.output);
    fs::create_dir_all(&output).map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not make the drawing set's output folder: {e}"),
        )
        .in_file(&output)
    })?;
    let marked = set_in_mark(folder, &set.output);
    let sheets: Vec<SheetOutcome> = rows
        .iter()
        .map(|row| {
            let mark = sheet_mark(marked.as_deref(), &rows_files, row);
            let template = template_path(folder, row);
            let dwg = output.join(format!("{}.dwg", row.sheet));
            let mut notices = Vec::new();
            let built = read_template(&template, row, set.units, &mut notices).and_then(|record| {
                build_sheet(&record, &template, &dwg, &mark, request.overwrite, settings)
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
    let scan = leftovers(
        &SetOutputs {
            folder,
            output: &output,
            marked: marked.as_deref(),
            rows_files: &rows_files,
            sheets: rows.iter().map(|row| row.sheet.as_str()).collect(),
        },
        request.clean.unwrap_or(set.clean),
    );
    Ok(SetReport {
        set: folder.display().to_string(),
        units: set.units,
        output: output.display().to_string(),
        built,
        failed: sheets.len() - built,
        sheets,
        no_set_in_marks: marked.is_none().then(|| no_set_in_marks(&set.output)),
        inconsistent: crate::change::inconsistent(folder),
        leftovers: scan.leftovers,
        leftovers_error: scan.error,
    })
}

/// The build mark of the DWG of the sheet `row` names, in a set whose DWGs'
/// marks name it as `set` ([`set_in_mark`]) and whose rows files are
/// `rows_files`. For a set of more than one rows file it names the row's
/// own as well, relative to the set's folder, so that a render or a plot of
/// the DWG on its own reads that file alone; not when the mark names no
/// set, nor when the file's path holds `;`, which ends it in the mark: that
/// DWG's set's rows files are all read, as for a set of one.
pub(crate) fn sheet_mark(set: Option<&str>, rows_files: &[RowsFile], row: &Row) -> BuildMark {
    let written = &rows_files[row.file_index].written;
    let named = set.is_some() && rows_files.len() > 1 && !written.contains(';');
    BuildMark {
        set: set.map(str::to_owned),
        rows: named.then(|| written.clone()),
    }
}

/// The drawing set whose folder is `set` as the build marks of the DWGs it
/// writes to its output folder, `output` as its settings write it, record
/// it: the way back from the output folder to the set, with `/` between
/// folders on every system, such as `..` for `output: dwg`,
/// `../demo-plant` for `output: ../issued` from a set in `demo-plant`, and
/// `.` for `output: .`. See [`way_back`].
///
/// The set's folder gives its own name alone, and only when the output
/// folder is beside the set: the name of the folder the file system
/// resolves it to, through links, which is where `..` from the output
/// folder leads.
pub(crate) fn set_in_mark(set: &Path, output: &str) -> Option<String> {
    way_back(output, || {
        let real = fs::canonicalize(set).ok()?;
        Some(real.file_name()?.to_str()?.to_owned())
    })
}

/// The way back from the output folder `output`, as a set's settings write
/// it, to the set: each folder name it goes down into is a `..` back, and
/// a `..` up out of the set is the set's own folder's name, `set_name`,
/// back. Nothing but `output` and that name reaches it, so it holds nothing
/// of where the set is on a machine and is the same on every machine.
///
/// None when the way back would name a folder above the set, as from
/// `output: ../../issued`, or `output` is not relative: a build mark never
/// holds those.
fn way_back(output: &str, set_name: impl FnOnce() -> Option<String>) -> Option<String> {
    if output.starts_with('/') {
        return None;
    }
    let mut up = 0;
    let mut down: Vec<&str> = Vec::new();
    for part in output.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if down.pop().is_none() {
                    up += 1;
                }
            }
            name => down.push(name),
        }
    }
    let mut back = vec!["..".to_owned(); down.len()];
    match up {
        0 => {}
        1 => back.push(set_name()?),
        _ => return None,
    }
    Some(if back.is_empty() {
        ".".to_owned()
    } else {
        back.join("/")
    })
}

/// Why the DWGs of a set whose output folder is `output`, as its settings
/// write it, name no set in their build marks.
fn no_set_in_marks(output: &str) -> String {
    format!(
        "the set's DWGs name no set in their build marks: the way back to the set from its output \
         folder, `{output}`, would name a folder above the set, which a build mark never holds. \
         A render or a plot of one of these DWGs on its own says it was not issued; the set's own \
         plot still says so. An output folder inside the set, or beside it, such as `dwg` or \
         `../issued`, is named"
    )
}

/// The drawing set a DWG was built from, found from its build mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DwgSet {
    /// The set's folder, in full: as reached from the DWG's folder as it
    /// was given, or as the file system resolves it.
    pub folder: PathBuf,
    /// The DWG's path in the set as an accept records it, such as
    /// `dwg/DEMO-001.dwg`, or `../issued/DEMO-001.dwg` for a set whose
    /// output folder is outside it.
    pub path: String,
    /// The rows file the DWG's sheet's row is in, as its build mark names
    /// it, relative to the set's folder: named for a set of more than one
    /// rows file ([`sheet_mark`]). None when the mark names none.
    pub rows: Option<String>,
}

/// The drawing set the DWG at `dwg` was built from: the folder its build
/// mark names, from the DWG's folder, when that folder holds a drawing set
/// whose DWG of the sheet the DWG is named for is this very file. The mark
/// is followed from the DWG's folder as it is given, as the set writes its
/// output folder, and failing that from the folder as the file system
/// resolves it, through links.
///
/// Otherwise, why not, in words: the DWG carries no build mark, its mark
/// names no set, the set is not there, or it is no longer the set the DWG
/// was built from, since its output folder is not where the DWG is. The
/// words name the set as the mark writes it, never by its path in full.
pub(crate) fn set_of_dwg(dwg: &Path) -> Result<DwgSet, String> {
    set_of_dwg_with_mark(dwg, None, dwg::build_mark(dwg))
}

fn set_of_dwg_with_mark(
    dwg: &Path,
    source_id: Option<crate::build::FileId>,
    mark: Option<dwg::BuildMark>,
) -> Result<DwgSet, String> {
    let Some(mark) = mark else {
        return Err(NO_MARK.to_owned());
    };
    let Some(written) = mark.set else {
        return Err(NO_SET.to_owned());
    };
    let from = folder_of(dwg);
    let mut folders: Vec<PathBuf> = [lexical(from), physical(from)]
        .into_iter()
        .flatten()
        .filter_map(|from| follow(&from, &written))
        .collect();
    folders.dedup();
    let mut why = None;
    for folder in folders {
        match set_at(folder, dwg, source_id, &written) {
            Ok(set) => {
                return Ok(DwgSet {
                    rows: mark.rows,
                    ..set
                })
            }
            Err(not) => {
                why.get_or_insert(not);
            }
        }
    }
    Err(why.unwrap_or_else(|| {
        format!(
            "the DWG's build mark names its drawing set as `{written}`, which leads to no folder \
             from the DWG's: a build mark names the set's folder relative to the DWG's"
        )
    }))
}

/// The drawing set at `folder`, which the build mark of the DWG at `dwg`
/// names as `written`, if it is the set the DWG was built from.
fn set_at(
    folder: PathBuf,
    dwg: &Path,
    source_id: Option<crate::build::FileId>,
    written: &str,
) -> Result<DwgSet, String> {
    let named =
        format!("the drawing set the DWG's build mark names, `{written}` from the DWG's folder,");
    let settings_file = folder.join(SET_SETTINGS_FILE);
    if !settings_file.is_file() {
        return Err(format!(
            "{named} is not there: that folder holds no set's settings ({SET_SETTINGS_FILE}), \
             so the DWG or its set has been moved, or the set taken away"
        ));
    }
    let settings = read_set_settings(&settings_file)
        .map_err(|e| format!("{named} cannot be read: {}", e.message))?;
    let name = format!(
        "{}.dwg",
        dwg.file_stem().unwrap_or_default().to_string_lossy()
    );
    let built = folder.join(&settings.output).join(&name);
    if !crate::build::is_same_file_id(&built, dwg, source_id) {
        return Err(format!(
            "{named} is no longer the set the DWG was built from: its DWGs go to its output \
             folder, `{}`, and this DWG is not its `{name}` there, so the DWG has been copied or \
             moved out of that folder, or the set's output folder has changed",
            settings.output
        ));
    }
    // Named in the set as an accept names it, by the same rule.
    Ok(DwgSet {
        path: relative_to(&folder, &built),
        folder,
        rows: None,
    })
}

/// What the sheet a DWG was built as declares about its paper, read from
/// the drawing set its build mark names ([`set_of_dwg`]): its template
/// filled in from its row, as a build reads it.
pub(crate) struct DwgPaper {
    /// The paper the sheet declares, in `units`; none when it declares
    /// none.
    pub paper: Option<Paper>,
    /// The unit the paper is in: the set's.
    pub units: Unit,
    /// The template and the rows, with their bytes, so that an output
    /// drawn from the DWG can be stamped with them: whether it declares a
    /// paper or not, a change to them can change the output's page.
    pub read: Vec<FileRead>,
}

/// What the sheet a DWG was built as declares about its paper ([`DwgPaper`]).
/// None when the DWG names no set, or the set, its rows or the DWG's row
/// or template cannot be read: the DWG then plots to its extents. When the
/// DWG's mark names its rows file, only that file is read, so another of
/// the set's rows files that cannot be read does not touch it. The DWG's
/// row is the one whose DWG in the set's output folder is this very file,
/// as the file system tells it, so letter case is matched as the file
/// system matches it.
#[cfg(test)]
pub(crate) fn paper_of_dwg(dwg: &Path) -> Option<DwgPaper> {
    paper_of_dwg_in(
        dwg,
        crate::build::file_id_of_path(dwg),
        set_of_dwg(dwg).ok()?,
    )
}

/// Same as [`paper_of_dwg`], reading the build mark from `bytes` already
/// loaded from `dwg` and matching rows against `source_id` captured from that
/// same opened handle so the DWG file is never opened a second time. Refuses
/// with [`ErrorCode::RollbackFailed`] without modifying the set when the set
/// the DWG belongs to holds an unfinished journal commit (ADR-0005).
pub(crate) fn paper_of_dwg_bytes(
    dwg: &Path,
    source_id: Option<crate::build::FileId>,
    bytes: &[u8],
) -> Result<Option<DwgPaper>, Error> {
    let Ok(set) = set_of_dwg_with_mark(dwg, source_id, dwg::build_mark_bytes(bytes)) else {
        return Ok(None);
    };
    crate::change::refuse_unfinished(&set.folder, crate::change::Unfinished::BuiltDwg)?;
    Ok(paper_of_dwg_in(dwg, source_id, set))
}

fn paper_of_dwg_in(
    dwg: &Path,
    source_id: Option<crate::build::FileId>,
    set: DwgSet,
) -> Option<DwgPaper> {
    let settings_file = set.folder.join(SET_SETTINGS_FILE);
    let settings = read_set_settings(&settings_file).ok()?;
    let rows_files = marked_rows_files(&set.folder, &settings, set.rows.as_deref())?;
    let rows = rows_of(&rows_files).ok()?;
    let output = set.folder.join(&settings.output);
    let row = rows.iter().find(|r| {
        crate::build::is_same_file_id(&output.join(format!("{}.dwg", r.sheet)), dwg, source_id)
    })?;
    let rows_file = &rows_files[row.file_index];
    let template = template_path(&set.folder, row);
    let template_bytes = fs::read(&template).ok()?;
    let text = std::str::from_utf8(&template_bytes).ok()?;
    let mut notices = Vec::new();
    let (record, _) = template_at(text, &template, row, settings.units, &mut notices).ok()?;
    Some(DwgPaper {
        paper: record.sheet.paper,
        units: settings.units,
        read: vec![
            (SourceKind::Template, template, template_bytes),
            (
                SourceKind::Rows,
                rows_file.path.clone(),
                rows_file.text.clone().into_bytes(),
            ),
        ],
    })
}

/// The rows files the sheet of a DWG whose build mark names `marked` is
/// read from, in the set whose folder is `folder` and whose settings are
/// `settings`: the rows file the mark names, alone, when it is one of the
/// set's rows files, found by the file it leads to, however the settings
/// spell it now; otherwise, when the mark names none or one that is no
/// longer the set's, every rows file of the set, as for a set of one. None
/// when they cannot be found or read.
fn marked_rows_files(
    folder: &Path,
    settings: &SetSettings,
    marked: Option<&str>,
) -> Option<Vec<RowsFile>> {
    if let Some(written) = marked {
        let wanted = folder.join(written);
        let plain = relative_to(folder, &wanted);
        let found = rows_paths(folder, settings)
            .ok()?
            .into_iter()
            .find(|found| {
                relative_to(folder, &found.path) == plain || is_same_file(&found.path, &wanted)
            });
        if let Some(found) = found {
            return read_rows_file_in(&Disk, found).ok().map(|file| vec![file]);
        }
    }
    read_rows_files_in(&Disk, folder, settings).ok()
}

/// A file read to draw an output, with what it is and its bytes, for the
/// output's stamp.
pub(crate) type FileRead = (SourceKind, PathBuf, Vec<u8>);

/// Why a DWG that carries no build mark names no drawing set.
const NO_MARK: &str = "the DWG carries no build mark AgentDrafter can read, so it names no \
                       drawing set";

/// Why a DWG whose build mark records no set names none.
const NO_SET: &str = "the DWG's build mark names no drawing set: it was built from a sheet \
                      description on its own, not from a set, or by a set whose output folder \
                      the way back to the set from would name a folder above the set";

/// The folder a file at `path` is in: `.` for a bare file name.
fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    }
}

/// `path` in full, made whole from the working folder, with `.` and `..`
/// worked out as written: no link is followed.
fn lexical(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    let mut whole = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                whole.pop();
            }
            other => whole.push(other.as_os_str()),
        }
    }
    Some(whole)
}

/// `path` as the file system resolves it: in full, through links, with
/// `.` and `..` worked out. The part of it that is not there yet, such as
/// an output folder a change set will make, is worked out as written.
fn physical(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    let (mut resolved, rest) = absolute.ancestors().find_map(|there| {
        let rest = absolute.strip_prefix(there).ok()?;
        Some((fs::canonicalize(there).ok()?, rest))
    })?;
    for part in rest.components() {
        match part {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => resolved.push(name),
            _ => {}
        }
    }
    Some(resolved)
}

/// The folder a build mark's `written` path leads to from the folder
/// `from`, which is in full with no `.` or `..`: each `..` goes up a
/// folder. None when `written` is not a relative path of folder names, or
/// goes up past the root.
fn follow(from: &Path, written: &str) -> Option<PathBuf> {
    if written.starts_with('/') {
        return None;
    }
    let mut folder = from.to_path_buf();
    for part in written.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !folder.pop() {
                    return None;
                }
            }
            name => {
                let mut components = Path::new(name).components();
                match (components.next(), components.next()) {
                    (Some(Component::Normal(_)), None) => folder.push(name),
                    _ => return None,
                }
            }
        }
    }
    Some(folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dwg::BuildMark;

    /// Copies the YAML files of the folder `from` into `to`, keeping its
    /// folders.
    fn copy_yaml(from: &Path, to: &Path) {
        for entry in fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            let target = to.join(path.file_name().unwrap());
            if path.is_dir() {
                copy_yaml(&path, &target);
            } else if path.extension().is_some_and(|e| e == "yaml") {
                fs::create_dir_all(to).unwrap();
                fs::copy(&path, &target).unwrap();
            }
        }
    }

    /// The rows file a DWG's build mark names is found among the set's
    /// however the settings spell it now, and read alone; when it is no
    /// longer one of them, every rows file of the set is read, so the
    /// sheet still finds its paper and its row's file.
    #[test]
    fn a_marked_rows_file_is_found_however_the_settings_spell_it_and_else_all_are_read() {
        let set = std::env::temp_dir()
            .join(format!(
                "agentdrafter-set-marked-rows-{}",
                std::process::id()
            ))
            .join("two-kinds");
        let _ = fs::remove_dir_all(&set);
        copy_yaml(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/two-kinds"),
            &set,
        );
        let reply = build_set_with(
            &SetBuildRequest {
                set: set.clone(),
                overwrite: false,
                clean: None,
            },
            &Settings::default(),
        );
        assert!(reply.ok, "{:#?}", reply.error);
        let dwg = set.join("dwg").join("SCHED-201.dwg");
        assert_eq!(
            dwg::build_mark(&dwg).and_then(|mark| mark.rows).as_deref(),
            Some("rows/schedules.yaml")
        );
        // The rows files the DWG's paper is read from, under `rows`.
        let rows_read = |rows: &str| -> Vec<String> {
            fs::write(
                set.join(SET_SETTINGS_FILE),
                format!("units: in\noutput: dwg\nrows: {rows}\n"),
            )
            .unwrap();
            let found = paper_of_dwg(&dwg).expect("the sheet's paper is found");
            assert!(found.paper.is_some(), "{rows}");
            found
                .read
                .into_iter()
                .filter(|(kind, ..)| *kind == SourceKind::Rows)
                .map(|(_, path, _)| relative_to(&set, &path))
                .collect()
        };

        // Respelled, and read alone: another rows file that cannot be read
        // does not touch it.
        let areas = set.join("rows").join("areas.yaml");
        let areas_text = fs::read_to_string(&areas).unwrap();
        fs::write(&areas, "rows: [\n").unwrap();
        assert_eq!(
            rows_read("[rows/areas.yaml, ./rows/schedules.yaml]"),
            ["rows/schedules.yaml"]
        );
        assert_eq!(
            rows_read("[rows/areas.yaml, rows/../rows/schedules.yaml]"),
            ["rows/schedules.yaml"]
        );
        fs::write(&areas, areas_text).unwrap();

        // The marked file is no longer one of the set's: every rows file is
        // read, and the sheet's row is found in the file it is in now.
        fs::rename(
            set.join("rows").join("schedules.yaml"),
            set.join("rows").join("sched.yaml"),
        )
        .unwrap();
        assert_eq!(
            rows_read("[rows/areas.yaml, rows/sched.yaml]"),
            ["rows/sched.yaml"]
        );
        let _ = fs::remove_dir_all(set.parent().unwrap());
    }

    /// Built on this system's own paths, on Windows as on the others: each
    /// DWG names its set relative to its own folder, with `/` between
    /// folders, and is found to be the set's from it.
    #[test]
    fn a_set_s_dwgs_name_its_folder_relative_to_their_own_on_every_system() {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-set-mark-{}", std::process::id()))
            .join("jobs");
        let _ = fs::remove_dir_all(&dir);
        let set = dir.join("demo-set");
        copy_yaml(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo-set"),
            &set,
        );
        let settings = fs::read_to_string(set.join(SET_SETTINGS_FILE)).unwrap();
        for (output, written, in_set) in [
            ("dwg", "..", "dwg/DEMO-002.dwg"),
            ("../issued", "../demo-set", "../issued/DEMO-002.dwg"),
            ("out/a/b", "../../..", "out/a/b/DEMO-002.dwg"),
            (".", ".", "DEMO-002.dwg"),
            ("../../issued", "", ""),
        ] {
            fs::write(
                set.join(SET_SETTINGS_FILE),
                settings.replace("output: dwg", &format!("output: {output}")),
            )
            .unwrap();
            let reply = build_set_with(
                &SetBuildRequest {
                    set: set.clone(),
                    overwrite: true,
                    clean: None,
                },
                &Settings::default(),
            );
            assert!(reply.ok, "{output}: {:#?}", reply.error);
            let dwg = set.join(output).join("DEMO-002.dwg");
            let said = reply.result.unwrap().no_set_in_marks;
            if written.is_empty() {
                // The way back would name `jobs`, above the set: the DWG
                // names no set, and the reply says why.
                assert_eq!(
                    dwg::build_mark(&dwg),
                    Some(BuildMark {
                        set: None,
                        rows: None
                    })
                );
                assert!(said.unwrap().contains("`../../issued`"));
                assert_eq!(set_of_dwg(&dwg).unwrap_err(), NO_SET);
                continue;
            }
            assert_eq!(said, None, "{output}");
            assert_eq!(
                dwg::build_mark(&dwg),
                Some(BuildMark {
                    set: Some(written.to_owned()),
                    rows: None,
                }),
                "{output}"
            );
            let found = set_of_dwg(&dwg).unwrap();
            assert_eq!(found.path, in_set, "{output}");
            assert!(is_same_file(&found.folder, &set), "{output}");
        }
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    /// A mark is followed on Windows paths as on any other, `/` between its
    /// folders, and a mark that is not folder names leads nowhere.
    #[cfg(windows)]
    #[test]
    fn a_mark_s_path_is_followed_on_windows() {
        assert_eq!(
            follow(Path::new(r"\\?\C:\jobs\issued"), "../demo-plant"),
            Some(PathBuf::from(r"\\?\C:\jobs\demo-plant"))
        );
        assert_eq!(follow(Path::new(r"\\?\C:\jobs"), "C:/jobs"), None);
        assert_eq!(follow(Path::new(r"\\?\C:\jobs"), r"..\plant"), None);
    }

    #[test]
    fn a_mark_s_path_is_followed_from_the_dwg_s_folder() {
        let root = physical(Path::new(".")).unwrap();
        let issued = root.join("jobs").join("issued");
        assert_eq!(
            follow(&issued, "../demo-plant"),
            Some(root.join("jobs").join("demo-plant"))
        );
        assert_eq!(follow(&issued, ".."), Some(root.join("jobs")));
        assert_eq!(follow(&issued, "."), Some(issued.clone()));
        assert_eq!(follow(&issued, "/jobs/demo-plant"), None);
        let top = root.ancestors().last().unwrap();
        assert_eq!(follow(top, ".."), None, "up past the root");
    }

    #[test]
    fn a_folder_not_there_yet_is_resolved_from_the_part_that_is() {
        let here = fs::canonicalize(".").unwrap();
        assert_eq!(
            physical(Path::new("not-there-yet/dwg")),
            Some(here.join("not-there-yet").join("dwg"))
        );
        assert_eq!(
            physical(Path::new("not-there-yet/../out")),
            Some(here.join("out"))
        );
        assert_eq!(folder_of(Path::new("A.dwg")), Path::new("."));
    }

    /// The mark is the way back from the output folder to the set, worked
    /// out from `output` as written and the set's own folder's name alone,
    /// and never names a folder above the set.
    #[test]
    fn a_mark_is_the_way_back_from_the_output_folder_as_the_set_writes_it() {
        let mark = |output: &str| way_back(output, || Some("demo-set".to_owned()));
        assert_eq!(mark("dwg").as_deref(), Some(".."));
        assert_eq!(mark("../issued").as_deref(), Some("../demo-set"));
        assert_eq!(mark("out/a/b").as_deref(), Some("../../.."));
        assert_eq!(mark(".").as_deref(), Some("."));
        assert_eq!(mark("./dwg").as_deref(), Some(".."));
        assert_eq!(mark("dwg/../out").as_deref(), Some(".."));
        assert_eq!(mark("..").as_deref(), Some("demo-set"));
        assert_eq!(mark("../shared/issued").as_deref(), Some("../../demo-set"));
        // The way back would name the folder above the set: no set.
        assert_eq!(mark("../../issued"), None);
        assert_eq!(mark("../../../x/issued"), None);
        assert_eq!(mark("/srv/issued"), None);
        // The set's name is asked for only when the way back needs it.
        assert_eq!(
            way_back("dwg", || panic!("not needed")).as_deref(),
            Some("..")
        );
        assert_eq!(way_back("../issued", || None), None, "a set with no name");
        assert!(no_set_in_marks("../../issued").contains("`../../issued`"));
    }

    /// The set's name is its real folder's: a set reached as `link/..`,
    /// where `link` leads into the set, is named as the set, never by the
    /// folder the path goes through.
    #[cfg(unix)]
    #[test]
    fn a_set_reached_through_a_link_is_named_by_its_real_folder() {
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-set-name-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let set = dir.join("srv").join("demo-set");
        fs::create_dir_all(set.join("child")).unwrap();
        fs::create_dir_all(dir.join("home").join("zach")).unwrap();
        std::os::unix::fs::symlink(set.join("child"), dir.join("home/zach/link")).unwrap();
        let reached = dir.join("home/zach/link/..");
        assert_eq!(
            set_in_mark(&reached, "../issued").as_deref(),
            Some("../demo-set")
        );
        assert_eq!(set_in_mark(&reached, "dwg").as_deref(), Some(".."));
        let _ = fs::remove_dir_all(&dir);
    }
}
