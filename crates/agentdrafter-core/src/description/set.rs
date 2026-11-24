//! Reads a drawing set's settings and its rows.
//!
//! A drawing set is a folder. Its settings, in `set.yaml` at the top of the
//! folder, declare the set's unit, the folder its DWGs go to and the rows
//! files its rows are in: one, a folder of them or a list, built in the order
//! given. Each row names one sheet, the template it is drawn from and the
//! values the template takes from it, and may hold objects of its own, which
//! are on that sheet alone. Sheets' names are unique across the rows files.
//! Every file is written in the same
//! strict YAML as a sheet description, and `docs/drawing-set.md` sets them
//! out.

use std::fs;
use std::path::{Path, PathBuf};

use super::blocks::{relative_path, RelativePath};
use super::strict_yaml::{self, index_path, key_path, Kind, Node, Pos};
use super::{
    at, bad_value, read_file_in, scalar, sequence, unit, wrong_type, Disk, Fields, ReadFiles,
};
use crate::build::{is_same_file, DrawingFormat};
use crate::change::relative_to;
use crate::error::{Error, ErrorCode};
use crate::set::SET_SETTINGS_FILE;
use crate::units::Unit;

/// The keys of a drawing set's settings.
pub(crate) const SETTINGS_KEYS: [&str; 6] =
    ["units", "output", "rows", "clean", "plot_style", "format"];

/// The keys at the top of a set's rows file.
pub(crate) const ROWS_FILE_KEYS: [&str; 1] = ["rows"];

/// The keys a row takes.
pub(crate) const ROW_KEYS: [&str; 4] = ["sheet", "template", "values", ROW_OBJECTS_KEY];

/// The key of a row's own objects, which are on its sheet alone.
pub(crate) const ROW_OBJECTS_KEY: &str = "objects";

/// The longest sheet name, in characters: with `.dwg` after it, its file
/// name is at most 255 characters, as Windows allows.
pub(crate) const LONGEST_SHEET_NAME: usize = 251;

/// Characters a sheet name cannot hold, since it names a file on Windows,
/// Linux and macOS: `/` and `\`, which separate folders, and the others
/// Windows refuses in a file name.
pub(crate) const FORBIDDEN_IN_SHEET_NAMES: &str = "/\\:*?\"<>|";

/// The names Windows keeps for devices. A file whose name, up to its first
/// `.`, is one of them, in any case, cannot be made there.
pub(crate) const DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A drawing set's settings.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SetSettings {
    /// The set's declared unit: every template, sheet description and block
    /// library in the set is in it.
    pub units: Unit,
    /// The folder the set's DWGs go to, relative to the set's folder, as the
    /// settings write it.
    pub output: String,
    /// Each path `rows` names, relative to the set's folder, as the
    /// settings write it: a rows file or a folder of them. One, unless
    /// `rows` is a list.
    pub rows: Vec<RowsNamed>,
    /// Whether `rows` is written as a list of paths, not as one path.
    pub rows_listed: bool,
    /// Whether a build or a plot of the set moves its leftover outputs into
    /// `superseded/` unless a run asks otherwise: `clean: true`. False when
    /// the settings leave it out.
    pub clean: bool,
    /// The CTB or STB plot style file name the set plots with unless a plot
    /// request names another.
    pub plot_style: Option<String>,
    /// The drawing format the set builds (`dwg`, ASCII `dxf`, or `dxf_binary`),
    /// unless a build asks for another. `dwg` when the settings leave it out.
    pub format: DrawingFormat,
}

/// A path the settings' `rows` names: a rows file or a folder of them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowsNamed {
    /// The path as the settings write it, relative to the set's folder.
    pub written: String,
    /// Where the settings write it, such as `rows` or `rows[1]`.
    pub field: String,
    pub line: usize,
    pub column: usize,
}

/// One rows file of a drawing set, as it is read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowsFile {
    /// Its path relative to the set's folder, as the settings reach it,
    /// with `/` between folders: `rows.yaml`, or `rows/loops.yaml` for the
    /// file `loops.yaml` of the folder `rows`.
    pub written: String,
    /// Its path joined onto the set's folder.
    pub path: PathBuf,
    /// Its text.
    pub text: String,
}

/// One row of a drawing set: one sheet, the template it is drawn from and
/// the values the template takes from it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// The sheet's name, which names its DWG.
    pub sheet: String,
    /// The template, relative to the set's folder, as the row writes it.
    pub template: String,
    /// The rows file the row is in, joined onto the set's folder.
    pub file: PathBuf,
    /// The place of the row's rows file among the set's, counted from 0.
    pub(crate) file_index: usize,
    /// Where the row is in its rows file, counted from 0.
    pub(crate) index: usize,
    /// Where the row's values are written, or the row itself when it has none.
    pub(super) at: Pos,
    /// The row's values, in the order it writes them.
    pub(super) values: Vec<RowValue>,
    /// The row's own objects, as the rows file writes them: they are on
    /// this sheet alone, after its template's objects. They are read with
    /// the sheet, since what they may name is in its template.
    pub(super) objects: Vec<Node>,
}

/// One value of a row: the text a template's `{row: NAME}` stands for.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RowValue {
    pub name: String,
    pub text: String,
    pub key_at: Pos,
    pub at: Pos,
}

impl Row {
    pub(super) fn value(&self, name: &str) -> Option<&RowValue> {
        self.values.iter().find(|v| v.name == name)
    }

    /// The path of the row's value `name` in the rows file, such as
    /// `rows[2].values.TITLE`.
    pub(super) fn value_path(&self, name: &str) -> String {
        key_path(&key_path(&index_path("rows", self.index), "values"), name)
    }

    /// The path of the row's objects in the rows file, such as
    /// `rows[2].objects`.
    pub(super) fn objects_path(&self) -> String {
        key_path(&index_path("rows", self.index), ROW_OBJECTS_KEY)
    }
}

/// The file `row`'s template is in, for a set whose folder is `folder`:
/// the path the row writes, relative to the set's folder. Every reader of a
/// set finds a sheet's template here.
pub(crate) fn template_path(folder: &Path, row: &Row) -> PathBuf {
    folder.join(&row.template)
}

/// Reads the drawing set's settings in the file at `path`.
pub(crate) fn read_set_settings(path: &Path) -> Result<SetSettings, Error> {
    read_set_settings_in(&Disk, path)
}

/// Reads the drawing set's settings in the file at `path`, from `files`.
pub(crate) fn read_set_settings_in(
    files: &dyn ReadFiles,
    path: &Path,
) -> Result<SetSettings, Error> {
    let text = read_file_in(files, path, "the drawing set's settings")?;
    parse_set_settings(&text).map_err(|e| e.or_in_file(path))
}

/// Reads the text of a drawing set's settings.
pub(super) fn parse_set_settings(text: &str) -> Result<SetSettings, Error> {
    let Some(root) = strict_yaml::read(text)? else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            "the drawing set's settings are empty; they need `units`, `output` and `rows`",
        ));
    };
    let fields = Fields::of(&root, "", &SETTINGS_KEYS, "a drawing set's settings")?;
    let units = match fields.get("units") {
        Some(node) => unit(node, "units")?,
        None => {
            return Err(Error::new(
                ErrorCode::MissingUnit,
                format!(
                    "the drawing set declares no unit; add `units:` with one of {}",
                    Unit::symbols()
                ),
            )
            .at_field("units")
            .at_position(root.at.line, root.at.column))
        }
    };
    let output = relative_path(
        fields.require_text("output")?,
        "output",
        &RelativePath {
            shape: "the folder the set's DWGs go to, such as `dwg`",
            whose: "the output folder's path",
            from: "the drawing set's folder",
            names: "a folder",
        },
    )?;
    let given = fields.require("rows")?;
    let rows_listed = matches!(given.kind, Kind::Sequence(_));
    let rows = if rows_listed {
        let items = sequence(given, "rows", "a list of rows files and folders of them")?;
        if items.is_empty() {
            return Err(bad_value(
                given,
                "rows",
                "`rows` lists no rows file; list one or more, each the path of a rows file or \
                 of a folder of them, such as `rows/loops.yaml` or `rows`",
            ));
        }
        let named = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let field = index_path("rows", i);
                let shape = "the path of a rows file or of a folder of them, such as \
                             `rows/loops.yaml` or `rows`";
                // A bare null names no file, so it is no path in a list.
                if item.is_bare_null() {
                    return Err(wrong_type(item, &field, shape));
                }
                let written = relative_path(
                    item,
                    &field,
                    &RelativePath {
                        shape,
                        whose: "a rows file's or folder's path",
                        from: "the drawing set's folder",
                        names: "a file or a folder",
                    },
                )?;
                Ok(rows_named(written, field, item.at))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        for (i, later) in named.iter().enumerate() {
            if let Some(first) = named[..i].iter().find(|n| n.written == later.written) {
                return Err(Error::new(
                    ErrorCode::DuplicateName,
                    format!(
                        "the rows file `{}` is named twice, by `{}` and by `{}`; each rows file \
                         is named once, so that each of its rows is one sheet",
                        later.written, first.field, later.field
                    ),
                )
                .at_field(&later.field)
                .at_position(later.line, later.column));
            }
        }
        named
    } else {
        let written = relative_path(
            fields.require_text("rows")?,
            "rows",
            &RelativePath {
                shape: "the path of the file the set's rows are in, such as `rows.yaml`",
                whose: "the rows file's path",
                from: "the drawing set's folder",
                names: "a file",
            },
        )?;
        vec![rows_named(written, "rows".to_owned(), given.at)]
    };
    let clean = match fields.get("clean") {
        Some(node) => super::boolean(node, "clean")?,
        None => false,
    };
    let plot_style = match fields.get("plot_style") {
        Some(node) => Some(super::page_setup::plot_style_file(node, "plot_style")?),
        None => None,
    };
    let format = match fields.get("format") {
        Some(node) => {
            let text = scalar(
                node,
                "format",
                "a drawing format: `dwg`, `dxf` (ASCII DXF) or `dxf_binary`",
            )?;
            DrawingFormat::from_name(text).ok_or_else(|| {
                bad_value(
                    node,
                    "format",
                    format!("unknown drawing format `{text}`; choose `dwg`, `dxf` or `dxf_binary`"),
                )
            })?
        }
        None => DrawingFormat::Dwg,
    };
    Ok(SetSettings {
        units,
        output: output.to_owned(),
        rows,
        rows_listed,
        clean,
        plot_style,
        format,
    })
}

fn rows_named(written: &str, field: String, at: Pos) -> RowsNamed {
    RowsNamed {
        written: written.to_owned(),
        field,
        line: at.line,
        column: at.column,
    }
}

/// The extension of the rows files a folder `rows` names gives.
const ROWS_FILE_EXTENSION: &str = ".yaml";

/// A rows file the settings of a drawing set reach.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowsPath {
    /// Its path relative to the set's folder, as the settings reach it,
    /// with `/` between folders, such as `rows/areas.yaml`.
    pub written: String,
    /// Its path joined onto the set's folder.
    pub path: PathBuf,
}

/// Reads the rows files of the drawing set whose folder is `folder` and
/// whose settings are `settings`, as [`rows_paths`] finds them, in the
/// order the set builds them.
pub(crate) fn read_rows_files_in(
    files: &dyn ReadFiles,
    folder: &Path,
    settings: &SetSettings,
) -> Result<Vec<RowsFile>, Error> {
    rows_paths(folder, settings)?
        .into_iter()
        .map(|found| read_rows_file_in(files, found))
        .collect()
}

/// Reads the rows file `found`, from `files`.
pub(crate) fn read_rows_file_in(files: &dyn ReadFiles, found: RowsPath) -> Result<RowsFile, Error> {
    let text = read_file_in(files, &found.path, "the drawing set's rows")?;
    Ok(RowsFile {
        written: found.written,
        path: found.path,
        text,
    })
}

/// The rows files of the drawing set whose folder is `folder` and whose
/// settings are `settings`, in the order the set builds them, without
/// reading them: each path `rows` names in turn, a rows file itself or a
/// folder's `.yaml` files, not those of the folders in it, in the byte
/// order of their names, so that every system lists them alike.
///
/// Refused, at the path in the settings: a folder that holds no rows file;
/// a file reached twice, by two paths or a path and a folder; and a list's
/// path, a folder or a folder's file that leads outside the set's folder,
/// as written or as the file system resolves it, or through a link, the
/// file itself or a folder on its way in the set. One path that names a
/// file, as settings that name one rows file write it, is read wherever it
/// leads, as it always has been.
pub(crate) fn rows_paths(folder: &Path, settings: &SetSettings) -> Result<Vec<RowsPath>, Error> {
    let settings_file = folder.join(SET_SETTINGS_FILE);
    // Each file reached, with the path in the settings that reached it.
    let mut reached: Vec<(String, PathBuf, &RowsNamed)> = Vec::new();
    for named in &settings.rows {
        let place = |error: Error| {
            error
                .at_field(&named.field)
                .at_position(named.line, named.column)
                .in_file(&settings_file)
        };
        let path = folder.join(&named.written);
        let is_folder = fs::metadata(&path).is_ok_and(|m| m.is_dir());
        // A list's paths and a folder are plain files and folders in the
        // set; one path that names a file is read as it always has been.
        let kept_in_set = is_folder || settings.rows_listed;
        let outside = relative_to(folder, &path).split('/').next() == Some("..");
        if outside && kept_in_set {
            return Err(place(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`{}` is outside the drawing set's folder; the rows files a list or a folder \
                     names are in the set's folder",
                    named.written
                ),
            )));
        }
        if kept_in_set {
            plain_in_set(folder, &named.written).map_err(place)?;
        }
        let members = if is_folder {
            let members = folder_members(&path, &named.written).map_err(|e| {
                place(Error::new(
                    ErrorCode::ReadFailed,
                    format!("could not list the folder `{}`: {e}", named.written),
                ))
            })?;
            if members.is_empty() {
                return Err(place(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "the folder `{}` holds no rows file: a folder gives the set the \
                         `{ROWS_FILE_EXTENSION}` files in it, not those in its folders, and it \
                         has none",
                        named.written
                    ),
                )));
            }
            for (written, _) in &members {
                plain_in_set(folder, written).map_err(place)?;
            }
            members
        } else {
            vec![(named.written.clone(), path)]
        };
        for (written, path) in members {
            let plain = relative_to(folder, &path);
            let first = reached
                .iter()
                .find(|(_, p, _)| relative_to(folder, p) == plain || is_same_file(p, &path));
            if let Some((first_written, _, by)) = first {
                let how = |n: &RowsNamed, w: &str| {
                    if n.written == w {
                        format!("`{}`", n.field)
                    } else {
                        format!("`{}`, the folder `{}`", n.field, n.written)
                    }
                };
                return Err(place(Error::new(
                    ErrorCode::DuplicateName,
                    format!(
                        "the rows file `{written}` is reached twice, by {} and by {}; each rows \
                         file is named once, so that each of its rows is one sheet",
                        how(by, first_written),
                        how(named, &written),
                    ),
                )));
            }
            reached.push((written, path, named));
        }
    }
    Ok(reached
        .into_iter()
        .map(|(written, path, _)| RowsPath { written, path })
        .collect())
}

/// Refuses the rows file `written`, relative to the set's `folder`, that a
/// list or a folder reaches, when the file itself or a folder on its way in
/// the set is a link, a symbolic link or a junction, or when it leads
/// outside the set's folder as the file system resolves it: a change set
/// writes a rows file where it is, so it is a plain file in the set.
fn plain_in_set(folder: &Path, written: &str) -> Result<(), Error> {
    let mut at = folder.to_owned();
    for part in written.split('/') {
        at.push(part);
        if part == "." || part == ".." {
            continue;
        }
        if fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`{written}` leads through a link, `{}`; a rows file a list or a folder names \
                     is a plain file in the drawing set's folder, reached through plain folders, \
                     so that a change set writes it where it is",
                    relative_to(folder, &at)
                ),
            ));
        }
    }
    if let (Ok(set), Ok(file)) = (fs::canonicalize(folder), fs::canonicalize(&at)) {
        if !file.starts_with(&set) {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "`{written}` is outside the drawing set's folder as the file system resolves \
                     it; the rows files a list or a folder names are in the set's folder"
                ),
            ));
        }
    }
    Ok(())
}

/// The rows files of the folder at `path`, which the settings write as
/// `written`: its files whose names end in `.yaml`, in the byte order of
/// their names, each with its path in the set and joined onto the set's
/// folder. A name that is not Unicode is no rows file's.
fn folder_members(path: &Path, written: &str) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut names: Vec<String> = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let is_file = fs::metadata(entry.path()).is_ok_and(|m| m.is_file());
        if is_file && name.len() > ROWS_FILE_EXTENSION.len() && name.ends_with(ROWS_FILE_EXTENSION)
        {
            names.push(name);
        }
    }
    names.sort_unstable();
    Ok(names
        .into_iter()
        .map(|name| (format!("{written}/{name}"), path.join(&name)))
        .collect())
}

/// Reads the rows of the drawing set whose folder is `folder` and whose
/// settings are `settings`: the rows of each of its rows files in turn.
pub(crate) fn read_set_rows(folder: &Path, settings: &SetSettings) -> Result<Vec<Row>, Error> {
    read_set_rows_in(&Disk, folder, settings)
}

/// Reads the rows of a drawing set as [`read_set_rows`] does, from `files`.
pub(crate) fn read_set_rows_in(
    files: &dyn ReadFiles,
    folder: &Path,
    settings: &SetSettings,
) -> Result<Vec<Row>, Error> {
    rows_of(&read_rows_files_in(files, folder, settings)?)
}

/// The rows of a drawing set's rows files, read as [`read_rows_files_in`]
/// reads them, each file's in turn: the set's rows in the order it builds
/// them. A sheet is named once in the set, across its rows files as in one.
pub(crate) fn rows_of(files: &[RowsFile]) -> Result<Vec<Row>, Error> {
    let written: Vec<&str> = files.iter().map(|f| f.written.as_str()).collect();
    let mut sheets: Vec<SheetNamed> = Vec::new();
    let mut rows = Vec::new();
    for (f, file) in files.iter().enumerate() {
        let read = parse_rows_of(&file.text, &file.path, f, &written, &mut sheets)
            .map_err(|e| e.or_in_file(&file.path))?;
        rows.extend(read);
    }
    Ok(rows)
}

/// Reads the text of a drawing set's rows file.
#[cfg(test)]
pub(super) fn parse_rows(text: &str) -> Result<Vec<Row>, Error> {
    parse_rows_of(text, Path::new(""), 0, &[""], &mut Vec::new())
}

/// A sheet's name given in a set's rows, folded to lower case, with where
/// it is given: the place of its rows file, its row and its place there.
struct SheetNamed {
    folded: String,
    file: usize,
    row: usize,
    at: Pos,
}

/// Reads the text of the rows file at `path`, the set's rows file `f`, of
/// those the set writes as `written`. `sheets` holds the sheets' names the
/// set's rows files before it give.
fn parse_rows_of(
    text: &str,
    path: &Path,
    f: usize,
    written: &[&str],
    sheets: &mut Vec<SheetNamed>,
) -> Result<Vec<Row>, Error> {
    let Some(root) = strict_yaml::read(text)? else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            "the rows file is empty; it needs `rows`",
        ));
    };
    let fields = Fields::of(&root, "", &ROWS_FILE_KEYS, "a rows file")?;
    let items = sequence(fields.require("rows")?, "rows", "a list of rows")?;
    let place = RowPlace { path, f, written };
    items
        .iter()
        .enumerate()
        .map(|(index, node)| row(node, index, &place, sheets))
        .collect()
}

/// The rows file a row is read from.
struct RowPlace<'a> {
    path: &'a Path,
    f: usize,
    /// The paths of the set's rows files, as the set writes them.
    written: &'a [&'a str],
}
fn row(
    node: &Node,
    index: usize,
    place: &RowPlace,
    sheets: &mut Vec<SheetNamed>,
) -> Result<Row, Error> {
    let path = index_path("rows", index);
    let fields = Fields::of(node, &path, &ROW_KEYS, "a row")?;
    let sheet = sheet_name(
        fields.require_text("sheet")?,
        &fields.path_of("sheet"),
        index,
        place,
        sheets,
    )?;
    let template = relative_path(
        fields.require_text("template")?,
        &fields.path_of("template"),
        &RelativePath {
            shape: "the path of the sheet's template, such as `templates/border.yaml`",
            whose: "the template's path",
            from: "the drawing set's folder",
            names: "a file",
        },
    )?;
    let (values, at) = match fields.get("values") {
        Some(values) => (row_values(values, &fields.path_of("values"))?, values.at),
        None => (Vec::new(), node.at),
    };
    let objects = match fields.get(ROW_OBJECTS_KEY) {
        Some(list) => sequence(
            list,
            &fields.path_of(ROW_OBJECTS_KEY),
            "a list of objects, which are on this sheet alone",
        )?
        .to_vec(),
        None => Vec::new(),
    };
    Ok(Row {
        sheet,
        template: template.to_owned(),
        file: place.path.to_owned(),
        file_index: place.f,
        index,
        at,
        values,
        objects,
    })
}

fn row_values(node: &Node, path: &str) -> Result<Vec<RowValue>, Error> {
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(
            node,
            path,
            "a mapping from the names of values to their text",
        ));
    };
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        let value_path = key_path(path, &entry.key);
        if let Some(problem) = value_name_problem(&entry.key) {
            return Err(Error::new(ErrorCode::BadValue, problem)
                .at_field(&value_path)
                .at_position(entry.key_at.line, entry.key_at.column));
        }
        // A bare null leaves the value out: the row does not give it.
        if entry.value.is_bare_null() {
            continue;
        }
        let text = scalar(&entry.value, &value_path, "the value's text")?;
        values.push(RowValue {
            name: entry.key.clone(),
            text: text.to_owned(),
            key_at: entry.key_at,
            at: entry.value.at,
        });
    }
    Ok(values)
}

/// Why `name` cannot name a value of a row, if it cannot: a name is one or
/// more ASCII letters, digits, `_` and `-`.
impl Row {
    /// This row with the values `names` as `other` gives them, and its own
    /// otherwise: a row as it would be had only some of a change set's
    /// edits to its values been made.
    pub(crate) fn with_values_of(&self, other: &Row, names: &[String]) -> Row {
        let mut row = self.clone();
        for value in &mut row.values {
            if names.contains(&value.name) {
                if let Some(theirs) = other.values.iter().find(|v| v.name == value.name) {
                    value.text = theirs.text.clone();
                }
            }
        }
        row
    }
}

pub(crate) fn value_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("the name of a value cannot be empty".to_owned());
    }
    name.chars()
        .find(|&c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .map(|c| {
            format!(
                "the name of a value is ASCII letters, digits, `_` and `-`, so it cannot hold `{}`",
                c.escape_default()
            )
        })
}

/// A sheet's name, which names its DWG and no other sheet's in the set,
/// ignoring case. `sheets` holds the names given before it, in its rows
/// file and the set's rows files before it, each with where it was given.
fn sheet_name(
    node: &Node,
    path: &str,
    row: usize,
    place: &RowPlace,
    sheets: &mut Vec<SheetNamed>,
) -> Result<String, Error> {
    let name = scalar(node, path, "the sheet's name, such as `DEMO-001`")?;
    if let Some(problem) = sheet_name_problem(name) {
        return Err(bad_value(node, path, problem));
    }
    let folded = name.to_lowercase();
    if let Some(first) = sheets.iter().find(|n| n.folded == folded) {
        // Given before in this rows file, or in another of the set's.
        let given = if first.file == place.f {
            format!("at line {}", first.at.line)
        } else {
            format!(
                "in the rows file `{}`, at line {}, `{}`,",
                place.written[first.file],
                first.at.line,
                index_path("rows", first.row)
            )
        };
        return Err(at(
            Error::new(
                ErrorCode::DuplicateName,
                format!(
                    "the sheet `{name}` is named {given} already; a sheet's name names its DWG, so each is given once in a set, and names that differ only in case are one name"
                ),
            ),
            path,
            node.at,
        ));
    }
    sheets.push(SheetNamed {
        folded,
        file: place.f,
        row,
        at: node.at,
    });
    Ok(name.to_owned())
}

/// Why `name` cannot name a sheet, if it cannot. A sheet's name, with
/// `.dwg` after it, is its DWG's file name, so it is one Windows, Linux and
/// macOS all allow.
fn sheet_name_problem(name: &str) -> Option<String> {
    let device = name.split('.').next().unwrap_or(name);
    if name.is_empty() {
        Some("a sheet's name cannot be empty".to_owned())
    } else if name.chars().count() > LONGEST_SHEET_NAME {
        Some(format!(
            "a sheet's name has at most {LONGEST_SHEET_NAME} characters, so that its DWG's file name has at most 255"
        ))
    } else if name.trim() != name {
        Some("a sheet's name cannot start or end with a space".to_owned())
    } else if name.ends_with('.') {
        Some("a sheet's name cannot end with `.`, which Windows drops from a file name".to_owned())
    } else if let Some(c) = name
        .chars()
        .find(|&c| FORBIDDEN_IN_SHEET_NAMES.contains(c) || c.is_control())
    {
        Some(format!(
            "a sheet's name names its DWG, so it cannot hold `{}`: it holds none of {FORBIDDEN_IN_SHEET_NAMES} and no control characters",
            c.escape_default()
        ))
    } else if DEVICE_NAMES.iter().any(|d| d.eq_ignore_ascii_case(device)) {
        Some(format!(
            "`{device}` is a name Windows keeps for a device, so no file can be called `{name}.dwg` there; Windows keeps {}",
            DEVICE_NAMES.join(", ")
        ))
    } else {
        None
    }
}
