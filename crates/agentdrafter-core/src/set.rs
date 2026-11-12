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
//!
//! Each DWG names the set it was built from in its build mark, by the
//! set's folder relative to the DWG's, so that a render or a plot of the
//! DWG on its own finds its set wherever the set's output folder is
//! ([`set_of_dwg`]). Zach decided on 2026-11-11 that the mark never holds
//! a path in full, since these DWGs go to plants.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::build::{build_sheet, is_same_file, BuildReport};
use crate::change::relative_to;
use crate::description::{read_rows, read_set_settings, read_template, template_path};
use crate::dwg;
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
    /// The set's inconsistent mark, when a change set landed on some of its
    /// sheets and failed on others: the build builds the sheets as their
    /// records are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inconsistent: Option<crate::change::Inconsistent>,
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
            let template = template_path(folder, row);
            let dwg = output.join(format!("{}.dwg", row.sheet));
            let mut notices = Vec::new();
            let built = read_template(&template, row, &rows_file, set.units, &mut notices)
                .and_then(|record| {
                    build_sheet(
                        &record,
                        &template,
                        &dwg,
                        Some(folder),
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
        inconsistent: crate::change::inconsistent(folder),
    })
}

/// The drawing set's folder `set` as the build mark of a DWG written to
/// `dwg` records it: relative to the DWG's folder, as the file system
/// resolves both, with `/` between folders on every system, such as `..`
/// for a DWG in the set's `dwg` folder or `../demo-plant` for one in
/// `../issued`; `.` when the DWG is in the set's own folder. None when no
/// relative path leads there, as from a folder on another drive on
/// Windows: the mark never holds a path in full.
pub(crate) fn set_in_mark(dwg: &Path, set: &Path) -> Option<String> {
    relative_path(&physical(folder_of(dwg))?, &physical(set)?)
}

/// The drawing set a DWG was built from, found from its build mark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DwgSet {
    /// The set's folder, as the file system resolves it.
    pub folder: PathBuf,
    /// The DWG's path in the set as an accept records it, such as
    /// `dwg/DEMO-001.dwg`, or `../issued/DEMO-001.dwg` for a set whose
    /// output folder is outside it.
    pub path: String,
}

/// The drawing set the DWG at `dwg` was built from: the folder its build
/// mark names, from the DWG's folder, when that folder holds a drawing set
/// whose DWG of the sheet the DWG is named for is this very file.
///
/// Otherwise, why not, in words: the DWG carries no build mark, its mark
/// names no set, the set is not there, or it is no longer the set the DWG
/// was built from, since its output folder is not where the DWG is. The
/// words name the set as the mark writes it, never by its path in full.
pub(crate) fn set_of_dwg(dwg: &Path) -> Result<DwgSet, String> {
    let Some(mark) = dwg::build_mark(dwg) else {
        return Err(NO_MARK.to_owned());
    };
    let Some(written) = mark.set else {
        return Err(NO_SET.to_owned());
    };
    let named =
        format!("the drawing set the DWG's build mark names, `{written}` from the DWG's folder,");
    let folder = physical(folder_of(dwg))
        .and_then(|from| follow(&from, &written))
        .ok_or_else(|| {
            format!(
                "the DWG's build mark names its drawing set as `{written}`, which leads to no \
                 folder from the DWG's: a build mark names the set's folder relative to the \
                 DWG's"
            )
        })?;
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
    if !is_same_file(&folder.join(&settings.output).join(&name), dwg) {
        return Err(format!(
            "{named} is no longer the set the DWG was built from: its DWGs go to its output \
             folder, `{}`, and this DWG is not its `{name}` there, so the DWG has been copied or \
             moved out of that folder, or the set's output folder has changed",
            settings.output
        ));
    }
    // Worked out from the settings as written, as an accept works it out:
    // joined onto the folder, resolved, `..` would be taken up on Windows.
    let path = relative_to(Path::new(""), &Path::new(&settings.output).join(&name));
    Ok(DwgSet { path, folder })
}

/// Why a DWG that carries no build mark names no drawing set.
const NO_MARK: &str = "the DWG carries no build mark AgentDrafter can read, so it names no \
                       drawing set";

/// Why a DWG whose build mark records no set names none.
const NO_SET: &str = "the DWG's build mark names no drawing set: it was built from a sheet \
                      description on its own, not from a set, or into a folder from which no \
                      relative path leads to its set, such as one on another drive";

/// The folder a file at `path` is in: `.` for a bare file name.
fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    }
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

/// The path from the folder `from` to the folder `to`, both in full and
/// resolved, with `/` between folders, such as `../demo-plant`; `.` when
/// they are one. None when they do not share a root, as two drives on
/// Windows do not, or a folder's name is not Unicode.
fn relative_path(from: &Path, to: &Path) -> Option<String> {
    let from: Vec<Component> = from.components().collect();
    let to: Vec<Component> = to.components().collect();
    let root = |path: &[Component]| {
        path.iter()
            .take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
            .count()
    };
    let start = root(&from);
    if start == 0 || root(&to) != start || from[..start] != to[..start] {
        return None;
    }
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - shared];
    for part in &to[shared..] {
        parts.push(part.as_os_str().to_str()?);
    }
    Some(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

/// The folder a build mark's `written` path leads to from the folder
/// `from`, which is resolved: each `..` goes up a folder, as the file
/// system would take it from there. None when `written` is not a relative
/// path of folder names, or goes up past the root.
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
                },
                &Settings::default(),
            );
            assert!(reply.ok, "{output}: {:#?}", reply.error);
            let dwg = set.join(output).join("DEMO-002.dwg");
            assert_eq!(
                dwg::build_mark(&dwg),
                Some(BuildMark {
                    set: Some(written.to_owned())
                }),
                "{output}"
            );
            let found = set_of_dwg(&dwg).unwrap();
            assert_eq!(found.path, in_set, "{output}");
            assert!(is_same_file(&found.folder, &set), "{output}");
        }
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_set_s_folder_is_named_from_the_dwg_s_on_unix() {
        let from = |a: &str, b: &str| relative_path(Path::new(a), Path::new(b));
        assert_eq!(
            from("/jobs/plant/dwg", "/jobs/plant").as_deref(),
            Some("..")
        );
        assert_eq!(
            from("/jobs/issued", "/jobs/demo-plant").as_deref(),
            Some("../demo-plant")
        );
        assert_eq!(from("/jobs/plant", "/jobs/plant").as_deref(), Some("."));
        assert_eq!(
            from("/jobs/plant/out/a/b", "/jobs/plant").as_deref(),
            Some("../../..")
        );
        assert_eq!(from("/a", "/b/c").as_deref(), Some("../b/c"));
        assert_eq!(from("/", "/jobs").as_deref(), Some("jobs"));
        assert_eq!(from("jobs/issued", "/jobs/plant"), None, "not in full");
    }

    /// The mark is relative on Windows too, with `/` between folders, and
    /// none is recorded across drives, since it would be a path in full.
    #[cfg(windows)]
    #[test]
    fn a_set_s_folder_is_named_from_the_dwg_s_on_windows() {
        let from = |a: &str, b: &str| relative_path(Path::new(a), Path::new(b));
        assert_eq!(
            from(r"C:\jobs\issued", r"C:\jobs\demo-plant").as_deref(),
            Some("../demo-plant")
        );
        assert_eq!(
            from(r"\\?\C:\jobs\plant\dwg", r"\\?\C:\jobs\plant").as_deref(),
            Some("..")
        );
        assert_eq!(
            from(r"\\server\share\issued", r"\\server\share\plant").as_deref(),
            Some("../plant")
        );
        assert_eq!(from(r"C:\jobs\issued", r"D:\jobs\plant"), None);
        assert_eq!(from(r"\\?\C:\jobs", r"\\?\D:\jobs"), None);
        assert_eq!(from(r"C:\jobs", r"\\?\C:\jobs"), None, "resolved alike");
        // What it leads to from the DWG's folder, resolved.
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
        assert_eq!(
            set_in_mark(Path::new("not-there-yet/issued/A.dwg"), Path::new(".")).as_deref(),
            Some("../..")
        );
    }
}
