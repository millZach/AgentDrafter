//! Reads table styles and tables.
//!
//! A table style is a mapping from its name to what its rows draw with,
//! like a text style. A table is an object of a sheet: `objects.rs` lists
//! it and hands its keys to [`table`] here. A table belongs on a sheet, not
//! in a block.

use super::layers::{name_error, named_entries, object_color, object_lineweight};
use super::strict_yaml::{index_path, key_path, Kind, Node};
use super::text::{style_name, text_style_at};

/// The key a table style and its rows name their text style under.
const TEXT_STYLE_KEY: &str = "text_style";
use super::{at, bad_value, boolean, choice, length, point, sequence, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{Point3, RowKind, Table, TableCell, TableStyle, STANDARD_TABLE_STYLE};
use crate::text::{Attachment, TextStyle};
use crate::units::Unit;

/// The keys a table style takes.
pub(crate) const TABLE_STYLE_KEYS: [&str; 9] = [
    TEXT_STYLE_KEY,
    "text_height",
    "horizontal_margin",
    "vertical_margin",
    "border_color",
    "border_lineweight",
    "title",
    "header",
    "data",
];

/// The keys a kind of row of a table style takes.
pub(crate) const ROW_STYLE_KEYS: [&str; 3] = [TEXT_STYLE_KEY, "text_height", "justify"];

/// The keys a table takes.
pub(crate) const TABLE_KEYS: [&str; 12] = [
    "position",
    "columns",
    "rows",
    "cells",
    "title",
    "header",
    "table_style",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The keys a cell written as a mapping takes.
pub(crate) const CELL_KEYS: [&str; 5] = ["text", "style", "height", "justify", "merge"];

pub(super) fn table_styles(
    node: &Node,
    units: Unit,
    text_styles: &[TextStyle],
) -> Result<Vec<TableStyle>, Error> {
    let entries = named_entries(node, "table_styles", "table style")?;
    let mut styles = Vec::with_capacity(entries.len());
    for entry in entries {
        // As for text styles: `Standard` is written exactly.
        if entry.key != STANDARD_TABLE_STYLE && entry.key.eq_ignore_ascii_case(STANDARD_TABLE_STYLE)
        {
            return Err(name_error(
                entry,
                "table_styles",
                format!(
                    "`{}` is the table style `{STANDARD_TABLE_STYLE}`, which every drawing has, written in another case, and AutoCAD ignores case in names; to set what it draws with, list it as `{STANDARD_TABLE_STYLE}`",
                    entry.key
                ),
            ));
        }
        let path = key_path("table_styles", &entry.key);
        let fields = Fields::of(&entry.value, &path, &TABLE_STYLE_KEYS, "a table style")?;
        let text_style = text_style_at(&fields, TEXT_STYLE_KEY, text_styles)?;
        fields.require("text_height")?;
        let text_height =
            positive_length(&fields, "text_height", units, "a text height")?.unwrap_or_default();
        let margin = TableStyle::standard(units).horizontal_margin;
        let mut style = TableStyle::with(&entry.key, &text_style, [text_height; 3], margin);
        if let Some(m) = margin_of(&fields, "horizontal_margin", units)? {
            style.horizontal_margin = m;
        }
        if let Some(m) = margin_of(&fields, "vertical_margin", units)? {
            style.vertical_margin = m;
        }
        if let Some(node) = fields.get("border_color") {
            style.border_color = object_color(node, &fields.path_of("border_color"))?;
        }
        if let Some(node) = fields.get("border_lineweight") {
            style.border_lineweight =
                object_lineweight(node, &fields.path_of("border_lineweight"))?;
        }
        for kind in RowKind::ALL {
            let Some(node) = fields.get(kind.name()) else {
                continue;
            };
            let row_path = fields.path_of(kind.name());
            let row_fields = Fields::of(
                node,
                &row_path,
                &ROW_STYLE_KEYS,
                &format!("a table style's {} rows", kind.name()),
            )?;
            let row = style.row_mut(kind);
            if row_fields.text(TEXT_STYLE_KEY).is_some() {
                row.text_style = text_style_at(&row_fields, TEXT_STYLE_KEY, text_styles)?;
            }
            if let Some(height) =
                positive_length(&row_fields, "text_height", units, "a text height")?
            {
                row.text_height = height;
            }
            if let Some(justify) = justify(&row_fields)? {
                row.justify = justify;
            }
        }
        styles.push(style);
    }
    Ok(styles)
}

/// A margin: a length of 0 or more.
fn margin_of(fields: &Fields, key: &str, units: Unit) -> Result<Option<f64>, Error> {
    let Some(node) = fields.get(key) else {
        return Ok(None);
    };
    let path = fields.path_of(key);
    let margin = length(node, &path, units)?;
    if margin < 0.0 {
        return Err(bad_value(node, &path, "a margin is a length of 0 or more"));
    }
    Ok(Some(margin + 0.0))
}

/// The length under `key`, more than 0, if it is given. `what` names it in
/// the error, such as "a text height".
fn positive_length(
    fields: &Fields,
    key: &str,
    units: Unit,
    what: &str,
) -> Result<Option<f64>, Error> {
    let Some(node) = fields.get(key) else {
        return Ok(None);
    };
    let path = fields.path_of(key);
    let value = length(node, &path, units)?;
    if value > 0.0 {
        Ok(Some(value))
    } else {
        Err(bad_value(node, &path, format!("{what} is more than 0")))
    }
}

/// Where text sits in its cell, if it is given: one of the nine
/// attachments of multiline text.
fn justify(fields: &Fields) -> Result<Option<Attachment>, Error> {
    let Some(node) = fields.get("justify") else {
        return Ok(None);
    };
    let path = fields.path_of("justify");
    let names = Attachment::ALL.map(Attachment::name);
    let listed = format!("a cell's text is justified {}", names.join(", "));
    let text = choice(node, &path, "a justification", &listed)?;
    match names.iter().position(|n| *n == text) {
        Some(i) => Ok(Some(Attachment::ALL[i])),
        None => Err(bad_value(
            node,
            &path,
            format!("`{text}` is not a justification; {listed}"),
        )),
    }
}

pub(super) fn table(
    fields: &Fields,
    units: Unit,
    layer: String,
    text_styles: &[TextStyle],
    table_styles: &[TableStyle],
) -> Result<Table, Error> {
    let position = point(
        fields.require("position")?,
        &fields.path_of("position"),
        units,
    )?;
    let columns = sizes(fields, "columns", units, "a column's width")?;
    let rows = sizes(fields, "rows", units, "a row's height")?;
    let title = match fields.get("title") {
        Some(node) => boolean(node, &fields.path_of("title"))?,
        None => false,
    };
    let header = match fields.get("header") {
        Some(node) => boolean(node, &fields.path_of("header"))?,
        None => false,
    };
    if title && header && rows.len() < 2 {
        let node = fields.require("header")?;
        return Err(bad_value(
            node,
            &fields.path_of("header"),
            "a header row is the row after the title row, and this table has one row",
        ));
    }
    let table_style = table_style_name(fields, table_styles)?;
    let cells_node = fields.require("cells")?;
    let cells_path = fields.path_of("cells");
    let listed = sequence(cells_node, &cells_path, "a list of rows of cells")?;
    if listed.len() != rows.len() {
        return Err(bad_value(
            cells_node,
            &cells_path,
            format!(
                "`cells` lists {} rows and `rows` gives {}; list one row of cells for each row",
                listed.len(),
                rows.len()
            ),
        ));
    }
    let mut cells = Vec::with_capacity(listed.len());
    for (r, row_node) in listed.iter().enumerate() {
        let row_path = index_path(&cells_path, r);
        let items = sequence(row_node, &row_path, "a list of cells, one for each column")?;
        if items.len() != columns.len() {
            return Err(bad_value(
                row_node,
                &row_path,
                format!(
                    "the row lists {} cells and `columns` gives {}; list one cell for each column, `''` for an empty one",
                    items.len(),
                    columns.len()
                ),
            ));
        }
        let row = items
            .iter()
            .enumerate()
            .map(|(c, item)| cell(item, &index_path(&row_path, c), units, text_styles))
            .collect::<Result<Vec<_>, _>>()?;
        cells.push(row);
    }
    check_merges(&cells, listed, &cells_path)?;
    Ok(Table {
        position: Point3 {
            z: position.z + 0.0,
            ..position
        },
        columns,
        rows,
        cells,
        title,
        header,
        table_style,
        layer,
        properties: Default::default(),
    })
}

/// A list of at least one length more than 0: a table's column widths or
/// row heights. `what` names one, such as "a column's width".
fn sizes(fields: &Fields, key: &str, units: Unit, what: &str) -> Result<Vec<f64>, Error> {
    let node = fields.require(key)?;
    let path = fields.path_of(key);
    let items = sequence(node, &path, "a list of lengths")?;
    if items.is_empty() {
        return Err(bad_value(
            node,
            &path,
            format!("a table has at least one {}", &key[..key.len() - 1]),
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let item_path = index_path(&path, i);
            let size = length(item, &item_path, units)?;
            if size > 0.0 {
                Ok(size)
            } else {
                Err(bad_value(
                    item,
                    &item_path,
                    format!("{what} is more than 0"),
                ))
            }
        })
        .collect()
}

/// The table style a table draws with: `Standard` or one of the sheet's.
fn table_style_name(fields: &Fields, styles: &[TableStyle]) -> Result<String, Error> {
    let Some(node) = fields.text("table_style") else {
        return Ok(STANDARD_TABLE_STYLE.to_owned());
    };
    let path = fields.path_of("table_style");
    let mut choices = vec![STANDARD_TABLE_STYLE];
    choices.extend(
        styles
            .iter()
            .map(|s| s.name.as_str())
            .filter(|n| *n != STANDARD_TABLE_STYLE),
    );
    let listed = format!("the sheet's table styles: {}", choices.join(", "));
    let text = choice(node, &path, "the name of a table style", &listed)?;
    if choices.contains(&text) {
        return Ok(text.to_owned());
    }
    Err(at(
        Error::new(
            ErrorCode::UnknownName,
            format!("unknown table style `{text}`; {listed}"),
        ),
        &path,
        node.at,
    ))
}

/// One cell: its text alone, or a mapping of its text and what it draws
/// with. A bare null is an empty cell.
fn cell(
    node: &Node,
    path: &str,
    units: Unit,
    text_styles: &[TextStyle],
) -> Result<TableCell, Error> {
    if node.is_bare_null() {
        return Ok(TableCell::default());
    }
    if let Kind::Scalar(written) = &node.kind {
        return Ok(TableCell {
            text: cell_text(node, path, written)?,
            ..TableCell::default()
        });
    }
    let fields = Fields::of(node, path, &CELL_KEYS, "a cell, its text or a mapping,")?;
    let text = match fields.text("text") {
        Some(text_node) => {
            let text_path = fields.path_of("text");
            let written = super::scalar(text_node, &text_path, "the cell's text")?;
            cell_text(text_node, &text_path, written)?
        }
        None => String::new(),
    };
    let style = match fields.text("style") {
        Some(_) => Some(style_name(&fields, text_styles)?),
        None => None,
    };
    let height = positive_length(&fields, "height", units, "a text height")?;
    let justify = justify(&fields)?;
    let merge = match fields.get("merge") {
        Some(merge_node) => Some(merge(merge_node, &fields.path_of("merge"))?),
        None => None,
    };
    Ok(TableCell {
        text,
        style,
        height,
        justify,
        merge,
    })
}

/// A cell's text: what shows, with line breaks between its lines and none
/// at the end, or empty for an empty cell.
fn cell_text(node: &Node, path: &str, written: &str) -> Result<String, Error> {
    let text = written.trim_end_matches(['\n', '\r']).replace("\r\n", "\n");
    let problem = if !text.is_empty() && text.trim().is_empty() {
        Some("a cell's text shows something; write `''` for an empty cell".to_owned())
    } else {
        text.chars()
            .find(|c| c.is_control() && *c != '\n')
            .map(|c| {
                format!(
                    "text cannot hold the control character `{}`",
                    c.escape_default()
                )
            })
    };
    match problem {
        Some(problem) => Err(bad_value(node, path, problem)),
        None => Ok(text),
    }
}

/// A merge: `[rows, columns]`, two whole numbers from 1, not both 1.
fn merge(node: &Node, path: &str) -> Result<[usize; 2], Error> {
    const SHAPE: &str = "a merge, `[rows, columns]`, two whole numbers such as `[1, 3]`";
    let items = sequence(node, path, SHAPE)?;
    let [down, across] = items else {
        return Err(bad_value(node, path, format!("expected {SHAPE}")));
    };
    let whole = |item: &Node, i: usize| -> Result<usize, Error> {
        let item_path = index_path(path, i);
        let text = super::scalar(item, &item_path, "a whole number")?;
        match text.parse::<usize>() {
            Ok(n) if n >= 1 && text.bytes().all(|b| b.is_ascii_digit()) => Ok(n),
            _ => Err(at(
                Error::new(
                    ErrorCode::BadNumber,
                    format!("expected a whole number from 1, found `{text}`"),
                ),
                &item_path,
                item.at,
            )),
        }
    };
    let span = [whole(down, 0)?, whole(across, 1)?];
    if span == [1, 1] {
        return Err(bad_value(
            node,
            path,
            "a merge spans more than one cell; `[1, 1]` is the cell alone, so leave `merge` out",
        ));
    }
    Ok(span)
}

/// Refuses a merge that runs past the table or over another, and a cell a
/// merge spans that is not empty.
fn check_merges(cells: &[Vec<TableCell>], listed: &[Node], path: &str) -> Result<(), Error> {
    let rows = cells.len();
    let columns = cells.first().map_or(0, Vec::len);
    let mut owner: Vec<Vec<Option<[usize; 2]>>> = vec![vec![None; columns]; rows];
    for (r, row) in cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let Some([down, across]) = cell.merge else {
                continue;
            };
            let cell_node = &sequence(&listed[r], "", "")?[c];
            let merge_path = key_path(&index_path(&index_path(path, r), c), "merge");
            if down > rows - r || across > columns - c {
                return Err(bad_value(
                    cell_node,
                    &merge_path,
                    format!(
                        "the merge spans {down} rows and {across} columns from row {r}, column {c}, past the table's {rows} rows and {columns} columns (counted from 0)"
                    ),
                ));
            }
            for (rr, row_owner) in owner.iter_mut().enumerate().skip(r).take(down) {
                for (cc, square) in row_owner.iter_mut().enumerate().skip(c).take(across) {
                    if let Some([or, oc]) = square {
                        return Err(bad_value(
                            cell_node,
                            &merge_path,
                            format!(
                                "the merge from row {r}, column {c} runs over the merge from row {or}, column {oc}, at row {rr}, column {cc}"
                            ),
                        ));
                    }
                    *square = Some([r, c]);
                }
            }
        }
    }
    for (r, row) in cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let Some([or, oc]) = owner[r][c] else {
                continue;
            };
            if [or, oc] == [r, c] || *cell == TableCell::default() {
                continue;
            }
            let cell_node = &sequence(&listed[r], "", "")?[c];
            return Err(bad_value(
                cell_node,
                &index_path(&index_path(path, r), c),
                format!(
                    "the cell at row {r}, column {c} is merged into the cell at row {or}, column {oc}, and so is empty: write `''`"
                ),
            ));
        }
    }
    Ok(())
}
