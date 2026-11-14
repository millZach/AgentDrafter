//! Table styles and tables in a DWG.
//!
//! Part of the seam ([`super`]): only this module and its siblings name
//! acadrust. A table is AutoCAD's TABLE, written with its table style, its
//! rows, columns and cells, and an anonymous block, `*T` and a number,
//! holding the lines and multiline text it draws, which AutoCAD shows until
//! it next lays the table out. acadrust fills that block from the table's
//! data, but with every text in `Standard` and every line in the table's
//! own pen; AutoCAD 2026, laying out the same made-up table, drew each
//! cell's text in its row's text style and the borders in the table style's
//! border colour and lineweight. So once acadrust has filled the block, each
//! of its texts is given the text style its cell draws with, and each of its
//! lines the border pen, as AutoCAD would draw them.
//!
//! A cell's text is coded as multiline text is, and its own text style,
//! height and justification are written as AutoCAD 2026 wrote them on a
//! made-up table: a style and a height as overrides of its content, a
//! justification as an override of its cell style.
//!
//! A table is read back when it has a model in the core: flat on the XY
//! plane and running along X, in a table style the core read, each cell
//! holding text or nothing, with no override but a text style, a height and
//! a justification.

use acadrust::entities::table::{CellRange, CellStyle, CellStylePropertyFlags as Override};
use acadrust::entities::{EntityType, Table as DwgTable, TableCell as DwgCell};
use acadrust::objects::{
    CellAlignment, ObjectType, RowCellStyle, TableCellBorder, TableStyle as DwgTableStyle,
};
use acadrust::types::{Color, Handle, LineWeight, Vector3};
use acadrust::CadDocument;

use super::text::{mtext_code, mtext_contents};
use super::{point, vector};
use crate::error::{Error, ErrorCode};
use crate::model::{
    Lineweight, ObjectColor, ObjectLineweight, RowKind, RowStyle, Sheet, Table, TableCell,
    TableStyle, STANDARD_TABLE_STYLE,
};
use crate::text::{Attachment, STANDARD};

/// The name of the dictionary a drawing keeps its table styles in.
const TABLE_STYLES: &str = "ACAD_TABLESTYLE";

/// What a cell's text content overrides by default: its data type and its
/// format, as AutoCAD writes a cell holding text.
const TEXT_CONTENT: i32 = 3;

/// The row cell style each kind of row draws with, as AutoCAD numbers them.
const fn style_id(kind: RowKind) -> i32 {
    match kind {
        RowKind::Title => 1,
        RowKind::Header => 2,
        RowKind::Data => 3,
    }
}

fn alignment(justify: Attachment) -> CellAlignment {
    let at = Attachment::ALL
        .iter()
        .position(|a| *a == justify)
        .unwrap_or(0);
    CellAlignment::from((at + 1) as i16)
}

fn justify(alignment: i32) -> Option<Attachment> {
    let at = usize::try_from(alignment).ok()?.checked_sub(1)?;
    Attachment::ALL.get(at).copied()
}

fn dwg_color(color: ObjectColor) -> Color {
    match color {
        ObjectColor::ByLayer => Color::ByLayer,
        ObjectColor::ByBlock => Color::ByBlock,
        ObjectColor::Index(index) => Color::Index(index),
    }
}

fn dwg_lineweight(lineweight: ObjectLineweight) -> LineWeight {
    match lineweight {
        ObjectLineweight::ByLayer => LineWeight::ByLayer,
        ObjectLineweight::ByBlock => LineWeight::ByBlock,
        ObjectLineweight::Default => LineWeight::Default,
        ObjectLineweight::Width(width) => LineWeight::Value(width.hundredths()),
    }
}

/// The drawing's dictionary of table styles.
fn styles_dictionary(drawing: &CadDocument) -> Option<Handle> {
    drawing
        .objects
        .iter()
        .find_map(|(handle, object)| match object {
            ObjectType::Dictionary(d) => d
                .entries
                .iter()
                .any(|(name, entry)| {
                    name == STANDARD_TABLE_STYLE
                        && matches!(drawing.objects.get(entry), Some(ObjectType::TableStyle(_)))
                })
                .then_some(*handle),
            _ => None,
        })
}

/// The handle of the table style named `name` in the drawing.
fn style_handle(drawing: &CadDocument, name: &str) -> Option<Handle> {
    drawing
        .objects
        .iter()
        .find_map(|(handle, object)| match object {
            ObjectType::TableStyle(style) if style.name == name => Some(*handle),
            _ => None,
        })
}

/// Writes every table style the sheet's tables draw with, and every one it
/// lists, into the drawing. `Standard` is in every drawing: the sheet sets
/// what it draws with when it lists it, or a table draws with it. The
/// drawing's text styles have their handles already.
pub(super) fn add_table_styles(drawing: &mut CadDocument, sheet: &Sheet) -> Result<(), Error> {
    let mut wanted: Vec<TableStyle> = sheet.table_styles.clone();
    for object in &sheet.objects {
        if let crate::model::Object::Table(table) = object {
            if !wanted.iter().any(|s| s.name == table.table_style) {
                if let Some(style) = sheet.table_style(&table.table_style) {
                    wanted.push(style);
                }
            }
        }
    }
    if wanted.is_empty() {
        return Ok(());
    }
    let dictionary = styles_dictionary(drawing).ok_or_else(|| {
        Error::new(
            ErrorCode::DwgWrite,
            format!("the drawing has no {TABLE_STYLES} dictionary to add table styles to"),
        )
    })?;
    for style in &wanted {
        match style_handle(drawing, &style.name) {
            Some(handle) => {
                let written = dwg_table_style(drawing, style, handle, dictionary);
                drawing
                    .objects
                    .insert(handle, ObjectType::TableStyle(written));
            }
            None => {
                let handle = drawing.allocate_handle();
                let written = dwg_table_style(drawing, style, handle, dictionary);
                drawing
                    .objects
                    .insert(handle, ObjectType::TableStyle(written));
                if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&dictionary) {
                    d.add_entry(&style.name, handle);
                }
            }
        }
    }
    Ok(())
}

fn dwg_table_style(
    drawing: &CadDocument,
    style: &TableStyle,
    handle: Handle,
    dictionary: Handle,
) -> DwgTableStyle {
    let mut written = DwgTableStyle::new(&style.name);
    written.handle = handle;
    written.owner_handle = dictionary;
    written.horizontal_margin = style.horizontal_margin;
    written.vertical_margin = style.vertical_margin;
    let border = TableCellBorder::with_style(
        dwg_color(style.border_color),
        dwg_lineweight(style.border_lineweight),
    );
    for (row, rows) in [
        (&style.title, &mut written.title_row_style),
        (&style.header, &mut written.header_row_style),
        (&style.data, &mut written.data_row_style),
    ] {
        rows.text_style_name.clone_from(&row.text_style);
        rows.text_style_handle = drawing.text_styles.get(&row.text_style).map(|s| s.handle);
        rows.text_height = row.text_height;
        rows.alignment = alignment(row.justify);
        rows.set_all_borders(border.clone());
    }
    written
}

/// The DWG table for one of ours, drawing with `style`, which the drawing
/// holds. Its block is made once it is in the drawing: see [`fill_block`].
pub(super) fn dwg_table(
    drawing: &CadDocument,
    table: &Table,
    style: &TableStyle,
) -> Result<DwgTable, Error> {
    let style_handle = style_handle(drawing, &style.name).ok_or_else(|| {
        Error::new(
            ErrorCode::DwgWrite,
            format!(
                "a table draws with table style `{}`, which the drawing does not hold",
                style.name
            ),
        )
    })?;
    let mut written = DwgTable::new(
        vector(table.position),
        table.rows.len(),
        table.columns.len(),
    );
    written.table_style_handle = Some(style_handle);
    for (column, width) in written.columns.iter_mut().zip(&table.columns) {
        column.width = *width;
    }
    for (r, (row, height)) in written.rows.iter_mut().zip(&table.rows).enumerate() {
        row.height = *height;
        row.style_id = style_id(table.row_kind(r));
        for (cell, ours) in row.cells.iter_mut().zip(&table.cells[r]) {
            *cell = dwg_cell(drawing, ours);
        }
    }
    for (r, row) in table.cells.iter().enumerate() {
        for c in 0..row.len() {
            if let Some([down, across]) = table.span(r, c) {
                written.merge_cells(CellRange::new(r, c, r + down - 1, c + across - 1));
            }
        }
    }
    Ok(written)
}

fn dwg_cell(drawing: &CadDocument, cell: &TableCell) -> DwgCell {
    let mut written = if cell.text.is_empty() && cell.style.is_none() && cell.height.is_none() {
        DwgCell::new()
    } else {
        DwgCell::text(&mtext_code(&cell.text, None))
    };
    if let Some(content) = written.contents.first_mut() {
        content.format_override_flags = TEXT_CONTENT;
        if let Some(name) = &cell.style {
            content.format_override_flags |= Override::TEXT_STYLE.bits() as i32;
            content.text_style_handle = drawing.text_styles.get(name).map(|s| s.handle);
            content.text_style_name.clone_from(name);
        }
        if let Some(height) = cell.height {
            content.format_override_flags |= Override::TEXT_HEIGHT.bits() as i32;
            content.text_height = height;
        }
    }
    if let Some(justify) = cell.justify {
        let mut own = CellStyle::new();
        own.override_flags = Override::ALIGNMENT.bits() as i32;
        own.alignment = alignment(justify) as i32;
        written.style = Some(own);
    }
    written
}

/// Fills the block of the table at `handle`, which the drawing holds, with
/// what it draws: acadrust's lines and texts, each text in the text style
/// its cell draws with and each line in the border pen of `style`, as
/// AutoCAD draws them.
pub(super) fn fill_block(
    drawing: &mut CadDocument,
    handle: Handle,
    table: &Table,
    style: &TableStyle,
) -> Result<(), Error> {
    let failed = || {
        Error::new(
            ErrorCode::DwgWrite,
            "the DWG library could not make the block a table draws".to_owned(),
        )
    };
    if !drawing.refresh_table_block(handle) {
        return Err(failed());
    }
    let Some(EntityType::Table(written)) = drawing.get_entity(handle) else {
        return Err(failed());
    };
    let record = written.block_record_handle.ok_or_else(failed)?;
    let members = drawing
        .block_records
        .iter()
        .find(|r| r.handle == record)
        .map(|r| r.entity_handles.clone())
        .ok_or_else(failed)?;
    // The block is in the table's own coordinates: its top left corner at
    // the origin.
    let origin = table.position;
    let laid = table.laid_out(style);
    for member in members {
        match drawing.get_entity_mut(member) {
            Some(EntityType::MText(text)) => {
                let at = [
                    text.insertion_point.x + origin.x,
                    text.insertion_point.y + origin.y,
                ];
                let cell = laid.iter().find(|cell| {
                    cell.text.is_some()
                        && (cell.x[0]..=cell.x[1]).contains(&at[0])
                        && (cell.y[0]..=cell.y[1]).contains(&at[1])
                });
                if let Some(text_style) = cell
                    .and_then(|cell| cell.text.as_ref())
                    .map(|t| t.style.clone())
                {
                    text.style = text_style;
                }
            }
            Some(EntityType::Line(line)) if !line.common.invisible => {
                line.common.color = dwg_color(style.border_color);
                line.common.line_weight = dwg_lineweight(style.border_lineweight);
            }
            _ => {}
        }
    }
    Ok(())
}

/// The drawing's table styles the core has a model for, and how many it
/// has not.
pub(super) fn read_table_styles(drawing: &CadDocument) -> (Vec<TableStyle>, usize) {
    let mut styles = Vec::new();
    let mut not_read = 0;
    let mut written: Vec<&DwgTableStyle> = drawing
        .objects
        .values()
        .filter_map(|object| match object {
            ObjectType::TableStyle(style) => Some(style),
            _ => None,
        })
        .collect();
    written.sort_by_key(|style| style.handle.value());
    for style in written {
        match read_table_style(drawing, style) {
            Some(style) => styles.push(style),
            None => not_read += 1,
        }
    }
    (styles, not_read)
}

fn read_table_style(drawing: &CadDocument, written: &DwgTableStyle) -> Option<TableStyle> {
    let rows = [
        &written.title_row_style,
        &written.header_row_style,
        &written.data_row_style,
    ];
    // Every border of every row in one pen, shown.
    let first = &rows[0].top_border;
    let borders = |row: &RowCellStyle| {
        [
            row.top_border.clone(),
            row.bottom_border.clone(),
            row.left_border.clone(),
            row.right_border.clone(),
            row.horizontal_inside_border.clone(),
            row.vertical_inside_border.clone(),
        ]
    };
    let one_pen = rows.iter().flat_map(|row| borders(row)).all(|border| {
        !border.is_invisible
            && border.color == first.color
            && border.line_weight == first.line_weight
    });
    if !one_pen || first.is_invisible {
        return None;
    }
    let border_color = match first.color {
        Color::ByLayer => ObjectColor::ByLayer,
        Color::ByBlock => ObjectColor::ByBlock,
        Color::Index(index) if index >= 1 => ObjectColor::Index(index),
        _ => return None,
    };
    let border_lineweight = match first.line_weight {
        LineWeight::ByLayer => ObjectLineweight::ByLayer,
        LineWeight::ByBlock => ObjectLineweight::ByBlock,
        LineWeight::Default => ObjectLineweight::Default,
        LineWeight::Value(hundredths) => {
            ObjectLineweight::Width(Lineweight::from_hundredths(hundredths)?)
        }
    };
    let row = |row: &RowCellStyle| -> Option<RowStyle> {
        if row.fill_enabled || row.text_color != Color::ByBlock || row.text_height <= 0.0 {
            return None;
        }
        let text_style = row
            .text_style_handle
            .and_then(|h| drawing.text_styles.iter().find(|s| s.handle == h))
            .map(|s| s.name.clone())
            .or_else(|| (!row.text_style_name.is_empty()).then(|| row.text_style_name.clone()))
            .unwrap_or_else(|| STANDARD.to_owned());
        Some(RowStyle {
            text_style,
            text_height: row.text_height,
            justify: justify(row.alignment as i32)?,
        })
    };
    if written.flow_direction != acadrust::objects::TableFlowDirection::Down {
        return None;
    }
    Some(TableStyle {
        name: written.name.clone(),
        horizontal_margin: written.horizontal_margin,
        vertical_margin: written.vertical_margin,
        border_color,
        border_lineweight,
        title: row(rows[0])?,
        header: row(rows[1])?,
        data: row(rows[2])?,
    })
}

/// The table's block, which holds what it draws and is read with it.
pub(super) fn block_of(written: &DwgTable) -> (Option<Handle>, &str) {
    (written.block_record_handle, written.block_name.as_str())
}

/// One of our tables, if the DWG table has a model in the core: flat and
/// running along X, in one of `styles`, each cell holding text or nothing
/// with no override but a text style, a height and a justification, and
/// its rows a title, a header and data rows in that order.
pub(super) fn read_table(
    drawing: &CadDocument,
    written: &DwgTable,
    styles: &[TableStyle],
    layer: String,
) -> Option<Table> {
    if written.normal != Vector3::UNIT_Z
        || written.horizontal_direction != Vector3::new(1.0, 0.0, 0.0)
        || written.rows.is_empty()
        || written.columns.is_empty()
        || !written.break_data.is_empty()
    {
        return None;
    }
    let style_name = match written.table_style_handle {
        Some(handle) => match drawing.objects.get(&handle) {
            Some(ObjectType::TableStyle(style)) => style.name.clone(),
            _ => return None,
        },
        None => STANDARD_TABLE_STYLE.to_owned(),
    };
    styles.iter().find(|s| s.name == style_name)?;
    let kinds: Vec<i32> = written.rows.iter().map(|row| row.style_id).collect();
    let (title, header) = row_kinds(&kinds)?;
    let mut cells = Vec::with_capacity(written.rows.len());
    for row in &written.rows {
        if row.cells.len() != written.columns.len() || row.height <= 0.0 {
            return None;
        }
        let read: Option<Vec<TableCell>> =
            row.cells.iter().map(|c| read_cell(drawing, c)).collect();
        cells.push(read?);
    }
    let mut table = Table {
        position: point(written.insertion_point),
        columns: written.columns.iter().map(|c| c.width).collect(),
        rows: written.rows.iter().map(|r| r.height).collect(),
        cells,
        title,
        header,
        table_style: style_name,
        layer,
        properties: Default::default(),
    };
    if table.columns.iter().any(|w| *w <= 0.0) {
        return None;
    }
    let mut merged = written.merged_ranges.clone();
    if merged.is_empty() {
        let mut from_cells = written.clone();
        from_cells.sync_merged_ranges_from_cells();
        merged = from_cells.merged_ranges;
    }
    for range in merged {
        let cell = table
            .cells
            .get_mut(range.top_row)?
            .get_mut(range.left_col)?;
        cell.merge = Some([range.row_count(), range.col_count()]);
    }
    // The cells a merge spans are empty, as a sheet writes them.
    let owners = table.owners();
    for (r, row) in table.cells.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            if owners[r][c] != [r, c] && *cell != TableCell::default() {
                return None;
            }
        }
    }
    Some(table)
}

/// Whether a table's first row is a title and the next a header, from the
/// row cell style each row draws with: a title first, if any, then a
/// header, if any, then data rows.
fn row_kinds(kinds: &[i32]) -> Option<(bool, bool)> {
    let title = kinds.first() == Some(&style_id(RowKind::Title));
    let after = usize::from(title);
    let header = kinds.get(after) == Some(&style_id(RowKind::Header));
    let data_from = after + usize::from(header);
    kinds[data_from..]
        .iter()
        .all(|k| *k == style_id(RowKind::Data))
        .then_some((title, header))
}

fn read_cell(drawing: &CadDocument, cell: &DwgCell) -> Option<TableCell> {
    let laid = cell.binary_layout();
    let cell = laid.as_ref();
    if cell.contents.len() > 1 || cell.cell_type != acadrust::entities::table::CellType::Text {
        return None;
    }
    let mut read = TableCell::default();
    if let Some(content) = cell.contents.first() {
        if content.field_handle.is_some() || content.block_handle.is_some() {
            return None;
        }
        let own = content.format_override_flags as u32 | content.format_property_flags as u32;
        let known =
            TEXT_CONTENT as u32 | Override::TEXT_STYLE.bits() | Override::TEXT_HEIGHT.bits();
        if own & !known != 0 {
            return None;
        }
        let (text, width_factor) = mtext_contents(content.value.display())?;
        if width_factor.is_some() {
            return None;
        }
        read.text = text;
        if content.sets(Override::TEXT_STYLE) {
            let name = content
                .text_style_handle
                .and_then(|h| drawing.text_styles.iter().find(|s| s.handle == h))
                .map(|s| s.name.clone())
                .or_else(|| {
                    (!content.text_style_name.is_empty()).then(|| content.text_style_name.clone())
                })?;
            read.style = Some(name);
        }
        if content.sets(Override::TEXT_HEIGHT) {
            if content.text_height <= 0.0 {
                return None;
            }
            read.height = Some(content.text_height);
        }
    }
    if let Some(own) = &cell.style {
        let overridden = own.overridden();
        if overridden.bits() & !Override::ALIGNMENT.bits() != 0 {
            return None;
        }
        if overridden.contains(Override::ALIGNMENT) {
            read.justify = Some(justify(own.alignment)?);
        }
    }
    Some(read)
}
