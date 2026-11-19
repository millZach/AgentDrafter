//! Fills a template in from a row of a drawing set.
//!
//! A template is written with what a sheet description holds. Where a
//! sheet's value comes from its row, the template writes `{row: NAME}` in
//! place of the value: as an insert's attribute value, or an item of the
//! list of values of a tag its block defines more than once, as a text's
//! or an mtext's contents, or as a table cell's text. A sheet is the template with each `{row: NAME}`
//! replaced by the row's value `NAME`, read as a sheet description is read.
//!
//! A text's or an mtext's contents, and the value of an attribute an
//! insert's block defines once, may also be a list of parts, plain text and
//! values from the row joined in order ([`super::parts`]), as Zach decided
//! on 2026-11-19 (#102). A tag the block defines more than once takes a
//! list of its values, as before, so which a list is comes of the block.
//!
//! A row may also hold objects of its own, which are on its sheet alone,
//! after the template's. They write their values themselves: a value from
//! the row has no place in them.
//!
//! A row that lacks a value the template needs is an error naming the sheet
//! and the value. A value the template has no place for does not stop the
//! sheet but is reported as a notice, as Zach decided on 2026-11-11. A value the
//! sheet description's rules refuse where the template puts it, such as
//! empty contents for a text, is an error at the value in the rows file.

use std::path::Path;

use super::blocks::{self, Noticed};
use super::ids::ObjectRecord;
use super::parts::{self, read_parts};
use super::records::{notices_of, SheetRecord};
use super::set::{value_name_problem, Row};
use super::strict_yaml::{self, index_path, key_path, Kind, Node, Pos};
use super::{
    bad_value, library_beside_in, read_file_in, scalar, sheet_and_records, Disk, Fields,
    LibraryFile, ReadFiles, RowObjects,
};
use crate::error::{Error, ErrorCode};
use crate::model::{BlockLibrary, Sheet};
use crate::set::{Notice, NoticeCode};
use crate::units::Unit;

/// The one key of a value a template takes from a row: `{row: NAME}`.
pub(crate) const ROW_KEY: &str = "row";

/// How many times the block of an insert carries an attribute: given the
/// block's name and the tag, in capitals, the number of its definitions
/// with that tag an insert carries a value for, when the block is known.
/// A list given for a tag carried once is a list of parts; for a tag
/// carried more than once, the tag's values.
pub(crate) type Carries<'a> = &'a dyn Fn(&str, &str) -> Option<usize>;

/// Knows no block, so a list given for an attribute is taken item by item,
/// as a tag's values.
pub(crate) fn no_blocks(_: &str, _: &str) -> Option<usize> {
    None
}

/// How many times the block `block` of `library` carries `tag`, as
/// [`Carries`] asks.
fn carried_in(library: &BlockLibrary, block: &str, tag: &str) -> Option<usize> {
    library
        .blocks
        .iter()
        .find(|b| b.name == block)
        .map(|b| b.carried_attributes().filter(|d| d.tag == tag).count())
}

/// One place a row's value went in a template.
struct Filled {
    /// The path of the place in the template, such as
    /// `objects[1].insert.attributes.TITLE`.
    place: String,
    /// The name of the row's value.
    name: String,
    /// Where `{row: NAME}` is written in the template.
    at: Pos,
    /// The list of parts the value went in, as the template writes it, when
    /// it went in one.
    parts: Option<String>,
}

/// Reads the template in the file at `path` as the sheet `row` names,
/// filled in with the row's values. The row is in the rows file at
/// `rows_file`, and the sheet is built in a drawing set whose declared unit
/// is `set_units`, which the template must be in.
///
/// An error about a row's value names the rows file; one in the template
/// names the template, and one in the block library it names, the library.
/// A value of the row the template has no place for is added to `notices`,
/// whether or not the sheet is built.
///
/// The sheet comes with the record of each of its objects and is named by
/// its row, whose name joins its objects' IDs.
pub(crate) fn read_template(
    path: &Path,
    row: &Row,
    rows_file: &Path,
    set_units: Unit,
    notices: &mut Vec<Notice>,
) -> Result<SheetRecord, Error> {
    read_template_in(&Disk, path, row, rows_file, set_units, notices)
}

/// Reads a template as [`read_template`] does, and the block library it
/// names, from `files`.
pub(crate) fn read_template_in(
    files: &dyn ReadFiles,
    path: &Path,
    row: &Row,
    rows_file: &Path,
    set_units: Unit,
    notices: &mut Vec<Notice>,
) -> Result<SheetRecord, Error> {
    let text = read_file_in(files, path, "the template")?;
    template_in(files, &text, path, row, rows_file, set_units, notices).map(|(record, _)| record)
}

/// Reads `text`, the template in the file at `path`, as [`read_template`]
/// reads it, and gives the block library's file as it was read, if the
/// template names one, so that an output can be stamped with the bytes it
/// was drawn from.
pub(crate) fn template_at(
    text: &str,
    path: &Path,
    row: &Row,
    rows_file: &Path,
    set_units: Unit,
    notices: &mut Vec<Notice>,
) -> Result<(SheetRecord, Option<LibraryFile>), Error> {
    template_in(&Disk, text, path, row, rows_file, set_units, notices)
}

/// Reads a template's text as [`template_at`] does, with the block library
/// it names read from `files`, such as a change set's edited library over
/// the disk.
pub(crate) fn template_in(
    files: &dyn ReadFiles,
    text: &str,
    path: &Path,
    row: &Row,
    rows_file: &Path,
    set_units: Unit,
    notices: &mut Vec<Notice>,
) -> Result<(SheetRecord, Option<LibraryFile>), Error> {
    let mut library: Option<LibraryFile> = None;
    let mut noticed = Vec::new();
    let (sheet, objects) = fill_with_records(
        text,
        row,
        rows_file,
        set_units,
        &mut library_beside_in(files, path, &mut library),
        notices,
        &mut noticed,
    )
    .map_err(|e| e.or_in_file(path))?;
    // What reading the block library noticed, then the sheet's objects.
    if let Some(read) = &mut library {
        let from_library = std::mem::take(&mut read.noticed);
        notices.extend(notices_of(from_library, &row.sheet, &row.template, path));
    }
    notices.extend(notices_of(noticed, &row.sheet, &row.template, path));
    let record = SheetRecord {
        name: row.sheet.clone(),
        file: path.to_owned(),
        sheet,
        objects,
        rows_file: Some(rows_file.to_owned()),
    };
    Ok((record, library))
}

/// The block library a template's text names under `block_library`, as it
/// writes the path; none when it names none or cannot be read.
pub(crate) fn library_named(text: &str) -> Option<String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let root = strict_yaml::read(text).ok()??;
    let Kind::Mapping(entries) = &root.kind else {
        return None;
    };
    let entry = entries.iter().find(|e| e.key == "block_library")?;
    match &entry.value.kind {
        Kind::Scalar(path) if !entry.value.is_bare_null() => Some(path.clone()),
        _ => None,
    }
}

/// Fills a template in from a row as [`fill_with_records`] does, without
/// its objects' records.
#[cfg(test)]
pub(super) fn fill(
    text: &str,
    row: &Row,
    rows_file: &Path,
    set_units: Unit,
    load: &mut dyn FnMut(&str) -> Result<BlockLibrary, Error>,
    notices: &mut Vec<Notice>,
) -> Result<Sheet, Error> {
    fill_with_records(
        text,
        row,
        rows_file,
        set_units,
        load,
        notices,
        &mut Vec::new(),
    )
    .map(|(sheet, _)| sheet)
}

/// Reads the text of a template as the sheet `row` names, filled in with
/// the row's values, with the record of each of the sheet's objects: where
/// the template writes it and the ID it carries there. `load` reads the
/// block library the template names, as for a sheet description. An error
/// about a row's value names `rows_file`; any other is left for the caller
/// to place in the template's file, unless it names the block library's. A
/// value of the row the template has no place for is added to `notices`,
/// and what reading the sheet's objects noticed to `noticed`.
pub(super) fn fill_with_records(
    text: &str,
    row: &Row,
    rows_file: &Path,
    set_units: Unit,
    load: &mut dyn FnMut(&str) -> Result<BlockLibrary, Error>,
    notices: &mut Vec<Notice>,
    noticed: &mut Vec<Noticed>,
) -> Result<(Sheet, Vec<ObjectRecord>), Error> {
    let mut root = strict_yaml::read(text)?;
    let mut filled: Vec<Filled> = Vec::new();
    let mut missing: Vec<(String, String)> = Vec::new();
    // A list given for an attribute is a list of parts or a tag's values,
    // as the insert's block carries the tag once or more, so the block
    // library is read first when the template gives one.
    let mut early: Option<(String, Result<BlockLibrary, Error>)> =
        root.as_ref().and_then(library_read_early).map(|written| {
            let read = load(&written);
            (written, read)
        });
    if let Some(root) = root.as_mut() {
        let library = early.as_ref().and_then(|(_, read)| read.as_ref().ok());
        let carries = |block: &str, tag: &str| carried_in(library?, block, tag);
        each_place(root, &carries, &mut |node, place, lists| {
            if lists.take() {
                // Only an mtext's contents take more than one line.
                let one_line = lists == Lists::OneLine;
                if let Some(parts) = read_parts(node, &place, one_line)? {
                    let pattern = parts::pattern(&parts);
                    let joined =
                        parts::join(&parts, |name| row.value(name).map(|v| v.text.clone()));
                    let lacking = joined.as_ref().err().cloned().unwrap_or_default();
                    for name in parts::names(&parts) {
                        if lacking.contains(&name) {
                            missing.push((name, place.clone()));
                        } else {
                            filled.push(Filled {
                                place: place.clone(),
                                name,
                                at: node.at,
                                parts: Some(pattern.clone()),
                            });
                        }
                    }
                    if let Ok(text) = joined {
                        node.kind = Kind::Scalar(text);
                    }
                    return Ok(());
                }
            }
            let Some(name) = row_value_name(node, &place)? else {
                return Ok(());
            };
            match row.value(&name) {
                Some(value) => {
                    node.kind = Kind::Scalar(value.text.clone());
                    filled.push(Filled {
                        place,
                        name,
                        at: node.at,
                        parts: None,
                    });
                }
                None => missing.push((name, place)),
            }
            Ok(())
        })?;
    }
    notices.extend(unused_values(row, &filled, &missing, rows_file));
    if !missing.is_empty() {
        return Err(missing_values(row, &missing).in_file(rows_file));
    }
    let row_objects = RowObjects {
        items: &row.objects,
        path: row.objects_path(),
        file: rows_file,
    };
    // The library read first is the one the sheet reads.
    let mut load_once = |written: &str| match early.take() {
        Some((first, read)) if first == written => read,
        _ => load(written),
    };
    sheet_and_records(
        root,
        &mut load_once,
        Some(set_units),
        Some(row_objects),
        noticed,
    )
    .map_err(|e| at_row_value(e, &filled, row, rows_file))
}

/// The block library a template's tree names, as it writes the path, when
/// one of its inserts gives a list for an attribute, whose meaning the
/// library says; none otherwise, or when the path is wrong, for the
/// sheet's reading to refuse.
fn library_read_early(root: &Node) -> Option<String> {
    let Kind::Mapping(entries) = &root.kind else {
        return None;
    };
    let objects = entries.iter().find(|e| e.key == "objects")?;
    let Kind::Sequence(items) = &objects.value.kind else {
        return None;
    };
    let lists = items.iter().any(|item| {
        let Kind::Mapping(types) = &item.kind else {
            return false;
        };
        let [object] = types.as_slice() else {
            return false;
        };
        let Kind::Mapping(keys) = &object.value.kind else {
            return false;
        };
        object.key == "insert"
            && keys.iter().any(|k| {
                k.key == "attributes"
                    && matches!(&k.value.kind, Kind::Mapping(values)
                        if values.iter().any(|v| matches!(v.value.kind, Kind::Sequence(_))))
            })
    });
    if !lists {
        return None;
    }
    let library = entries.iter().find(|e| e.key == "block_library")?;
    if library.value.is_bare_null() {
        return None;
    }
    blocks::library_path(&library.value, "block_library")
        .ok()
        .map(str::to_owned)
}

/// A notice for each of the row's values its template has no place for.
/// Such a value does not stop the sheet, as Zach decided on 2026-11-11, but
/// is reported, so a stray or misspelt value stays in sight. `filled` and
/// `missing` are the template's places, with the value's name of each.
fn unused_values(
    row: &Row,
    filled: &[Filled],
    missing: &[(String, String)],
    rows_file: &Path,
) -> Vec<Notice> {
    let mut takes: Vec<&str> = Vec::new();
    let names = filled
        .iter()
        .map(|f| f.name.as_str())
        .chain(missing.iter().map(|(name, _)| name.as_str()));
    for name in names {
        if !takes.contains(&name) {
            takes.push(name);
        }
    }
    let takes = if takes.is_empty() {
        "it takes no values from a row".to_owned()
    } else {
        format!("it takes {}", takes.join(", "))
    };
    row.values
        .iter()
        .filter(|v| !filled.iter().any(|f| f.name == v.name))
        .map(|unused| Notice {
            code: NoticeCode::UnusedValue,
            message: format!(
                "sheet `{}` gives the value `{}`, which its template `{}` has no place for, so \
                 it is not used; {takes}",
                row.sheet, unused.name, row.template
            ),
            sheet: row.sheet.clone(),
            template: row.template.clone(),
            field: row.value_path(&unused.name),
            line: unused.key_at.line,
            column: unused.key_at.column,
            file: rows_file.display().to_string(),
        })
        .collect()
}

/// The names of the row's values a template's text takes, each once, in
/// the order it first writes them. A template that cannot be read, or
/// writes a value from the row wrongly, takes none.
#[cfg(test)]
pub(super) fn names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(Some(mut root)) = strict_yaml::read(text) {
        let _ = each_place(&mut root, &no_blocks, &mut |node, place, lists| {
            let found = match lists.take().then(|| parts::parts_of(node)).flatten() {
                Some(parts) => parts::names(&parts),
                None => row_value_name(node, &place)?.into_iter().collect(),
            };
            for name in found {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            Ok(())
        });
    }
    names
}

/// Refuses `{row: NAME}` in a sheet description built on its own: only a
/// template built from a row of a drawing set has a row to take it from.
/// So is a list of parts that holds one.
pub(super) fn refuse_row_values(root: Option<&mut Node>) -> Result<(), Error> {
    let Some(root) = root else {
        return Ok(());
    };
    each_place(
        root,
        &no_blocks,
        &mut |node, place, _| match row_value_in(node, &place) {
            Some((at, field)) => Err(Error::new(
                ErrorCode::WrongType,
                "a value from a row, `{row: NAME}`, is filled in when a template is built as a \
                 sheet of a drawing set; a sheet description built on its own writes the value \
                 itself, so build the set's folder or write the value here",
            )
            .at_field(field)
            .at_position(at.line, at.column)),
            None => Ok(()),
        },
    )
}

/// Refuses `{row: NAME}` in one of a row's own objects, at `path` in the
/// rows file: the row writes its own objects' values itself. So is a list
/// of parts that holds one.
pub(super) fn refuse_row_values_in(item: &Node, path: &str) -> Result<(), Error> {
    let mut item = item.clone();
    places_in(
        &mut item,
        path,
        &no_blocks,
        &mut |node, place, _| match row_value_in(node, &place) {
            Some((at, field)) => Err(Error::new(
                ErrorCode::WrongType,
                "a value from a row, `{row: NAME}`, belongs in a template; a row's own objects \
                 are on its sheet alone, so write the value itself here",
            )
            .at_field(field)
            .at_position(at.line, at.column)),
            None => Ok(()),
        },
    )
}

/// Where `node`, at `place`, writes a value from the row, and the path of
/// it: `node` itself when it is `{row: NAME}`, or the first such item of a
/// list.
fn row_value_in(node: &Node, place: &str) -> Option<(Pos, String)> {
    let is_row = |n: &Node| matches!(&n.kind, Kind::Mapping(entries) if entries.iter().any(|e| e.key == ROW_KEY));
    if is_row(node) {
        return Some((node.at, place.to_owned()));
    }
    let Kind::Sequence(items) = &node.kind else {
        return None;
    };
    items
        .iter()
        .enumerate()
        .find(|(_, item)| is_row(item))
        .map(|(i, item)| (item.at, index_path(place, i)))
}

/// Whether a list of parts may stand at a place, and of what kind of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lists {
    /// None may: a table cell, a leader's text, a repeated tag's value.
    None,
    /// A text's contents, or the value of an attribute its insert's block
    /// carries once: one line.
    OneLine,
    /// An mtext's contents, which may take more than one line.
    Lines,
}

impl Lists {
    /// Whether a list of parts may stand there.
    pub(crate) fn take(self) -> bool {
        self != Lists::None
    }
}

/// What [`places_in`] calls with each place: the node there, its path,
/// and whether a list of parts may stand there, and of what kind of text.
pub(crate) type Visit<'a> = dyn FnMut(&mut Node, String, Lists) -> Result<(), Error> + 'a;

/// Calls `visit` with each place in a sheet's tree where a template may put
/// a value from a row, and the path of the place: each value under an
/// insert's `attributes`, and the `contents` of each text and mtext. A tree
/// of another shape is left as it is, for the reader to refuse. `carries`
/// says how many times an insert's block carries a tag.
fn each_place(root: &mut Node, carries: Carries, visit: &mut Visit) -> Result<(), Error> {
    let Kind::Mapping(entries) = &mut root.kind else {
        return Ok(());
    };
    let Some(objects) = entries.iter_mut().find(|e| e.key == "objects") else {
        return Ok(());
    };
    let Kind::Sequence(items) = &mut objects.value.kind else {
        return Ok(());
    };
    for (i, item) in items.iter_mut().enumerate() {
        places_in(item, &index_path("objects", i), carries, visit)?;
    }
    Ok(())
}

/// Calls `visit` with each value of an insert's `attributes`, or of a
/// multileader's block content's, at `path`. For an insert, `block` names
/// its block: a list given for a tag `carries` says it carries once is a
/// list of parts, visited whole.
fn attribute_places(
    attributes: &mut Node,
    path: &str,
    block: Option<&str>,
    carries: Carries,
    visit: &mut Visit,
) -> Result<(), Error> {
    let Kind::Mapping(values) = &mut attributes.kind else {
        return Ok(());
    };
    for value in values {
        let path = key_path(path, &value.key);
        let once =
            block.and_then(|block| carries(block, &blocks::tag_in_capitals(&value.key))) == Some(1);
        // A tag the block defines more than once takes a list, and each of
        // its items may be a value from the row.
        match &mut value.value.kind {
            Kind::Sequence(_) if once => visit(&mut value.value, path, Lists::OneLine)?,
            Kind::Sequence(items) => {
                for (i, item) in items.iter_mut().enumerate() {
                    visit(item, index_path(&path, i), Lists::None)?;
                }
            }
            _ => visit(&mut value.value, path, Lists::None)?,
        }
    }
    Ok(())
}

/// Calls `visit` with each place in the object `item`, at `path`, where a
/// template may put a value from a row, as [`each_place`] does for a
/// sheet's objects.
pub(super) fn places_in(
    item: &mut Node,
    path: &str,
    carries: Carries,
    visit: &mut Visit,
) -> Result<(), Error> {
    let Kind::Mapping(types) = &mut item.kind else {
        return Ok(());
    };
    let [object] = types.as_mut_slice() else {
        return Ok(());
    };
    let object_path = key_path(path, &object.key);
    let Kind::Mapping(keys) = &mut object.value.kind else {
        return Ok(());
    };
    match object.key.as_str() {
        "insert" => {
            let block = keys
                .iter()
                .find(|k| k.key == "block")
                .and_then(|k| match &k.value.kind {
                    Kind::Scalar(name) => Some(name.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let Some(attributes) = keys.iter_mut().find(|k| k.key == "attributes") else {
                return Ok(());
            };
            attribute_places(
                &mut attributes.value,
                &key_path(&object_path, "attributes"),
                Some(&block),
                carries,
                visit,
            )?;
        }
        "text" | "mtext" => {
            if let Some(contents) = keys.iter_mut().find(|k| k.key == "contents") {
                let lists = if object.key == "mtext" {
                    Lists::Lines
                } else {
                    Lists::OneLine
                };
                visit(
                    &mut contents.value,
                    key_path(&object_path, "contents"),
                    lists,
                )?;
            }
        }
        // A multileader's content's text, and an older leader's
        // annotation's.
        "mleader" | "leader" => {
            let holder = if object.key == "mleader" {
                "content"
            } else {
                "annotation"
            };
            let Some(held) = keys.iter_mut().find(|k| k.key == holder) else {
                return Ok(());
            };
            let Kind::Mapping(held_keys) = &mut held.value.kind else {
                return Ok(());
            };
            let held_path = key_path(&object_path, holder);
            for key in held_keys.iter_mut() {
                match key.key.as_str() {
                    "text" => visit(&mut key.value, key_path(&held_path, "text"), Lists::None)?,
                    // A multileader's block content's attribute values, as
                    // an insert's.
                    "attributes" if holder == "content" => {
                        attribute_places(
                            &mut key.value,
                            &key_path(&held_path, "attributes"),
                            None,
                            carries,
                            visit,
                        )?;
                    }
                    _ => {}
                }
            }
        }
        // Each cell's text: the cell itself when it is written as its text
        // or as a value from the row, or its `text`.
        "table" => {
            let Some(cells) = keys.iter_mut().find(|k| k.key == "cells") else {
                return Ok(());
            };
            let Kind::Sequence(rows) = &mut cells.value.kind else {
                return Ok(());
            };
            let cells_path = key_path(&object_path, "cells");
            for (r, row) in rows.iter_mut().enumerate() {
                let Kind::Sequence(items) = &mut row.kind else {
                    continue;
                };
                let row_path = index_path(&cells_path, r);
                for (c, item) in items.iter_mut().enumerate() {
                    let path = index_path(&row_path, c);
                    let Kind::Mapping(keys) = &mut item.kind else {
                        continue;
                    };
                    if keys.iter().any(|k| k.key == ROW_KEY) {
                        visit(item, path, Lists::None)?;
                    } else if let Some(text) = keys.iter_mut().find(|k| k.key == "text") {
                        visit(&mut text.value, key_path(&path, "text"), Lists::None)?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The name of the row's value a template puts at `node`, if it puts one
/// there: `{row: NAME}`. Any other mapping there is an error, since a
/// template puts text or a value from the row in that place.
pub(super) fn row_value_name(node: &Node, path: &str) -> Result<Option<String>, Error> {
    if !matches!(node.kind, Kind::Mapping(_)) {
        return Ok(None);
    }
    let fields = Fields::of(
        node,
        path,
        &[ROW_KEY],
        "a value from the row, `{row: NAME}`,",
    )?;
    let name_path = fields.path_of(ROW_KEY);
    let name_node = fields.require_text(ROW_KEY)?;
    let name = scalar(name_node, &name_path, "the name of one of the row's values")?;
    if let Some(problem) = value_name_problem(name) {
        return Err(bad_value(name_node, &name_path, problem));
    }
    Ok(Some(name.to_owned()))
}

/// The error for a row that lacks values its template needs: `missing`
/// holds each value's name and the place in the template it goes, in the
/// order the template writes them.
fn missing_values(row: &Row, missing: &[(String, String)]) -> Error {
    let mut names: Vec<&str> = Vec::new();
    for (name, _) in missing {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    let quoted = |items: Vec<&str>| {
        items
            .iter()
            .map(|i| format!("`{i}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let places = quoted(missing.iter().map(|(_, place)| place.as_str()).collect());
    let message = match names.as_slice() {
        [name] => format!(
            "sheet `{}` has no value `{name}`, which its template `{}` fills in at {places}; give it under the row's `values`",
            row.sheet, row.template
        ),
        _ => format!(
            "sheet `{}` has no values {}, which its template `{}` fills in at {places}; give them under the row's `values`",
            row.sheet,
            quoted(names.clone()),
            row.template
        ),
    };
    Error::new(ErrorCode::MissingValue, message)
        .at_field(row.value_path(names[0]))
        .at_position(row.at.line, row.at.column)
}

/// Moves an error about a value the row filled in to that value in the rows
/// file, saying which sheet and which place in the template it is about.
/// Any other error is left as it is.
fn at_row_value(error: Error, filled: &[Filled], row: &Row, rows_file: &Path) -> Error {
    let Some(f) = filled.iter().find(|f| {
        error.file.is_none()
            && error.field.as_deref() == Some(f.place.as_str())
            && error.line == Some(f.at.line)
            && error.column == Some(f.at.column)
    }) else {
        return error;
    };
    let at = row.value(&f.name).map_or(row.at, |v| v.at);
    let message = match &f.parts {
        None => format!(
            "the value `{}` of sheet `{}`, which its template `{}` fills in at `{}`: {}",
            f.name, row.sheet, row.template, f.place, error.message
        ),
        Some(pattern) => format!(
            "the text sheet `{}`'s template `{}` joins at `{}` from its parts {pattern}, with \
             the row's value `{}`: {}",
            row.sheet, row.template, f.place, f.name, error.message
        ),
    };
    Error {
        message,
        field: Some(row.value_path(&f.name)),
        line: Some(at.line),
        column: Some(at.column),
        file: Some(rows_file.display().to_string()),
        ..error
    }
}
