//! Tables and table styles: AutoCAD's TABLE and TABLESTYLE.
//!
//! A table is a grid of cells hung from its top left corner, its rows
//! running down and its columns across, as AutoCAD's TABLE draws one by
//! default. Each row and column is as high and as wide as the sheet says;
//! each cell holds text, laid out as AutoCAD lays a cell's text out, as
//! multiline text inside the cell's margins. A cell may span the cells
//! below it and to its right, merged as AutoCAD merges them.
//!
//! A table style is what the table's cells draw with unless they give
//! their own: the text style, text height and justification of each kind
//! of row, title, header and data, the cells' margins, and the pen of the
//! borders. AutoCAD 2026 laid out made-up tables so: each cell's text is
//! multiline text as wide as its cell less both side margins, at its
//! cell's left margin, middle or right margin across, and its top margin,
//! middle or bottom margin up, as its justification says; the borders are
//! the grid's lines, every row's boundary and every column's, with the
//! inside of a merged cell left out.

use serde::{Deserialize, Serialize};

use super::{is_false, layer_zero, ObjectColor, ObjectLineweight, Point3, Properties};
use crate::text::{Attachment, MText, STANDARD};
use crate::units::Unit;

/// The table style every drawing has, which a table draws with unless it
/// names another.
pub const STANDARD_TABLE_STYLE: &str = STANDARD;

/// A grid of cells with text, as AutoCAD's TABLE draws it: hung from its
/// top left corner, its rows running down and its columns across.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Table {
    /// The table's top left corner, where AutoCAD inserts it.
    pub position: Point3,
    /// The width of each column, left to right, each more than 0.
    pub columns: Vec<f64>,
    /// The height of each row, top to bottom, each more than 0.
    pub rows: Vec<f64>,
    /// The cells, row by row from the top, each row one cell for each
    /// column from the left.
    pub cells: Vec<Vec<TableCell>>,
    /// Whether the first row is a title row, drawn as the table style's
    /// title rows are.
    #[serde(default, skip_serializing_if = "is_false")]
    pub title: bool,
    /// Whether the row after the title, or the first row when there is no
    /// title, is a header row, drawn as the table style's header rows are.
    #[serde(default, skip_serializing_if = "is_false")]
    pub header: bool,
    /// The table style it draws with.
    #[serde(default = "standard_table_style")]
    pub table_style: String,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

fn standard_table_style() -> String {
    STANDARD_TABLE_STYLE.to_owned()
}

/// One cell of a table: its text, and what it draws its text with where
/// it does not take its row's.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    /// What the cell says, exactly as it shows; lines are separated by a
    /// line break (`\n`). Empty for an empty cell.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// The text style its text draws with, instead of its row's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// The height of its text's capitals, instead of its row's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Where its text sits in it, instead of where its row's sits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justify: Option<Attachment>,
    /// How many rows and columns the cell spans, `[rows, columns]`, itself
    /// among them, when it is merged with the cells below it and to its
    /// right. The cells it spans are empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<[usize; 2]>,
}

/// The kind of a table's row, which says how its cells draw by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    Title,
    Header,
    Data,
}

impl RowKind {
    /// Every kind, in the order a table style lists them.
    pub const ALL: [RowKind; 3] = [RowKind::Title, RowKind::Header, RowKind::Data];

    /// The name a table style writes, such as `title`.
    pub const fn name(self) -> &'static str {
        match self {
            RowKind::Title => "title",
            RowKind::Header => "header",
            RowKind::Data => "data",
        }
    }
}

/// What the cells of one kind of row draw their text with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RowStyle {
    /// The text style.
    pub text_style: String,
    /// The height of the capitals, more than 0.
    pub text_height: f64,
    /// Where the text sits in its cell.
    pub justify: Attachment,
}

/// What a table's cells draw with unless they give their own: AutoCAD's
/// TABLESTYLE.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableStyle {
    pub name: String,
    /// The space between a cell's sides and its text, left and right.
    pub horizontal_margin: f64,
    /// The space between a cell's top and bottom and its text.
    pub vertical_margin: f64,
    /// The colour the borders draw in: `ByBlock`, the table's own, unless
    /// the style gives another.
    #[serde(default = "by_block_color")]
    pub border_color: ObjectColor,
    /// The lineweight the borders draw with: `ByBlock`, the table's own,
    /// unless the style gives another.
    #[serde(default = "by_block_lineweight")]
    pub border_lineweight: ObjectLineweight,
    pub title: RowStyle,
    pub header: RowStyle,
    pub data: RowStyle,
}

fn by_block_color() -> ObjectColor {
    ObjectColor::ByBlock
}

fn by_block_lineweight() -> ObjectLineweight {
    ObjectLineweight::ByBlock
}

impl TableStyle {
    /// The `Standard` table style a sheet in `units` has unless it lists
    /// its own: on a metric sheet a title 6 mm high, header and data rows
    /// 4.5 mm and margins of 1.5 mm, and on an imperial one 0.25, 0.18 and
    /// 0.06 in, all in the `Standard` text style; titles and headers
    /// middle centre and data top centre, and borders `ByBlock`, as
    /// AutoCAD's own Standard table style has them.
    pub fn standard(units: Unit) -> Self {
        let (title, rest, margin) = if units.is_metric() {
            let mm = |x: f64| Unit::Mm.convert(x, units);
            (mm(6.0), mm(4.5), mm(1.5))
        } else {
            let inch = |x: f64| Unit::In.convert(x, units);
            (inch(0.25), inch(0.18), inch(0.06))
        };
        Self::with(STANDARD_TABLE_STYLE, STANDARD, [title, rest, rest], margin)
    }

    /// A style named `name` whose rows draw in `text_style` at the heights
    /// `[title, header, data]`, with margins of `margin` both ways, and
    /// AutoCAD's defaults otherwise.
    pub(crate) fn with(name: &str, text_style: &str, heights: [f64; 3], margin: f64) -> Self {
        let row = |height: f64, justify: Attachment| RowStyle {
            text_style: text_style.to_owned(),
            text_height: height,
            justify,
        };
        Self {
            name: name.to_owned(),
            horizontal_margin: margin,
            vertical_margin: margin,
            border_color: ObjectColor::ByBlock,
            border_lineweight: ObjectLineweight::ByBlock,
            title: row(heights[0], Attachment::MiddleCenter),
            header: row(heights[1], Attachment::MiddleCenter),
            data: row(heights[2], Attachment::TopCenter),
        }
    }

    /// The style of a kind of row.
    pub fn row(&self, kind: RowKind) -> &RowStyle {
        match kind {
            RowKind::Title => &self.title,
            RowKind::Header => &self.header,
            RowKind::Data => &self.data,
        }
    }

    pub(crate) fn row_mut(&mut self, kind: RowKind) -> &mut RowStyle {
        match kind {
            RowKind::Title => &mut self.title,
            RowKind::Header => &mut self.header,
            RowKind::Data => &mut self.data,
        }
    }
}

/// A cell's place in its table: its row and its column, counted from 0
/// from the top left.
pub type CellAt = [usize; 2];

/// One cell as its table lays it out: where it is, the box it covers,
/// merged cells and all, and its text as it draws.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidCell {
    /// Its row and column.
    pub at: CellAt,
    /// Its left and right sides.
    pub x: [f64; 2],
    /// Its bottom and top.
    pub y: [f64; 2],
    /// Its text, as multiline text in the cell's margins, or none when
    /// the cell is empty.
    pub text: Option<MText>,
    /// The table style's margins, across and up.
    pub margins: [f64; 2],
}

impl Table {
    /// The kind of row `row`.
    pub fn row_kind(&self, row: usize) -> RowKind {
        let title = usize::from(self.title);
        if self.title && row == 0 {
            RowKind::Title
        } else if self.header && row == title {
            RowKind::Header
        } else {
            RowKind::Data
        }
    }

    /// The x of each column's left side, and of the last one's right.
    pub fn column_edges(&self) -> Vec<f64> {
        let mut edges = vec![self.position.x];
        let mut x = self.position.x;
        for width in &self.columns {
            x += width;
            edges.push(x);
        }
        edges
    }

    /// The y of each row's top, and of the last one's bottom: the rows run
    /// down from the table's position.
    pub fn row_edges(&self) -> Vec<f64> {
        let mut edges = vec![self.position.y];
        let mut y = self.position.y;
        for height in &self.rows {
            y -= height;
            edges.push(y);
        }
        edges
    }

    /// For each square of the grid, the cell it belongs to: itself, or the
    /// merged cell that spans it.
    pub fn owners(&self) -> Vec<Vec<CellAt>> {
        let (rows, columns) = (self.rows.len(), self.columns.len());
        let mut owner: Vec<Vec<CellAt>> = (0..rows)
            .map(|r| (0..columns).map(|c| [r, c]).collect())
            .collect();
        for r in 0..rows {
            for c in 0..columns {
                let Some([down, across]) = self.span(r, c) else {
                    continue;
                };
                if owner[r][c] != [r, c] {
                    continue;
                }
                for row in owner.iter_mut().skip(r).take(down) {
                    for square in row.iter_mut().skip(c).take(across) {
                        *square = [r, c];
                    }
                }
            }
        }
        owner
    }

    /// How many rows and columns the cell at `row` and `column` spans,
    /// kept within the table, when it is merged; none when it is not.
    pub fn span(&self, row: usize, column: usize) -> Option<[usize; 2]> {
        let [down, across] = self.cells.get(row)?.get(column)?.merge?;
        let down = down.min(self.rows.len() - row).max(1);
        let across = across.min(self.columns.len() - column).max(1);
        (down > 1 || across > 1).then_some([down, across])
    }

    /// Every cell that is not spanned by another, laid out with `style`,
    /// row by row from the top left.
    pub fn laid_out(&self, style: &TableStyle) -> Vec<LaidCell> {
        let xs = self.column_edges();
        let ys = self.row_edges();
        let owners = self.owners();
        let mut laid = Vec::new();
        for (r, row) in self.cells.iter().enumerate().take(self.rows.len()) {
            for (c, cell) in row.iter().enumerate().take(self.columns.len()) {
                if owners[r][c] != [r, c] {
                    continue;
                }
                let [down, across] = self.span(r, c).unwrap_or([1, 1]);
                let x = [xs[c], xs[c + across]];
                let y = [ys[r + down], ys[r]];
                laid.push(LaidCell {
                    at: [r, c],
                    x,
                    y,
                    text: self.cell_text(cell, self.row_kind(r), x, y, style),
                    margins: [style.horizontal_margin, style.vertical_margin],
                });
            }
        }
        laid
    }

    /// A cell's text as AutoCAD lays it out: multiline text as wide as the
    /// cell less both side margins, at the left margin, the middle or the
    /// right margin across and the top margin, the middle or the bottom
    /// margin up, as its justification says.
    fn cell_text(
        &self,
        cell: &TableCell,
        kind: RowKind,
        x: [f64; 2],
        y: [f64; 2],
        style: &TableStyle,
    ) -> Option<MText> {
        if cell.text.is_empty() {
            return None;
        }
        let row = style.row(kind);
        let justify = cell.justify.unwrap_or(row.justify);
        let (h, v) = (style.horizontal_margin, style.vertical_margin);
        let at = Attachment::ALL
            .iter()
            .position(|a| *a == justify)
            .unwrap_or(0);
        let across = [x[0] + h, (x[0] + x[1]) / 2.0, x[1] - h][at % 3];
        let up = [y[1] - v, (y[0] + y[1]) / 2.0, y[0] + v][at / 3];
        Some(MText {
            contents: cell.text.clone(),
            position: Point3::new(across, up, self.position.z),
            height: cell.height.unwrap_or(row.text_height),
            width: (x[1] - x[0] - 2.0 * h).max(0.0),
            style: cell.style.clone().unwrap_or_else(|| row.text_style.clone()),
            width_factor: None,
            rotation: 0.0,
            justify,
            layer: self.layer.clone(),
            properties: self.properties.clone(),
        })
    }

    /// The borders as AutoCAD draws them: each row's boundary, top to
    /// bottom, as runs from left to right, then each column's boundary,
    /// left to right, as runs from top to bottom, leaving out the inside of
    /// each merged cell.
    pub fn borders(&self) -> Vec<[[f64; 2]; 2]> {
        let xs = self.column_edges();
        let ys = self.row_edges();
        let owner = self.owners();
        let (rows, columns) = (self.rows.len(), self.columns.len());
        let mut lines = Vec::new();
        for k in 0..=rows {
            let open = |c: usize| k == 0 || k == rows || owner[k - 1][c] != owner[k][c];
            let mut c = 0;
            while c < columns {
                if !open(c) {
                    c += 1;
                    continue;
                }
                let start = c;
                while c < columns && open(c) {
                    c += 1;
                }
                lines.push([[xs[start], ys[k]], [xs[c], ys[k]]]);
            }
        }
        for k in 0..=columns {
            let open = |r: usize| k == 0 || k == columns || owner[r][k - 1] != owner[r][k];
            let mut r = 0;
            while r < rows {
                if !open(r) {
                    r += 1;
                    continue;
                }
                let start = r;
                while r < rows && open(r) {
                    r += 1;
                }
                lines.push([[xs[k], ys[start]], [xs[k], ys[r]]]);
            }
        }
        lines
    }
}

/// The least height AutoCAD sets a cell's text in, as a share of the
/// text's height: the capitals and a third more. AutoCAD 2026 grew a row
/// of one line of text to this and both margins when it was lower.
pub const LINE_BOX: f64 = 4.0 / 3.0;

/// How much further down each line after the first reaches, as a share of
/// the text's height: AutoCAD's single line spacing. AutoCAD 2026 grew a
/// row of `n` lines to `1 + (n - 1) * 5/3` times the text's height and both
/// margins, for two to four lines of made-up text.
pub const LINE_STEP: f64 = 5.0 / 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(text: &str) -> TableCell {
        TableCell {
            text: text.to_owned(),
            ..TableCell::default()
        }
    }

    fn table() -> Table {
        let mut cells = vec![
            vec![cell("SCHEDULE"), cell(""), cell("")],
            vec![cell("TAG"), cell("FROM"), cell("TO")],
            vec![cell("C-1"), cell("JB-1"), cell("")],
        ];
        cells[0][0].merge = Some([1, 3]);
        Table {
            position: Point3::new(10.0, 100.0, 0.0),
            columns: vec![40.0, 25.0, 30.0],
            rows: vec![8.0, 6.0, 6.0],
            cells,
            title: true,
            header: true,
            table_style: STANDARD_TABLE_STYLE.to_owned(),
            layer: "0".to_owned(),
            properties: Properties::default(),
        }
    }

    #[test]
    fn rows_are_title_header_then_data() {
        let t = table();
        assert_eq!(t.row_kind(0), RowKind::Title);
        assert_eq!(t.row_kind(1), RowKind::Header);
        assert_eq!(t.row_kind(2), RowKind::Data);
        let plain = Table {
            title: false,
            ..t.clone()
        };
        assert_eq!(plain.row_kind(0), RowKind::Header);
        assert_eq!(plain.row_kind(1), RowKind::Data);
    }

    #[test]
    fn a_merged_cell_spans_and_its_inside_has_no_border() {
        let t = table();
        let laid = t.laid_out(&TableStyle::with("S", "Standard", [3.5, 2.5, 2.5], 1.0));
        // The title spans the three columns; the other six cells each one.
        assert_eq!(laid.len(), 7);
        assert_eq!(laid[0].x, [10.0, 105.0]);
        assert_eq!(laid[0].y, [92.0, 100.0]);
        let borders = t.borders();
        // Four row boundaries, each whole, then the column boundaries: the
        // outer two whole, the inner two under the title only.
        assert_eq!(
            borders,
            vec![
                [[10.0, 100.0], [105.0, 100.0]],
                [[10.0, 92.0], [105.0, 92.0]],
                [[10.0, 86.0], [105.0, 86.0]],
                [[10.0, 80.0], [105.0, 80.0]],
                [[10.0, 100.0], [10.0, 80.0]],
                [[50.0, 92.0], [50.0, 80.0]],
                [[75.0, 92.0], [75.0, 80.0]],
                [[105.0, 100.0], [105.0, 80.0]],
            ]
        );
    }

    #[test]
    fn a_cells_text_sits_in_its_margins_as_it_is_justified() {
        let t = table();
        let style = TableStyle::with("S", "Standard", [3.5, 2.5, 2.5], 1.0);
        let laid = t.laid_out(&style);
        // The title, middle centre of the merged row.
        let title = laid[0].text.as_ref().unwrap();
        assert_eq!(title.position, Point3::new(57.5, 96.0, 0.0));
        assert_eq!(title.width, 93.0);
        assert_eq!(title.height, 3.5);
        // Data top centre: its top margin, the middle across.
        let data = laid[4].text.as_ref().unwrap();
        assert_eq!(data.contents, "C-1");
        assert_eq!(data.position, Point3::new(30.0, 85.0, 0.0));
        assert_eq!(data.justify, Attachment::TopCenter);
        // An empty cell draws no text.
        assert!(laid[6].text.is_none());
    }
}
