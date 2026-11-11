//! Reads a drawing set's settings and its rows.
//!
//! A drawing set is a folder. Its settings, in `set.yaml` at the top of the
//! folder, declare the set's unit, the folder its DWGs go to and the file
//! its rows are in. Each row names one sheet, the template it is drawn from
//! and the values the template takes from it, and may hold objects of its
//! own, which are on that sheet alone. Both files are written in the same
//! strict YAML as a sheet description, and `docs/drawing-set.md` sets them
//! out.

use std::path::{Path, PathBuf};

use super::blocks::{relative_path, RelativePath};
use super::strict_yaml::{self, index_path, key_path, Kind, Node, Pos};
use super::{at, bad_value, read_file, scalar, sequence, unit, wrong_type, Fields};
use crate::error::{Error, ErrorCode};
use crate::units::Unit;

/// The keys of a drawing set's settings.
pub(crate) const SETTINGS_KEYS: [&str; 3] = ["units", "output", "rows"];

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
    /// The file the set's rows are in, relative to the set's folder, as the
    /// settings write it.
    pub rows: String,
}

/// One row of a drawing set: one sheet, the template it is drawn from and
/// the values the template takes from it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// The sheet's name, which names its DWG.
    pub sheet: String,
    /// The template, relative to the set's folder, as the row writes it.
    pub template: String,
    /// Where the row is in the rows file, counted from 0.
    pub(super) index: usize,
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
    let text = read_file(path, "the drawing set's settings")?;
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
        fields.require("output")?,
        "output",
        &RelativePath {
            shape: "the folder the set's DWGs go to, such as `dwg`",
            whose: "the output folder's path",
            from: "the drawing set's folder",
            names: "a folder",
        },
    )?;
    let rows = relative_path(
        fields.require("rows")?,
        "rows",
        &RelativePath {
            shape: "the path of the file the set's rows are in, such as `rows.yaml`",
            whose: "the rows file's path",
            from: "the drawing set's folder",
            names: "a file",
        },
    )?;
    Ok(SetSettings {
        units,
        output: output.to_owned(),
        rows: rows.to_owned(),
    })
}

/// Reads the rows in the file at `path`.
pub(crate) fn read_rows(path: &Path) -> Result<Vec<Row>, Error> {
    let text = read_file(path, "the drawing set's rows")?;
    parse_rows(&text).map_err(|e| e.or_in_file(path))
}

/// Reads `text`, the rows in the file at `path`, as [`read_rows`] reads
/// them, from bytes the caller has read, so that an output can be stamped
/// with them.
pub(crate) fn rows_at(text: &str, path: &Path) -> Result<Vec<Row>, Error> {
    parse_rows(text).map_err(|e| e.or_in_file(path))
}

/// Reads the text of a drawing set's rows file.
pub(super) fn parse_rows(text: &str) -> Result<Vec<Row>, Error> {
    let Some(root) = strict_yaml::read(text)? else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            "the rows file is empty; it needs `rows`",
        ));
    };
    let fields = Fields::of(&root, "", &ROWS_FILE_KEYS, "a rows file")?;
    let items = sequence(fields.require("rows")?, "rows", "a list of rows")?;
    let mut sheets: Vec<(String, Pos)> = Vec::with_capacity(items.len());
    items
        .iter()
        .enumerate()
        .map(|(index, node)| row(node, index, &mut sheets))
        .collect()
}

fn row(node: &Node, index: usize, sheets: &mut Vec<(String, Pos)>) -> Result<Row, Error> {
    let path = index_path("rows", index);
    let fields = Fields::of(node, &path, &ROW_KEYS, "a row")?;
    let sheet = sheet_name(fields.require("sheet")?, &fields.path_of("sheet"), sheets)?;
    let template = relative_path(
        fields.require("template")?,
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
    entries
        .iter()
        .map(|entry| {
            let value_path = key_path(path, &entry.key);
            if let Some(problem) = value_name_problem(&entry.key) {
                return Err(Error::new(ErrorCode::BadValue, problem)
                    .at_field(&value_path)
                    .at_position(entry.key_at.line, entry.key_at.column));
            }
            let text = scalar(&entry.value, &value_path, "the value's text")?;
            Ok(RowValue {
                name: entry.key.clone(),
                text: text.to_owned(),
                key_at: entry.key_at,
                at: entry.value.at,
            })
        })
        .collect()
}

/// Why `name` cannot name a value of a row, if it cannot: a name is one or
/// more ASCII letters, digits, `_` and `-`.
pub(super) fn value_name_problem(name: &str) -> Option<String> {
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

/// A sheet's name, which names its DWG and no other sheet's, ignoring case.
/// `sheets` holds the names given before it, folded to lower case, each
/// with where it was given.
fn sheet_name(node: &Node, path: &str, sheets: &mut Vec<(String, Pos)>) -> Result<String, Error> {
    let name = scalar(node, path, "the sheet's name, such as `DEMO-001`")?;
    if let Some(problem) = sheet_name_problem(name) {
        return Err(bad_value(node, path, problem));
    }
    let folded = name.to_lowercase();
    if let Some((_, first)) = sheets.iter().find(|(n, _)| *n == folded) {
        return Err(at(
            Error::new(
                ErrorCode::DuplicateName,
                format!(
                    "the sheet `{name}` is named at line {} already; a sheet's name names its DWG, so each is given once in a set, and names that differ only in case are one name",
                    first.line
                ),
            ),
            path,
            node.at,
        ));
    }
    sheets.push((folded, node.at));
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
