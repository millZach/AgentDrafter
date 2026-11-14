//! Reads a sheet description into a [`Sheet`], and the files of a drawing
//! set: its settings, its rows and its templates.
//!
//! A sheet description is a strict subset of YAML, written down in
//! `docs/sheet-description.md`. [`strict_yaml`] reads the subset into a tree
//! of text; this module gives the text its meaning and refuses anything the
//! sheet description does not know. Every error names the field it is about,
//! and an error about a choice lists the valid choices.
//!
//! [`layers`] reads linetypes and layers, [`objects`] reads the objects,
//! [`hatch`] reads hatch patterns and hatches, [`text`] reads text styles
//! and text, [`table`] reads table styles and tables, [`blocks`] reads
//! block libraries, their blocks and inserts of them, [`ids`] reads each object's ID, [`records`] gives a sheet with its
//! objects' IDs, and [`schema`] states the same keys as a JSON Schema for
//! agents to read. [`set`] reads a drawing set's settings and rows, [`template`] fills
//! a template in from a row, and [`set_schema`] states their schemas.

mod block_schema;
pub(crate) mod blocks;
pub(crate) mod edit;
mod emit;
mod hatch;
mod hatch_schema;
pub(crate) mod ids;
mod layers;
pub(crate) use layers::{one_name, LAYER_KEYS, LINETYPE_KEYS};
mod mark;
mod objects;
pub(crate) mod paper;
mod polyline;
mod records;
pub(crate) mod schema;
mod set;
mod set_schema;
mod strict_yaml;
mod table;
mod table_schema;
mod template;
mod text;
pub(crate) use text::TEXT_STYLE_KEYS;

use std::fs;
use std::path::{Path, PathBuf};

use ids::ObjectRecord;
use objects::{Place, Tables};
use strict_yaml::{index_path, key_path, Entry, Kind, Node, Pos};

use crate::error::{Error, ErrorCode};
use crate::model::{BlockLibrary, Point3, Sheet};
use crate::units::{parse_length, LengthError, Unit};

pub use blocks::parse_block_library;
pub(crate) use edit::{AttributeSet, Axis, GivenValue, Listed, ObjectList, RecordFile, TableEdit};
pub(crate) use emit::number_text;
pub(crate) use ids::ID_SEPARATOR;
pub(crate) use mark::{mark_text, parse_mark, MarkFailure};
pub(crate) use objects::article;
pub(crate) use records::{read_sheet_record, read_sheet_record_noting, SavedSet, SheetRecord};
pub(crate) use set::{
    read_rows, read_rows_in, read_set_settings, read_set_settings_in, rows_at, template_path, Row,
};
pub(crate) use set_schema::{rows_schema, set_schema, template_schema};
pub(crate) use table::CELL_KEYS;
pub(crate) use template::{
    library_named, read_template, read_template_in, template_at, template_in,
};

/// The keys at the top of a sheet description.
pub(crate) const SHEET_KEYS: [&str; 10] = [
    "units",
    "block_library",
    "ltscale",
    "linetypes",
    "layers",
    "hatch_patterns",
    "text_styles",
    "table_styles",
    "objects",
    paper::PAPER_KEY,
];

/// The keys an object of type `object` takes on a sheet: its own, then its
/// ID and its name. Empty for a type a sheet does not hold.
pub(crate) fn objects_table(object: &str) -> Vec<&'static str> {
    objects::OBJECT_TYPES
        .iter()
        .find(|(name, _)| *name == object)
        .map(|(_, keys)| {
            keys.iter()
                .chain(&ids::SHEET_OBJECT_KEYS)
                .copied()
                .collect()
        })
        .unwrap_or_default()
}

/// Every object type a sheet holds, in the order the schema lists them.
#[cfg(test)]
pub(crate) fn object_types() -> Vec<&'static str> {
    objects::OBJECT_TYPES
        .iter()
        .map(|(name, _)| *name)
        .collect()
}

/// Reads the text of a sheet description into a [`Sheet`].
///
/// Every length comes back in the sheet's declared unit; a number that
/// carries its own unit is converted to it. A sheet description that names
/// a block library is read with [`read_sheet_description`], which finds the
/// library beside it.
pub fn parse_sheet_description(text: &str) -> Result<Sheet, Error> {
    on_its_own(text, &mut no_library)
}

/// Reads the sheet description in the file at `path` into a [`Sheet`],
/// with the block library it names, if it names one. The library's path is
/// relative to the sheet description's folder.
///
/// An error in the block library names the library's file; any other names
/// the sheet description's.
pub fn read_sheet_description(path: &Path) -> Result<Sheet, Error> {
    let text = read_file(path, "the sheet description")?;
    sheet_description_at(&text, path).map(|(sheet, _)| sheet)
}

/// A block library's file as a sheet description's reading read it.
pub(crate) struct LibraryFile {
    /// Where it was read from.
    pub path: PathBuf,
    /// Its bytes.
    pub bytes: Vec<u8>,
    /// What reading it noticed, each placed in it.
    pub noticed: Vec<blocks::Noticed>,
}

/// Reads `text`, the sheet description in the file at `path`, as
/// [`read_sheet_description`] does, with the block library it names, and
/// gives the library's file as it was read, if it names one.
pub(crate) fn sheet_description_at(
    text: &str,
    path: &Path,
) -> Result<(Sheet, Option<LibraryFile>), Error> {
    let mut library = None;
    let sheet = on_its_own(text, &mut library_beside(path, &mut library))
        .map_err(|e| e.or_in_file(path))?;
    Ok((sheet, library))
}

/// Reads the text of a sheet description built on its own, which takes no
/// value from a row, with `load` reading the block library it names.
fn on_its_own(
    text: &str,
    load: &mut dyn FnMut(&str) -> Result<BlockLibrary, Error>,
) -> Result<Sheet, Error> {
    on_its_own_with_records(text, load, &mut Vec::new()).map(|(sheet, _)| sheet)
}

/// Reads a sheet description built on its own as [`on_its_own`] does, with
/// the record of each of its objects, adding what reading its objects
/// noticed to `noticed`.
fn on_its_own_with_records(
    text: &str,
    load: &mut dyn FnMut(&str) -> Result<BlockLibrary, Error>,
    noticed: &mut Vec<blocks::Noticed>,
) -> Result<(Sheet, Vec<ObjectRecord>), Error> {
    let mut root = strict_yaml::read(text)?;
    template::refuse_row_values(root.as_mut())?;
    sheet_and_records(root, load, None, None, noticed)
}

/// A row's own objects, which a sheet of a drawing set holds after its
/// template's: where they are in the rows file, and the file.
struct RowObjects<'a> {
    pub(super) items: &'a [Node],
    /// Their path in the rows file, such as `rows[1].objects`.
    pub(super) path: String,
    pub(super) file: &'a Path,
}

/// Where a drawing set's files are read from: the disk, or, for the set as
/// it was at an entry of its journal, the files as they were then.
pub(crate) trait ReadFiles {
    /// The text of the file at `path`.
    fn text(&self, path: &Path) -> std::io::Result<String>;
}

/// The files as they are on disk.
pub(crate) struct Disk;

impl ReadFiles for Disk {
    fn text(&self, path: &Path) -> std::io::Result<String> {
        fs::read_to_string(path)
    }
}

/// The text of the file at `path`. `what` names it in the error, such as
/// "the sheet description".
pub(crate) fn read_file(path: &Path, what: &str) -> Result<String, Error> {
    read_file_in(&Disk, path, what)
}

/// The text of the file at `path`, read from `files`.
pub(crate) fn read_file_in(
    files: &dyn ReadFiles,
    path: &Path,
    what: &str,
) -> Result<String, Error> {
    files.text(path).map_err(|e| {
        Error::new(ErrorCode::ReadFailed, format!("could not read {what}: {e}")).in_file(path)
    })
}

/// A loader for a sheet description that names no block library.
fn no_library(_: &str) -> Result<BlockLibrary, Error> {
    Err(Error::usage(
        "this sheet description names a block library, which is found from the sheet \
         description's folder; read it from its file",
    ))
}

/// A loader that reads the block library a sheet description or template
/// at `path` names, from the path it writes relative to its folder, and
/// keeps the library's file as it was read in `read`.
fn library_beside<'a>(
    path: &'a Path,
    read: &'a mut Option<LibraryFile>,
) -> impl FnMut(&str) -> Result<BlockLibrary, Error> + 'a {
    library_beside_in(&Disk, path, read)
}

/// A loader as [`library_beside`] gives, that reads from `files`.
fn library_beside_in<'a>(
    files: &'a dyn ReadFiles,
    path: &'a Path,
    read: &'a mut Option<LibraryFile>,
) -> impl FnMut(&str) -> Result<BlockLibrary, Error> + 'a {
    let folder = path.parent().unwrap_or(Path::new(""));
    move |written| {
        let file = folder.join(written);
        let text = files.text(&file).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "could not read the block library `{written}`, looked for at `{}`: {e}",
                    file.display()
                ),
            )
        })?;
        let (parsed, mut noticed) =
            blocks::read_block_library(&text).map_err(|e| e.in_file(&file))?;
        for n in &mut noticed {
            n.file = Some(file.clone());
        }
        *read = Some(LibraryFile {
            path: file,
            bytes: text.into_bytes(),
            noticed,
        });
        Ok(parsed)
    }
}

/// Reads a sheet as [`sheet_and_records`] does, without its objects' records.
#[cfg(test)]
fn sheet(
    root: Option<Node>,
    load: &mut dyn FnMut(&str) -> Result<BlockLibrary, Error>,
    set_units: Option<Unit>,
) -> Result<Sheet, Error> {
    sheet_and_records(root, load, set_units, None, &mut Vec::new()).map(|(sheet, _)| sheet)
}

/// Reads a sheet description, or a template once a row has filled it in,
/// from its tree, with the record of each of its objects, in the same
/// order: where it is written and the ID it carries there. `load` reads the
/// block library at the path the sheet writes, if it names one: an error it
/// returns about a file is the library's own, and one about no file is
/// placed at `block_library`. `set_units` is the declared unit of the
/// drawing set the sheet is built in, if it is built in one: the sheet must
/// be in it too. `row_objects` are the sheet's row's own objects, if it has
/// any: they follow the template's, name what the template defines, and an
/// error in one is placed in the rows file. What reading the objects
/// noticed, such as an attribute tag written in lowercase, is added to
/// `noticed`; what reading the block library noticed is kept with its file.
fn sheet_and_records(
    root: Option<Node>,
    load: &mut dyn FnMut(&str) -> Result<BlockLibrary, Error>,
    set_units: Option<Unit>,
    row_objects: Option<RowObjects>,
    noticed: &mut Vec<blocks::Noticed>,
) -> Result<(Sheet, Vec<ObjectRecord>), Error> {
    let Some(root) = root else {
        return Err(Error::new(
            ErrorCode::MissingKey,
            "the sheet description is empty; it needs `units` and `objects`",
        ));
    };
    let fields = Fields::of(&root, "", &SHEET_KEYS, "a sheet description")?;
    let units = match fields.get("units") {
        Some(node) => unit(node, "units")?,
        None => {
            return Err(Error::new(
                ErrorCode::MissingUnit,
                format!(
                    "the sheet declares no unit; add `units:` with one of {}",
                    Unit::symbols()
                ),
            )
            .at_field("units")
            .at_position(root.at.line, root.at.column))
        }
    };
    if let Some(set_units) = set_units.filter(|&set| set != units) {
        return Err(bad_value(
            fields.require("units")?,
            "units",
            format!(
                "the template is in {units} and the drawing set in {set_units}; every template, \
                 sheet description and block library in a set is in the set's declared unit"
            ),
        ));
    }
    let (block_library, library) = match fields.text("block_library") {
        Some(node) => {
            let path = "block_library";
            let written = blocks::library_path(node, path)?;
            let library = load(written).map_err(|e| {
                if e.file.is_some() {
                    e
                } else {
                    at(e, path, node.at)
                }
            })?;
            if library.units != units {
                return Err(bad_value(
                    node,
                    path,
                    format!(
                        "the block library is in {} and the sheet in {units}; a sheet draws on a block library in its own declared unit",
                        library.units
                    ),
                ));
            }
            (Some(written.to_owned()), Some(library))
        }
        None => (None, None),
    };
    let linetypes = match fields.get("linetypes") {
        Some(node) => layers::linetypes(node, units)?,
        None => Vec::new(),
    };
    let layers = match fields.get("layers") {
        Some(node) => layers::layers(node, &linetypes)?,
        None => Vec::new(),
    };
    let hatch_patterns = match fields.get("hatch_patterns") {
        Some(node) => hatch::hatch_patterns(node, units)?,
        None => Vec::new(),
    };
    let text_styles = match fields.get("text_styles") {
        Some(node) => text::text_styles(node)?,
        None => Vec::new(),
    };
    let table_styles = match fields.get("table_styles") {
        Some(node) => table::table_styles(node, units, &text_styles)?,
        None => Vec::new(),
    };
    let ltscale = match fields.get("ltscale") {
        Some(node) => layers::linetype_scale(node, "ltscale")?,
        None => 1.0,
    };
    let paper = match fields.get(paper::PAPER_KEY) {
        Some(node) => Some(paper::paper(node, paper::PAPER_KEY, units)?),
        None => None,
    };
    let list = sequence(fields.require("objects")?, "objects", "a list of objects")?;
    let tables = Tables {
        units,
        linetypes: &linetypes,
        layers: &layers,
        hatch_patterns: &hatch_patterns,
        styles: &text_styles,
        table_styles: &table_styles,
    };
    let mut place = Place::Sheet {
        library: library.as_ref(),
        objects: Vec::new(),
        noticed: Vec::new(),
    };
    let mut objects = list
        .iter()
        .enumerate()
        .map(|(i, node)| objects::object(node, &index_path("objects", i), &tables, &mut place))
        .collect::<Result<Vec<_>, _>>()?;
    let in_template = objects.len();
    let Place::Sheet {
        noticed: template_noticed,
        ..
    } = &mut place
    else {
        unreachable!("a sheet's objects are read on a sheet")
    };
    noticed.append(template_noticed);
    let row_objects_file = row_objects.as_ref().map(|row| row.file.to_owned());
    if let Some(row) = row_objects {
        for (j, node) in row.items.iter().enumerate() {
            let path = index_path(&row.path, j);
            template::refuse_row_values_in(node, &path).map_err(|e| e.in_file(row.file))?;
            let object = objects::object(node, &path, &tables, &mut place)
                .map_err(|e| e.or_in_file(row.file))?;
            objects.push(object);
        }
    }
    let Place::Sheet {
        objects: mut records,
        noticed: in_rows,
        ..
    } = place
    else {
        unreachable!("a sheet's objects are read on a sheet")
    };
    noticed.extend(in_rows.into_iter().map(|n| blocks::Noticed {
        file: row_objects_file.clone(),
        ..n
    }));
    for record in &mut records[in_template..] {
        record.in_row = true;
    }
    let mut sheet = Sheet {
        units,
        block_library,
        linetypes,
        layers,
        text_styles,
        table_styles,
        blocks: Vec::new(),
        objects,
        paper,
        ltscale,
    };
    if let Some(library) = &library {
        blocks::take_from_library(&mut sheet, library, &fields)?;
    }
    Ok((sheet, records))
}

fn point(node: &Node, path: &str, units: Unit) -> Result<Point3, Error> {
    const SHAPE: &str = "a point, a list of three lengths [x, y, z]";
    let items = sequence(node, path, SHAPE)?;
    let [x, y, z] = items else {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                format!(
                    "a point has three coordinates [x, y, z]; this one has {}",
                    items.len()
                ),
            ),
            path,
            node.at,
        ));
    };
    Ok(Point3::new(
        length(x, &index_path(path, 0), units)?,
        length(y, &index_path(path, 1), units)?,
        length(z, &index_path(path, 2), units)?,
    ))
}

/// A list of at least `fewest` points.
fn points(node: &Node, path: &str, units: Unit, fewest: usize) -> Result<Vec<Point3>, Error> {
    let items = sequence(node, path, "a list of points")?;
    if items.len() < fewest {
        return Err(at(
            Error::new(
                ErrorCode::WrongType,
                format!("expected at least {fewest} points, found {}", items.len()),
            ),
            path,
            node.at,
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| point(item, &index_path(path, i), units))
        .collect()
}

/// A length in the declared unit. A number that carries its own unit is converted.
fn length(node: &Node, path: &str, declared: Unit) -> Result<f64, Error> {
    const SHAPE: &str = "a length such as `100` or `4 in`";
    let text = scalar(node, path, SHAPE)?;
    let error = match parse_length(text) {
        Ok((value, None)) => return Ok(value),
        Ok((value, Some(own))) => match own.convert(value, declared) {
            converted if converted.is_finite() => return Ok(converted),
            _ => Error::new(
                ErrorCode::BadNumber,
                format!("`{text}` is too large once converted to {declared}"),
            ),
        },
        Err(LengthError::NotANumber) => Error::new(
            ErrorCode::BadNumber,
            format!("expected {SHAPE}, found {}", describe(node)),
        ),
        Err(LengthError::OutOfRange) => {
            Error::new(ErrorCode::BadNumber, format!("`{text}` is too large"))
        }
        Err(LengthError::UnknownUnit(unit)) => unknown_unit(&unit),
    };
    Err(at(error, path, node.at))
}

/// A number with no unit, such as an angle or a ratio. `shape` says what
/// is expected, such as "an angle in degrees, a number such as `90`".
fn plain_number(node: &Node, path: &str, shape: &str) -> Result<f64, Error> {
    let text = scalar(node, path, shape)?;
    let error = match parse_length(text) {
        Ok((value, None)) => return Ok(value),
        Err(LengthError::OutOfRange) => {
            Error::new(ErrorCode::BadNumber, format!("`{text}` is too large"))
        }
        Ok((_, Some(_))) | Err(LengthError::NotANumber | LengthError::UnknownUnit(_)) => {
            Error::new(
                ErrorCode::BadNumber,
                format!("expected {shape}, with no unit, found {}", describe(node)),
            )
        }
    };
    Err(at(error, path, node.at))
}

/// An angle in degrees, anticlockwise, brought into `[0, 360)`.
fn angle(node: &Node, path: &str) -> Result<f64, Error> {
    let degrees = plain_number(node, path, "an angle in degrees, a number such as `90`")?;
    Ok(normalize_degrees(degrees))
}

/// `degrees` brought into `[0, 360)`, so that 360, -90 and 450 read as 0,
/// 270 and 90.
pub(crate) fn normalize_degrees(degrees: f64) -> f64 {
    let turned = degrees.rem_euclid(360.0);
    // Rounding can land a tiny negative angle on 360; adding 0 turns -0 into 0.
    if turned >= 360.0 {
        0.0
    } else {
        turned + 0.0
    }
}

fn boolean(node: &Node, path: &str) -> Result<bool, Error> {
    match scalar(node, path, "`true` or `false`")? {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(wrong_type(node, path, "`true` or `false`")),
    }
}

/// A scalar's text. `expected` names what belongs there, for the error.
fn scalar<'a>(node: &'a Node, path: &str, expected: &str) -> Result<&'a str, Error> {
    match &node.kind {
        Kind::Scalar(text) => Ok(text),
        _ => Err(wrong_type(node, path, expected)),
    }
}

/// A scalar's text where one of a set of choices belongs. A value of
/// another shape is an error that lists `choices`, as the error for a wrong
/// choice does.
fn choice<'a>(node: &'a Node, path: &str, expected: &str, choices: &str) -> Result<&'a str, Error> {
    match &node.kind {
        Kind::Scalar(text) => Ok(text),
        _ => Err(at(
            Error::new(
                ErrorCode::WrongType,
                format!("expected {expected}, found {}; {choices}", describe(node)),
            ),
            path,
            node.at,
        )),
    }
}

fn unit(node: &Node, path: &str) -> Result<Unit, Error> {
    let text = choice(
        node,
        path,
        "a unit",
        &format!("known units: {}", Unit::symbols()),
    )?;
    let error = if text.is_empty() {
        Error::new(
            ErrorCode::MissingUnit,
            format!(
                "the sheet declares no unit; give `units:` one of {}",
                Unit::symbols()
            ),
        )
    } else if let Some(unit) = Unit::from_symbol(text) {
        return Ok(unit);
    } else {
        unknown_unit(text)
    };
    Err(at(error, path, node.at))
}

fn unknown_unit(unit: &str) -> Error {
    Error::new(
        ErrorCode::UnknownUnit,
        format!("unknown unit `{unit}`; known units: {}", Unit::symbols()),
    )
}

fn sequence<'a>(node: &'a Node, path: &str, expected: &str) -> Result<&'a [Node], Error> {
    match &node.kind {
        Kind::Sequence(items) => Ok(items),
        _ => Err(wrong_type(node, path, expected)),
    }
}

/// The keys of one mapping, checked against the keys it may have.
struct Fields<'a> {
    path: &'a str,
    at: Pos,
    entries: &'a [Entry],
}

impl<'a> Fields<'a> {
    /// Reads `node` as a mapping that may hold only the `known` keys.
    /// `what` names the mapping in messages, such as "a line".
    fn of(node: &'a Node, path: &'a str, known: &[&str], what: &str) -> Result<Self, Error> {
        let Kind::Mapping(entries) = &node.kind else {
            return Err(wrong_type(node, path, &format!("{what}, a mapping")));
        };
        if let Some(entry) = entries.iter().find(|e| !known.contains(&e.key.as_str())) {
            return Err(unknown_key(
                entry,
                path,
                &format!("unknown key `{}`", entry.key),
                &format!("{what} takes {}", known.join(", ")),
            ));
        }
        Ok(Self {
            path,
            at: node.at,
            entries,
        })
    }

    fn get(&self, key: &str) -> Option<&'a Node> {
        self.entries.iter().find(|e| e.key == key).map(|e| &e.value)
    }

    fn require(&self, key: &str) -> Result<&'a Node, Error> {
        self.get(key).ok_or_else(|| {
            Error::new(ErrorCode::MissingKey, format!("missing key `{key}`"))
                .at_field(key_path(self.path, key))
                .at_position(self.at.line, self.at.column)
        })
    }

    /// The value of the optional text or path field `key`: `None` when it
    /// is not written or is a bare null ([`Node::is_bare_null`]), which
    /// Zach decided on 2026-11-11 means the field is left out.
    fn text(&self, key: &str) -> Option<&'a Node> {
        self.get(key).filter(|node| !node.is_bare_null())
    }

    /// The value of the required text or path field `key`. Not written, or
    /// a bare null, which means it is left out, it is the `missing_key`
    /// error.
    fn require_text(&self, key: &str) -> Result<&'a Node, Error> {
        let node = self.require(key)?;
        if !node.is_bare_null() {
            return Ok(node);
        }
        let Kind::Scalar(written) = &node.kind else {
            unreachable!("a bare null is a scalar")
        };
        let message = if written.is_empty() {
            format!("`{key}` is left empty, which leaves it out, and it is required")
        } else {
            format!(
                "`{key}` is a bare `{written}`, which leaves it out, and it is required; to \
                 mean the text `{written}`, quote it: `'{written}'`"
            )
        };
        Err(at(
            Error::new(ErrorCode::MissingKey, message),
            &self.path_of(key),
            node.at,
        ))
    }

    /// Where `key` itself is written, if it is.
    fn key_at(&self, key: &str) -> Option<Pos> {
        self.entries.iter().find(|e| e.key == key).map(|e| e.key_at)
    }

    /// The path of the value under `key`.
    fn path_of(&self, key: &str) -> String {
        key_path(self.path, key)
    }
}

fn unknown_key(entry: &Entry, path: &str, what: &str, known: &str) -> Error {
    Error::new(ErrorCode::UnknownKey, format!("{what}; {known}"))
        .at_field(key_path(path, &entry.key))
        .at_position(entry.key_at.line, entry.key_at.column)
}

fn wrong_type(node: &Node, path: &str, expected: &str) -> Error {
    at(
        Error::new(
            ErrorCode::WrongType,
            format!("expected {expected}, found {}", describe(node)),
        ),
        path,
        node.at,
    )
}

/// A value of the right shape that is not allowed.
fn bad_value(node: &Node, path: &str, message: impl Into<String>) -> Error {
    at(Error::new(ErrorCode::BadValue, message), path, node.at)
}

/// Places an error at a field and position. The top level has no field name.
fn at(error: Error, path: &str, pos: Pos) -> Error {
    let error = error.at_position(pos.line, pos.column);
    if path.is_empty() {
        error
    } else {
        error.at_field(path)
    }
}

fn describe(node: &Node) -> String {
    match &node.kind {
        Kind::Scalar(text) if text.is_empty() => "nothing".to_owned(),
        Kind::Scalar(text) => format!("`{text}`"),
        Kind::Sequence(_) => "a list".to_owned(),
        Kind::Mapping(_) => "a mapping".to_owned(),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod geometry_tests;

#[cfg(test)]
mod hatch_tests;

#[cfg(test)]
mod text_tests;

#[cfg(test)]
mod block_tests;

#[cfg(test)]
mod set_tests;

#[cfg(test)]
mod id_tests;

#[cfg(test)]
mod edit_tests;

#[cfg(test)]
mod property_tests;

#[cfg(test)]
mod polyline_tests;
