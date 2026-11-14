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
//! border colour and lineweight, every text and line on the table's own
//! layer, the texts `ByBlock` in colour and `ByLayer` in lineweight. So once
//! acadrust has filled the block, each of its texts is given the text style
//! its cell draws with, and each of its lines the border pen, all on the
//! table's layer, as AutoCAD 2026 wrote them laying out made-up tables.
//! Each text is matched to its cell by where the layout attaches that
//! cell's text, which is where acadrust puts it.
//!
//! A cell's text is coded as multiline text is, and its own text style,
//! height and justification are written as AutoCAD 2026 wrote them on a
//! made-up table: a style and a height as overrides of its content, a
//! justification as an override of its cell style.
//!
//! A table is read back when it has a model in the core: flat on the XY
//! plane and running along X, in a table style the core read, each cell
//! holding text or nothing, with no override but a text style, a height and
//! a justification, and no override of the table's own, a row's or a
//! column's at all. A table style is read back when every cell style it
//! holds, the R2010 `_TITLE`, `_HEADER` and `_DATA` and any other, draws as
//! the model does: no fill, no turned text, its text `ByBlock`, every
//! border in the one pen and the style's margins.

use acadrust::entities::table::{CellRange, CellStyle, CellStylePropertyFlags as Override};
use acadrust::entities::{EntityType, Table as DwgTable, TableCell as DwgCell};
use acadrust::objects::{
    CellAlignment, NamedTableCellStyle, ObjectType, RowCellStyle, TableBorderType, TableCellBorder,
    TableCellStyleData, TableStyle as DwgTableStyle,
};
use acadrust::types::{Color, Handle, LineWeight, Vector3};
use acadrust::CadDocument;

use super::text::{attachment_of, mtext_code, mtext_contents};
use super::{point, vector};
use crate::error::{Error, ErrorCode};
use crate::model::{
    LaidCell, Lineweight, ObjectColor, ObjectLineweight, RowKind, RowStyle, Sheet, Table,
    TableCell, TableStyle, STANDARD_TABLE_STYLE,
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
    // How far a text's point may be from where the layout puts it: a
    // billionth of the table's reach, room for the rounding of sums made
    // from another corner.
    let reach = origin.x.abs()
        + origin.y.abs()
        + table.columns.iter().sum::<f64>()
        + table.rows.iter().sum::<f64>();
    let tolerance = 1e-9 * (1.0 + reach);
    for member in members {
        match drawing.get_entity_mut(member) {
            Some(EntityType::MText(text)) => {
                let justify = attachment_of(text.attachment_point).ok_or_else(failed)?;
                let at = [
                    text.insertion_point.x + origin.x,
                    text.insertion_point.y + origin.y,
                ];
                let cell = cell_of(&laid, at, justify, tolerance).ok_or_else(failed)?;
                if let Some(cell_text) = &cell.text {
                    text.style.clone_from(&cell_text.style);
                }
                table.layer.clone_into(&mut text.common.layer);
                text.common.color = Color::ByBlock;
                text.common.line_weight = LineWeight::ByLayer;
            }
            Some(EntityType::Line(line)) if !line.common.invisible => {
                table.layer.clone_into(&mut line.common.layer);
                line.common.color = dwg_color(style.border_color);
                line.common.line_weight = dwg_lineweight(style.border_lineweight);
            }
            _ => {}
        }
    }
    Ok(())
}

/// The cell whose text acadrust placed at `at`, attached at `justify`: the
/// one cell with text so justified whose text the layout attaches at `at`,
/// within `tolerance`. The layout and acadrust place a cell's text alike,
/// from the cell's box, its margins and its justification, so a text on an
/// edge two cells share, or in a row lower than its margins, is found all
/// the same. None when no cell, or more than one, is there.
fn cell_of(
    laid: &[LaidCell],
    at: [f64; 2],
    justify: Attachment,
    tolerance: f64,
) -> Option<&LaidCell> {
    let mut found = laid.iter().filter(|cell| {
        cell.text.as_ref().is_some_and(|t| {
            t.justify == justify && (t.position.x - at[0]).hypot(t.position.y - at[1]) <= tolerance
        })
    });
    let cell = found.next()?;
    found.next().is_none().then_some(cell)
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
    let read = TableStyle {
        name: written.name.clone(),
        horizontal_margin: written.horizontal_margin,
        vertical_margin: written.vertical_margin,
        border_color,
        border_lineweight,
        title: row(rows[0])?,
        header: row(rows[1])?,
        data: row(rows[2])?,
    };
    // The R2010 cell styles, which AutoCAD draws from, hold no more than
    // the legacy rows the style was read from.
    let first_pen = (first.color, first.line_weight);
    if let Some(base) = &written.modern_style {
        modern_draws_as_read(drawing, base, &read, first_pen, None)?;
    }
    for (_, named) in &written.modern_overrides {
        let kind = match named.name.as_str() {
            "_TITLE" => Some(&read.title),
            "_HEADER" => Some(&read.header),
            "_DATA" => Some(&read.data),
            _ => None,
        };
        modern_draws_as_read(drawing, named, &read, first_pen, kind)?;
    }
    Some(read)
}

/// Whether an R2010 cell style of a table style draws as the style the core
/// read, `read`: no fill, no turned text, text `ByBlock` and laid out in a
/// flow, every border shown, single, continuous and in the one pen `pen`,
/// and the style's margins. A cell style of a kind of row also has that
/// row's text style, height and justification, `row`. None when it does not.
fn modern_draws_as_read(
    drawing: &CadDocument,
    named: &NamedTableCellStyle,
    read: &TableStyle,
    pen: (Color, LineWeight),
    row: Option<&RowStyle>,
) -> Option<()> {
    let style: &TableCellStyleData = &named.cell_style;
    if style.data_flags == 0 {
        return Some(());
    }
    let format = &style.content_format;
    let plain = matches!(style.background_color, Color::None)
        && style.content_layout == 1
        && format.rotation == 0.0
        && matches!(format.content_color, Color::ByBlock);
    if !plain {
        return None;
    }
    if style.margin_override_flags != 0 {
        let margins = [
            style.horizontal_margin,
            style.right_margin,
            style.vertical_margin,
            style.bottom_margin,
        ];
        let want = [
            read.horizontal_margin,
            read.horizontal_margin,
            read.vertical_margin,
            read.vertical_margin,
        ];
        if margins != want {
            return None;
        }
    }
    // acadrust keeps an R2010 border's flag the other way round from a
    // legacy row's: `is_invisible` set on a border AutoCAD draws.
    let borders_drawn_in_pen = style.borders.iter().all(|grid| {
        grid.index_mask == 0
            || (grid.border.is_invisible
                && grid.border.border_type == TableBorderType::Single
                && grid.line_type.is_null()
                && (grid.border.color, grid.border.line_weight) == pen)
    });
    if !borders_drawn_in_pen {
        return None;
    }
    if let Some(row) = row {
        let text_style = drawing
            .text_styles
            .iter()
            .find(|s| s.handle == format.text_style)
            .map(|s| s.name.as_str())
            .unwrap_or(STANDARD);
        let same = format.text_height == row.text_height
            && justify(format.cell_alignment) == Some(row.justify)
            && (format.text_style.is_null() || text_style == row.text_style);
        if !same {
            return None;
        }
    }
    Some(())
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
    // An override of the table's own, a row's or a column's draws what the
    // model has no place for, such as a fill.
    let table_overrides = written.override_flag
        || written.override_border_color
        || written.override_border_line_weight
        || written.override_border_visibility
        || written.legacy_style_override.is_some()
        || written.legacy_border_colors.is_some()
        || written.legacy_border_line_weights.is_some()
        || written.legacy_border_visibility.is_some();
    let scopes = std::iter::once(&written.base_style)
        .chain(written.rows.iter().map(|row| &row.style))
        .chain(written.columns.iter().map(|column| &column.style));
    if table_overrides
        || scopes
            .flatten()
            .any(|style| overrides_beyond(style, Override::NONE))
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
    if cell.style_id != 0 {
        return None;
    }
    if let Some(own) = &cell.style {
        if overrides_beyond(own, Override::ALIGNMENT) {
            return None;
        }
        if own.overridden().contains(Override::ALIGNMENT) {
            read.justify = Some(justify(own.alignment)?);
        }
    }
    Some(read)
}

/// Whether a table's, a row's, a column's or a cell's own style overrides
/// anything but `allowed`: a property, its content's format, a margin, or a
/// border.
fn overrides_beyond(style: &CellStyle, allowed: Override) -> bool {
    let borders = [
        &style.top_border,
        &style.right_border,
        &style.bottom_border,
        &style.left_border,
    ];
    !(style.overridden() - allowed).is_empty()
        || style.content_format_override_flags != 0
        || style.margin_override_flags != 0
        || style.fill_enabled
        || !style.applied_border_edges.is_empty()
        || !style.additional_borders.is_empty()
        || borders.iter().any(|b| !b.override_flags.is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;

    use acadrust::entities::table::{CellStyle, CellStylePropertyFlags as Override};
    use acadrust::entities::EntityType;
    use acadrust::objects::ObjectType;
    use acadrust::types::{Color, LineWeight};
    use acadrust::{CadDocument, DwgReader, DwgWriter};

    use crate::build::{build, BuildRequest};
    use crate::model::{DrawingContents, Object};

    /// A change made to a drawing's table, or to one of a table style's
    /// R2010 cell styles, with what it is called.
    type TableChange = (&'static str, fn(&mut acadrust::entities::Table));
    type CellStyleChange = (&'static str, fn(&mut acadrust::objects::TableCellStyleData));

    /// An empty folder for one test under the system's temporary folder.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-dwg-table-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A made-up sheet of one table on layer `list`, in table style `s`.
    const SHEET: &str = "units: mm
layers:
  list: {color: green, lineweight: 0.18}
text_styles:
  A: {font: arial.ttf}
  B: {font: romans.shx}
table_styles:
  s:
    text_height: 2.5
    horizontal_margin: 0
    vertical_margin: 1
    data: {justify: top_left}
objects:
  - table:
      position: [0, 50, 0]
      columns: [20, 20]
      rows: [10, 10]
      table_style: s
      layer: list
      cells:
        - [{text: LEFT, style: A}, {text: RIGHT, style: B}]
        - [ONE, TWO]
";

    /// Builds `text` in a folder of its own, and gives the folder and the
    /// drawing the DWG holds.
    fn built(test: &str, text: &str) -> (PathBuf, CadDocument) {
        let dir = scratch(test);
        let description = dir.join("sheet.yaml");
        fs::write(&description, text).unwrap();
        build(&BuildRequest {
            description,
            output: None,
            overwrite: false,
        })
        .unwrap_or_else(|e| panic!("{e}"));
        let bytes = fs::read(dir.join("sheet.dwg")).unwrap();
        let drawing = DwgReader::from_stream(Cursor::new(bytes)).read().unwrap();
        (dir, drawing)
    }

    /// Writes `drawing` beside the built DWG and reads it as the core does.
    fn read_back(dir: &std::path::Path, drawing: &CadDocument) -> DrawingContents {
        let path = dir.join("changed.dwg");
        fs::write(&path, DwgWriter::write_to_vec(drawing).unwrap()).unwrap();
        super::super::read(&path).unwrap()
    }

    fn table_of(drawing: &mut CadDocument) -> &mut acadrust::entities::Table {
        let handle = drawing
            .model_space_entities()
            .find_map(|e| match e {
                EntityType::Table(t) => Some(t.common.handle),
                _ => None,
            })
            .unwrap();
        match drawing.get_entity_mut(handle) {
            Some(EntityType::Table(t)) => t,
            _ => unreachable!(),
        }
    }

    /// A style that fills its cells red: a background the model has no
    /// place for.
    fn filled() -> CellStyle {
        let mut style = CellStyle::new();
        style.property_flags = Override::BACKGROUND_COLOR;
        style.background_color = Color::Index(1);
        style.fill_enabled = true;
        style
    }

    #[test]
    fn a_table_rewritten_as_it_was_reads_back() {
        let (dir, drawing) = built("as-it-was", SHEET);
        let read = read_back(&dir, &drawing);
        assert_eq!(read.objects_not_read, 0, "{read:?}");
        assert!(matches!(read.objects[0], Object::Table(_)));
        assert_eq!(read.table_styles_not_read, 0);
    }

    #[test]
    fn a_fill_of_the_table_a_row_or_a_column_is_not_read() {
        let scopes: [TableChange; 3] = [
            ("table", |t| t.base_style = Some(filled())),
            ("row", |t| t.rows[1].style = Some(filled())),
            ("column", |t| t.columns[0].style = Some(filled())),
        ];
        for (scope, fill) in scopes {
            let (dir, mut drawing) = built(&format!("fill-{scope}"), SHEET);
            fill(table_of(&mut drawing));
            let read = read_back(&dir, &drawing);
            assert_eq!(read.objects_not_read, 1, "a filled {scope}: {read:?}");
            assert!(read.objects.is_empty(), "a filled {scope}");
        }
    }

    #[test]
    fn a_table_style_whose_cell_styles_draw_more_than_the_model_is_not_read() {
        let changes: [CellStyleChange; 3] = [
            ("turned", |c| {
                c.content_format.rotation = std::f64::consts::FRAC_PI_2;
            }),
            ("filled", |c| c.background_color = Color::Index(2)),
            ("red text", |c| {
                c.content_format.content_color = Color::Index(1)
            }),
        ];
        for (what, change) in changes {
            let (dir, mut drawing) = built(&format!("style-{}", what.replace(' ', "-")), SHEET);
            let style = drawing
                .objects
                .values_mut()
                .find_map(|o| match o {
                    ObjectType::TableStyle(s) if s.name == "s" => Some(s),
                    _ => None,
                })
                .unwrap();
            let data = style
                .modern_overrides
                .iter_mut()
                .find(|(_, n)| n.name == "_DATA")
                .expect("the R2010 data cell style");
            change(&mut data.1.cell_style);
            let read = read_back(&dir, &drawing);
            assert!(
                !read.table_styles.iter().any(|s| s.name == "s"),
                "{what}: {:?}",
                read.table_styles
            );
            assert_eq!(read.table_styles_not_read, 1, "{what}");
            assert_eq!(
                read.objects_not_read, 1,
                "{what}: the table in it is not read"
            );
        }
    }

    /// Each text of the table's block: what it says, its text style, layer,
    /// colour and lineweight.
    fn block_texts(drawing: &CadDocument) -> Vec<(String, String, String, Color, LineWeight)> {
        let names: Vec<String> = drawing
            .block_records
            .iter()
            .filter(|r| r.name.starts_with("*T"))
            .map(|r| r.name.clone())
            .collect();
        names
            .iter()
            .flat_map(|name| drawing.entities_in_block(name))
            .filter_map(|e| match e {
                EntityType::MText(m) => Some((
                    m.value.clone(),
                    m.style.clone(),
                    m.common.layer.clone(),
                    m.common.color,
                    m.common.line_weight,
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_text_on_an_edge_two_cells_share_takes_its_own_cells_style() {
        // With no side margin, the right cell's top left text sits on the
        // edge it shares with the left cell.
        let (_, drawing) = built("shared-edge", SHEET);
        let texts = block_texts(&drawing);
        let style_of = |says: &str| {
            texts
                .iter()
                .find(|t| t.0 == says)
                .map(|t| t.1.clone())
                .unwrap()
        };
        assert_eq!(style_of("LEFT"), "A");
        assert_eq!(style_of("RIGHT"), "B");
        assert_eq!(style_of("TWO"), "Standard");
    }

    /// The text style each text of the table's block draws with, by what
    /// it says.
    fn styles_by_text(drawing: &CadDocument) -> Vec<(String, String)> {
        let mut styles: Vec<(String, String)> = block_texts(drawing)
            .into_iter()
            .map(|(says, style, ..)| (says, style))
            .collect();
        styles.sort();
        styles
    }

    #[test]
    fn a_row_lower_than_its_margins_builds_with_its_text_in_the_block() {
        // Standard's margins are 1.5 mm, so a row 2 high puts its top
        // centre text 0.5 above its bottom edge.
        let (_, drawing) = built(
            "short-row",
            "units: mm
objects:
  - table:
      position: [0, 10, 0]
      columns: [20]
      rows: [2]
      cells: [[A]]
",
        );
        assert_eq!(
            styles_by_text(&drawing),
            [("A".to_owned(), "Standard".to_owned())]
        );
    }

    #[test]
    fn a_merged_cell_across_a_row_lower_than_both_margins_builds() {
        // The second row is 2 high, less than the 3 its margins take; the
        // first column's cell spans both rows.
        let (_, drawing) = built(
            "short-merged",
            "units: mm
text_styles:
  B: {font: romans.shx}
objects:
  - table:
      position: [0, 10, 0]
      columns: [20, 20]
      rows: [2.5, 2]
      cells:
        - [{text: M, merge: [2, 1]}, X]
        - ['', {text: Y, style: B}]
",
        );
        assert_eq!(
            styles_by_text(&drawing),
            [
                ("M".to_owned(), "Standard".to_owned()),
                ("X".to_owned(), "Standard".to_owned()),
                ("Y".to_owned(), "B".to_owned()),
            ]
        );
    }

    #[test]
    fn the_blocks_texts_and_lines_are_written_as_autocad_lays_a_table_out() {
        // AutoCAD 2026, laying a made-up table out, put each of its block's
        // texts and lines on the table's layer, the texts ByBlock in colour
        // and ByLayer in lineweight.
        let (_, drawing) = built("block-pens", SHEET);
        for (says, _, layer, color, lineweight) in block_texts(&drawing) {
            assert_eq!(layer, "list", "{says}");
            assert_eq!(color, Color::ByBlock, "{says}");
            assert_eq!(lineweight, LineWeight::ByLayer, "{says}");
        }
        let name = drawing
            .block_records
            .iter()
            .find(|r| r.name.starts_with("*T"))
            .unwrap()
            .name
            .clone();
        for e in drawing.entities_in_block(&name) {
            if let EntityType::Line(line) = e {
                if !line.common.invisible {
                    assert_eq!(line.common.layer, "list");
                }
            }
        }
    }
}
