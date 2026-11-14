//! Edits a table's grid in a record, as the table operations of a change
//! set ask: sets a cell's keys, and adds or deletes a row or a column,
//! keeping its merges, its title and its header in line.
//!
//! Cells, rows and columns count from 0 from the top left, `[row, column]`,
//! as the record's `cells[r][c]` does. Only the table's own nodes change;
//! every node the edit does not touch keeps where the file wrote it, so it
//! is written back exactly as it was.

use serde_json::{Map, Value};

use super::super::strict_yaml::{index_path, key_path, Entry, Kind, Node};
use super::super::table::CELL_KEYS;
use super::{new_entry, node_of, row_value_name};
use crate::error::{Error, ErrorCode};

/// Whether a row or column operation works down or across.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    Row,
    Column,
}

impl Axis {
    /// One of them, in words: `row` or `column`.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Axis::Row => "row",
            Axis::Column => "column",
        }
    }

    /// The table's key that lists their sizes: `rows` or `columns`.
    pub(crate) fn sizes_key(self) -> &'static str {
        match self {
            Axis::Row => "rows",
            Axis::Column => "columns",
        }
    }

    /// The operation's key that gives a new one's size: `height` or
    /// `width`.
    pub(crate) fn size_key(self) -> &'static str {
        match self {
            Axis::Row => "height",
            Axis::Column => "width",
        }
    }
}

/// An edit of a table's grid, as an operation sent it.
#[derive(Clone, Debug)]
pub(crate) enum TableEdit {
    /// Sets the keys of the cell `[row, column]`: each key under `set`
    /// takes its new value, written as a sheet description writes it, and
    /// `null` takes it away.
    SetCell {
        cell: [usize; 2],
        set: Map<String, Value>,
    },
    /// Adds a row or a column at `at`, after the last without it, `size`
    /// high or wide, with `cells`, one for each column or row.
    Add {
        axis: Axis,
        at: Option<usize>,
        size: Option<Value>,
        cells: Option<Vec<Value>>,
    },
    /// Deletes the row or column `index`.
    Delete { axis: Axis, index: usize },
}

/// What an edit of a table's grid did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TableEdited {
    /// Whether the table's keys were edited.
    pub edited: bool,
    /// The row's value a cell's text comes from and the text to set it to,
    /// when the edit set the text of a cell whose template takes it from
    /// the row.
    pub from_row: Option<(String, String)>,
    /// Where within the table the edit set something, such as
    /// `cells[2][1].merge` or `rows[3]`, so an error there is laid on it.
    pub touched: Vec<String>,
    /// Each cell the operation gave, where it is now, such as
    /// `cells[3][1]`, with its key in the operation, such as `cells[1]`.
    pub given: Vec<(String, String)>,
    /// What it did, in words, such as `adds a row at 3`.
    pub words: String,
    /// What else it did to keep the table in line, in words, such as `the
    /// merge of cell [0, 0] grows to [1, 5]`.
    pub notes: Vec<String>,
    /// The row or column it inserted or deleted, so places within the table
    /// an earlier edit touched follow their cells.
    pub shift: Option<Shift>,
}

/// A row or column inserted or deleted at `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shift {
    pub axis: Axis,
    pub at: usize,
    pub inserted: bool,
}

impl Shift {
    /// Where `path`, a place within the table such as `cells[2][1].style`
    /// or `rows[3]`, is once the row or column is inserted or deleted, or
    /// `None` when it was in the row or column deleted.
    pub(crate) fn path(&self, path: &str) -> Option<String> {
        /// The number in `[N]` at the start of `text`, and what follows.
        fn index(text: &str) -> Option<(usize, &str)> {
            let rest = text.strip_prefix('[')?;
            let (n, rest) = rest.split_once(']')?;
            Some((n.parse().ok()?, rest))
        }
        let moved = |n: usize| -> Option<usize> {
            match (self.inserted, n.cmp(&self.at)) {
                (true, std::cmp::Ordering::Less) | (false, std::cmp::Ordering::Less) => Some(n),
                (true, _) => Some(n + 1),
                (false, std::cmp::Ordering::Equal) => None,
                (false, std::cmp::Ordering::Greater) => Some(n - 1),
            }
        };
        let sizes = self.axis.sizes_key();
        if let Some((n, rest)) = path.strip_prefix(sizes).and_then(index) {
            return Some(format!("{sizes}[{}]{rest}", moved(n)?));
        }
        let Some((r, rest)) = path.strip_prefix("cells").and_then(index) else {
            return Some(path.to_owned());
        };
        match (self.axis, index(rest)) {
            (Axis::Row, _) => Some(format!("cells[{}]{rest}", moved(r)?)),
            (Axis::Column, Some((c, rest))) => Some(format!("cells[{r}][{}]{rest}", moved(c)?)),
            (Axis::Column, None) => Some(path.to_owned()),
        }
    }
}

/// A merged cell: its place and its span, `[rows, columns]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Merge {
    row: usize,
    column: usize,
    span: [usize; 2],
}

impl Merge {
    fn covers(&self, row: usize, column: usize) -> bool {
        (self.row..self.row + self.span[0]).contains(&row)
            && (self.column..self.column + self.span[1]).contains(&column)
    }

    fn words(&self) -> String {
        format!(
            "cell [{}, {}], which merges [{}, {}]",
            self.row, self.column, self.span[0], self.span[1]
        )
    }
}

/// An error in the operation, at `field` within it.
fn wrong(code: ErrorCode, message: impl Into<String>, field: &str) -> Error {
    Error::new(code, message).at_field(field)
}

/// The table's record cannot be edited as a grid: its `cells` do not match
/// its `rows` and `columns`, or one is not a list.
fn not_a_grid(why: &str) -> Error {
    wrong(
        ErrorCode::BadValue,
        format!("the table's record is not a grid this operation can edit: {why}; build the sheet to see what is wrong with it"),
        "target",
    )
}

fn list<'a>(keys: &'a [Entry], key: &str) -> Result<&'a [Node], Error> {
    match keys.iter().find(|e| e.key == key).map(|e| &e.value.kind) {
        Some(Kind::Sequence(items)) => Ok(items),
        _ => Err(not_a_grid(&format!("`{key}` is not a list"))),
    }
}

fn list_mut<'a>(keys: &'a mut [Entry], key: &str) -> Result<&'a mut Vec<Node>, Error> {
    match keys
        .iter_mut()
        .find(|e| e.key == key)
        .map(|e| &mut e.value.kind)
    {
        Some(Kind::Sequence(items)) => Ok(items),
        _ => Err(not_a_grid(&format!("`{key}` is not a list"))),
    }
}

/// The table's rows of cells, each row a list.
fn grid_mut(keys: &mut [Entry]) -> Result<Vec<&mut Vec<Node>>, Error> {
    list_mut(keys, "cells")?
        .iter_mut()
        .map(|row| match &mut row.kind {
            Kind::Sequence(cells) => Ok(cells),
            _ => Err(not_a_grid("a row of `cells` is not a list")),
        })
        .collect()
}

/// Whether the table's flag `key`, `title` or `header`, is set.
fn flag(keys: &[Entry], key: &str) -> bool {
    keys.iter()
        .find(|e| e.key == key)
        .is_some_and(|e| matches!(&e.value.kind, Kind::Scalar(text) if text == "true"))
}

/// The table's size, `[rows, columns]`, checking that its cells match it.
fn size(keys: &[Entry]) -> Result<[usize; 2], Error> {
    let rows = list(keys, "rows")?.len();
    let columns = list(keys, "columns")?.len();
    let cells = list(keys, "cells")?;
    if cells.len() != rows {
        return Err(not_a_grid(
            "`cells` does not list one row for each of `rows`",
        ));
    }
    for row in cells {
        match &row.kind {
            Kind::Sequence(items) if items.len() == columns => {}
            _ => {
                return Err(not_a_grid(
                    "a row of `cells` does not list one cell for each of `columns`",
                ))
            }
        }
    }
    Ok([rows, columns])
}

/// A whole number from 1, as a merge writes one.
fn count(node: &Node) -> Option<usize> {
    match &node.kind {
        Kind::Scalar(text) if text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().ok().filter(|n| *n >= 1)
        }
        _ => None,
    }
}

/// The span a cell's `merge` gives, when it reads as one.
fn cell_merge(cell: &Node) -> Option<[usize; 2]> {
    let Kind::Mapping(entries) = &cell.kind else {
        return None;
    };
    let merge = entries.iter().find(|e| e.key == "merge")?;
    match &merge.value.kind {
        Kind::Sequence(items) => match items.as_slice() {
            [down, across] => Some([count(down)?, count(across)?]),
            _ => None,
        },
        _ => None,
    }
}

/// Every merged cell of the table.
fn merges(keys: &[Entry]) -> Result<Vec<Merge>, Error> {
    let mut found = Vec::new();
    for (r, row) in list(keys, "cells")?.iter().enumerate() {
        let Kind::Sequence(cells) = &row.kind else {
            continue;
        };
        for (c, cell) in cells.iter().enumerate() {
            if let Some(span) = cell_merge(cell) {
                found.push(Merge {
                    row: r,
                    column: c,
                    span,
                });
            }
        }
    }
    Ok(found)
}

/// Whether a cell shows nothing and draws with nothing of its own, as a
/// cell a merge covers must: `''`, a bare null, or a mapping whose `text`
/// is empty and that gives nothing else.
fn is_empty(cell: &Node) -> bool {
    // The reader drops the line breaks at the end of a cell's text.
    let shows_nothing = |text: &str| text.trim_end_matches(['\n', '\r']).is_empty();
    match &cell.kind {
        Kind::Scalar(text) => shows_nothing(text) || cell.is_bare_null(),
        Kind::Mapping(entries) => entries.iter().all(|e| {
            e.value.is_bare_null()
                || (e.key == "text" && matches!(&e.value.kind, Kind::Scalar(t) if shows_nothing(t)))
        }),
        Kind::Sequence(_) => false,
    }
}

/// The span `value` sends for a merge, when it is one: two whole numbers
/// from 1, as numbers or as text.
fn sent_span(value: &Value) -> Option<[usize; 2]> {
    let whole = |v: &Value| -> Option<usize> {
        match v {
            Value::Number(n) => n.as_u64().and_then(|n| usize::try_from(n).ok()),
            Value::String(s) if s.bytes().all(|b| b.is_ascii_digit()) => s.parse().ok(),
            _ => None,
        }
        .filter(|n| *n >= 1)
    };
    match value {
        Value::Array(items) => match items.as_slice() {
            [down, across] => Some([whole(down)?, whole(across)?]),
            _ => None,
        },
        _ => None,
    }
}

/// Writes `span` as the merge of `cell`, or takes the merge away when the
/// span is the cell alone.
fn write_merge(cell: &mut Node, span: [usize; 2]) {
    let Kind::Mapping(entries) = &mut cell.kind else {
        return;
    };
    if span == [1, 1] {
        entries.retain(|e| e.key != "merge");
        if entries.is_empty() {
            *cell = Node::scalar("");
        }
        return;
    }
    let value = Node::new(Kind::Sequence(vec![
        Node::scalar(span[0].to_string()),
        Node::scalar(span[1].to_string()),
    ]));
    match entries.iter_mut().find(|e| e.key == "merge") {
        Some(entry) => entry.value = value,
        None => entries.push(new_entry("merge", value)),
    }
}

/// `places` listed in words, such as `[0, 1], [0, 2]`.
fn cells_words(places: &[(usize, usize)]) -> String {
    places
        .iter()
        .map(|(r, c)| format!("[{r}, {c}]"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Applies `edit` to the table whose keys are `keys`.
pub(super) fn edit_table(keys: &mut Vec<Entry>, edit: &TableEdit) -> Result<TableEdited, Error> {
    match edit {
        TableEdit::SetCell { cell, set } => set_cell(keys, *cell, set),
        TableEdit::Add {
            axis,
            at,
            size,
            cells,
        } => add(keys, *axis, *at, size.as_ref(), cells.as_deref()),
        TableEdit::Delete { axis, index } => delete(keys, *axis, *index),
    }
}

fn set_cell(
    keys: &mut [Entry],
    [r, c]: [usize; 2],
    set: &Map<String, Value>,
) -> Result<TableEdited, Error> {
    let [rows, columns] = size(keys)?;
    if r >= rows || c >= columns {
        return Err(wrong(
            ErrorCode::BadValue,
            format!(
                "the table has {rows} rows and {columns} columns, counted from 0 from the top \
                 left, so there is no cell [{r}, {c}]"
            ),
            "cell",
        ));
    }
    let merges = merges(keys)?;
    if let Some(over) = merges
        .iter()
        .find(|m| (m.row, m.column) != (r, c) && m.covers(r, c))
    {
        return Err(wrong(
            ErrorCode::BadValue,
            format!(
                "the cell [{r}, {c}] is inside the merge of {}, and shows nothing of its own; \
                 set that cell, or take its merge away first with set: {{\"merge\": null}} on it",
                over.words()
            ),
            "cell",
        ));
    }
    if let Some(span) = set.get("merge").and_then(sent_span) {
        let [down, across] = span;
        if down > rows - r || across > columns - c {
            return Err(wrong(
                ErrorCode::BadValue,
                format!(
                    "the merge spans {down} rows and {across} columns from the cell [{r}, {c}], \
                     past the table's {rows} rows and {columns} columns (counted from 0)"
                ),
                "set.merge",
            ));
        }
        let mine = Merge {
            row: r,
            column: c,
            span,
        };
        if let Some(other) = merges.iter().find(|m| {
            (m.row, m.column) != (r, c)
                && (0..m.span[0])
                    .any(|dr| (0..m.span[1]).any(|dc| mine.covers(m.row + dr, m.column + dc)))
        }) {
            return Err(wrong(
                ErrorCode::BadValue,
                format!(
                    "the merge from the cell [{r}, {c}] would run over the merge of {}; take \
                     that merge away first, or merge fewer cells",
                    other.words()
                ),
                "set.merge",
            ));
        }
        let grid = list(keys, "cells")?;
        let mut full = Vec::new();
        for (rr, row) in grid.iter().enumerate().skip(r).take(down) {
            let Kind::Sequence(cells) = &row.kind else {
                continue;
            };
            for (cc, cell) in cells.iter().enumerate().skip(c).take(across) {
                if (rr, cc) != (r, c) && !is_empty(cell) {
                    full.push((rr, cc));
                }
            }
        }
        if !full.is_empty() {
            return Err(wrong(
                ErrorCode::BadValue,
                format!(
                    "the merge from the cell [{r}, {c}] would cover cells that are not empty: \
                     {}; a cell a merge covers is empty, so empty them first with set_cell, \
                     set: {{\"text\": null}}, or merge fewer",
                    cells_words(&full)
                ),
                "set.merge",
            ));
        }
    }

    let mut grid = grid_mut(keys)?;
    let cell = &mut grid[r][c];
    // The row's value the template puts in the cell, as the cell itself or
    // as its `text`.
    let row_name = row_value_name(cell).or_else(|| match &cell.kind {
        Kind::Mapping(entries) => entries
            .iter()
            .find(|e| e.key == "text")
            .and_then(|e| row_value_name(&e.value)),
        _ => None,
    });
    let mut from_row = None;
    let mut edits: Vec<(&String, &Value)> = Vec::new();
    for (key, value) in set {
        if key == "text" {
            // Text sets the row's value; another value from the row, or
            // null, edits the template's cell itself.
            let text = match value {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            };
            if let (Some(name), Some(text)) = (&row_name, text) {
                from_row = Some((name.clone(), text));
                continue;
            }
        }
        edits.push((key, value));
    }
    // A key taken away that the cell does not give changes nothing.
    let gives = |key: &str| match &cell.kind {
        Kind::Mapping(entries) => {
            if row_value_name(cell).is_some() {
                key == "text"
            } else {
                entries.iter().any(|e| e.key == key)
            }
        }
        Kind::Scalar(_) => key == "text" && !is_empty(cell),
        Kind::Sequence(_) => false,
    };
    edits.retain(|(key, value)| !value.is_null() || gives(key));
    let mut touched = Vec::new();
    let place = index_path(&index_path("cells", r), c);
    if !edits.is_empty() {
        // The cell as a mapping: its text alone, or a value from the row,
        // becomes its `text`. One written as its text alone stays so when
        // the edit leaves it nothing but its text.
        let was_text = !matches!(&cell.kind, Kind::Mapping(_));
        if !matches!(&cell.kind, Kind::Mapping(_)) || row_value_name(cell).is_some() {
            let text = std::mem::replace(cell, Node::new(Kind::Mapping(Vec::new())));
            if !is_empty(&text) {
                if let Kind::Mapping(entries) = &mut cell.kind {
                    entries.push(new_entry("text", text));
                }
            }
        }
        let Kind::Mapping(entries) = &mut cell.kind else {
            unreachable!("made a mapping above")
        };
        for (key, value) in &edits {
            let position = entries.iter().position(|e| e.key == **key);
            match (value, position) {
                (Value::Null, Some(p)) => {
                    entries.remove(p);
                }
                (Value::Null, None) => {}
                (value, Some(p)) => entries[p].value = node_of(value),
                (value, None) => {
                    // A key the cell did not give goes in the order a cell's
                    // keys are listed.
                    let rank = |k: &str| CELL_KEYS.iter().position(|c| *c == k);
                    let at = entries
                        .iter()
                        .position(|e| rank(&e.key) > rank(key))
                        .unwrap_or(entries.len());
                    entries.insert(at, new_entry(key, node_of(value)));
                }
            }
            touched.push(key_path(&place, key));
        }
        match entries.as_slice() {
            [] => *cell = Node::scalar(""),
            [text]
                if was_text && text.key == "text" && matches!(text.value.kind, Kind::Scalar(_)) =>
            {
                *cell = text.value.clone();
            }
            _ => {}
        }
    }
    let set_keys: Vec<&str> = set.keys().map(String::as_str).collect();
    Ok(TableEdited {
        edited: !touched.is_empty(),
        from_row,
        touched,
        given: Vec::new(),
        words: format!("sets {} of cell [{r}, {c}]", set_keys.join(", ")),
        notes: Vec::new(),
        shift: None,
    })
}

fn add(
    keys: &mut [Entry],
    axis: Axis,
    at: Option<usize>,
    size_sent: Option<&Value>,
    cells: Option<&[Value]>,
) -> Result<TableEdited, Error> {
    let [rows, columns] = size(keys)?;
    let (count, across) = match axis {
        Axis::Row => (rows, columns),
        Axis::Column => (columns, rows),
    };
    let word = axis.word();
    let i = at.unwrap_or(count);
    if i > count {
        return Err(wrong(
            ErrorCode::BadValue,
            format!(
                "the table has {count} {word}s, so a {word} is added at 0 to {count}, {count} \
                 putting it after the last; there is no place {i}"
            ),
            "at",
        ));
    }
    if axis == Axis::Row {
        let title = flag(keys, "title");
        let header = flag(keys, "header");
        let head = usize::from(title) + usize::from(header);
        if i < head {
            let what = match (title, header) {
                (true, true) => "its title row and its header row, rows 0 and 1".to_owned(),
                (true, false) => "its title row, row 0".to_owned(),
                _ => "its header row, row 0".to_owned(),
            };
            return Err(wrong(
                ErrorCode::BadValue,
                format!(
                    "the table's first rows are {what}, so a row is added at {head} or after, \
                     not at {i}"
                ),
                "at",
            ));
        }
    }
    let new_cells: Vec<Node> = match cells {
        Some(sent) if sent.len() != across => {
            let other = match axis {
                Axis::Row => "column",
                Axis::Column => "row",
            };
            return Err(wrong(
                ErrorCode::BadValue,
                format!(
                    "`cells` lists {} cells and the table has {across} {other}s; list one cell \
                     for each {other}, `''` for an empty one",
                    sent.len()
                ),
                "cells",
            ));
        }
        Some(sent) => sent.iter().map(node_of).collect(),
        None => (0..across).map(|_| Node::scalar("")).collect(),
    };
    let sizes_key = axis.sizes_key();
    let size_node = match size_sent {
        Some(value) => node_of(value),
        None => {
            // The size of the one it follows, or the first's at 0, written
            // as the record writes it.
            let sizes = list(keys, sizes_key)?;
            let Some(like) = sizes.get(i.saturating_sub(1)) else {
                let size = axis.size_key();
                return Err(wrong(
                    ErrorCode::BadValue,
                    format!("the table has no {word} to take a {size} from; give {size}"),
                    size,
                ));
            };
            match &like.kind {
                Kind::Scalar(text) => Node::scalar(text.clone()),
                _ => like.clone(),
            }
        }
    };

    // A merge the new one falls inside grows by one.
    let mut notes = Vec::new();
    let mut touched = vec![index_path(sizes_key, i)];
    for m in merges(keys)? {
        let (first, along) = match axis {
            Axis::Row => (m.row, 0),
            Axis::Column => (m.column, 1),
        };
        if first < i && i < first + m.span[along] {
            let mut span = m.span;
            span[along] += 1;
            let mut grid = grid_mut(keys)?;
            write_merge(&mut grid[m.row][m.column], span);
            notes.push(format!(
                "the merge of cell [{}, {}] grows to [{}, {}]",
                m.row, m.column, span[0], span[1]
            ));
            touched.push(key_path(
                &index_path(&index_path("cells", m.row), m.column),
                "merge",
            ));
        }
    }
    list_mut(keys, sizes_key)?.insert(i, size_node);
    // Each cell given, where it goes, with its key in the operation.
    let place = |n: usize| match axis {
        Axis::Row => index_path(&index_path("cells", i), n),
        Axis::Column => index_path(&index_path("cells", n), i),
    };
    let given: Vec<(String, String)> = match cells {
        Some(sent) => (0..sent.len())
            .map(|n| (place(n), index_path("cells", n)))
            .collect(),
        None => Vec::new(),
    };
    match axis {
        Axis::Row => {
            list_mut(keys, "cells")?.insert(i, Node::new(Kind::Sequence(new_cells)));
            touched.push(index_path("cells", i));
        }
        Axis::Column => {
            let mut grid = grid_mut(keys)?;
            for (r, (row, cell)) in grid.iter_mut().zip(new_cells).enumerate() {
                row.insert(i, cell);
                touched.push(index_path(&index_path("cells", r), i));
            }
        }
    }
    Ok(TableEdited {
        edited: true,
        from_row: None,
        touched,
        words: format!("adds a {word} at {i}"),
        notes,
        given,
        shift: Some(Shift {
            axis,
            at: i,
            inserted: true,
        }),
    })
}

fn delete(keys: &mut Vec<Entry>, axis: Axis, i: usize) -> Result<TableEdited, Error> {
    let [rows, columns] = size(keys)?;
    let count = match axis {
        Axis::Row => rows,
        Axis::Column => columns,
    };
    let word = axis.word();
    if i >= count {
        return Err(wrong(
            ErrorCode::BadValue,
            format!("the table has {count} {word}s, counted from 0, so there is no {word} {i}"),
            word,
        ));
    }
    if count == 1 {
        return Err(wrong(
            ErrorCode::BadValue,
            format!(
                "the table has one {word}, and a table keeps at least one; delete the table \
                 instead"
            ),
            word,
        ));
    }
    let along = match axis {
        Axis::Row => 0,
        Axis::Column => 1,
    };
    let all = merges(keys)?;
    let first_of = |m: &Merge| match axis {
        Axis::Row => m.row,
        Axis::Column => m.column,
    };
    if let Some(m) = all.iter().find(|m| first_of(m) == i && m.span[along] > 1) {
        return Err(wrong(
            ErrorCode::BadValue,
            format!(
                "{word} {i} holds {}, so it spans {word}s {i} to {}; take its merge away first \
                 with set_cell, cell: [{}, {}], set: {{\"merge\": null}}",
                m.words(),
                i + m.span[along] - 1,
                m.row,
                m.column
            ),
            word,
        ));
    }
    let mut notes = Vec::new();
    let mut touched = Vec::new();
    for m in &all {
        let first = first_of(m);
        if first < i && i < first + m.span[along] {
            let mut span = m.span;
            span[along] -= 1;
            let mut grid = grid_mut(keys)?;
            write_merge(&mut grid[m.row][m.column], span);
            notes.push(if span == [1, 1] {
                format!(
                    "the merge of cell [{}, {}] spans that cell alone now, so it is taken away",
                    m.row, m.column
                )
            } else {
                format!(
                    "the merge of cell [{}, {}] shrinks to [{}, {}]",
                    m.row, m.column, span[0], span[1]
                )
            });
            touched.push(key_path(
                &index_path(&index_path("cells", m.row), m.column),
                "merge",
            ));
        }
    }
    if axis == Axis::Row {
        let title = flag(keys, "title");
        let header = flag(keys, "header");
        let mut cleared = Vec::new();
        if title && i == 0 {
            cleared.push("title");
        }
        if header && i == usize::from(title) {
            cleared.push("header");
        }
        for flag_key in cleared {
            keys.retain(|e| e.key != flag_key);
            notes.push(format!("the table has no {flag_key} row now"));
            touched.push(flag_key.to_owned());
        }
    }
    list_mut(keys, axis.sizes_key())?.remove(i);
    match axis {
        Axis::Row => {
            list_mut(keys, "cells")?.remove(i);
        }
        Axis::Column => {
            for row in grid_mut(keys)? {
                row.remove(i);
            }
        }
    }
    touched.push(axis.sizes_key().to_owned());
    Ok(TableEdited {
        edited: true,
        from_row: None,
        touched,
        words: format!("deletes {word} {i}"),
        notes,
        given: Vec::new(),
        shift: Some(Shift {
            axis,
            at: i,
            inserted: false,
        }),
    })
}
