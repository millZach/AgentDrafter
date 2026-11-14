//! The four default checks on one sheet.
//!
//! The sheet is drawn once through the one drawing pipeline, as `query
//! extents` draws it ([`measuring_drawing`]): every layer continuous, curves
//! within a millionth of the sheet's larger side, and text as its font
//! draws it. Every check measures that drawing, so what it measures is
//! what the PNG, the PDF and the viewer draw.
//!
//! - **`paper_size`**: the sheet's extents, as `query extents` gives them,
//!   are its paper's width and height, each within the tolerance. A sheet
//!   that declares no paper fails it once, saying so, and is not checked
//!   for objects off it.
//! - **`off_sheet`**: what each of the sheet's objects draws keeps within
//!   the paper, from `[0, 0]` to its width and height, within the
//!   tolerance. An insert is one object, what its block draws and its
//!   attributes together. A patterned hatch's boundary counts, as it does
//!   towards the extents, though its lines may stop short of it.
//! - **`text_overflow`**: each text and attribute an insert draws whose
//!   block declares its cell draws within the cell, placed by the insert's
//!   position, scale and rotation: what it draws is taken back into the
//!   block's own coordinates and held to the cell there, and how far it
//!   reaches past, enlarged by the insert's scale, is held to the
//!   tolerance on the sheet. Text with no cell is not checked. And each
//!   cell of a table holds its text inside its margins: across, what the
//!   text draws; up, the height AutoCAD sets its lines in: the text's
//!   height and 5/3 of it more for each line after the first, and at least
//!   4/3 of it, as AutoCAD 2026 grew made-up tables' rows of one to four
//!   lines to fit.
//! - **`text_on_text`**: the box round what each line of text draws, seen
//!   in the line's own direction, as the pipeline draws the line, overlaps
//!   no other text's by more than the tolerance: how far one would move to
//!   clear the other, the least along the sides of either box. A box with
//!   no breadth, round a single upright stroke such as `I`, counts too.
//!   Every text on the sheet counts, each line of a multiline text on its
//!   own, and each text and visible attribute an insert draws, against
//!   those of the same insert too.
//! - **`text_on_line`**: no line touches a text's letters as they print,
//!   by more than the tolerance (Zach, 2026-11-13). The sheet is drawn a
//!   second time as it prints ([`printing_drawing`]): linetypes laid out,
//!   so a line is its dashes, and layers that do not plot left out. A
//!   line is a line, polyline, arc, circle, ellipse or spline, a wide
//!   polyline's band among them; its ink is its pen's width round its
//!   path. A text's letters are its font's strokes, with their pen, or its
//!   outlines, filled. A line through the gaps between letters does not
//!   touch them. The lines of the text's own insert, such as a title
//!   block's cell lines, never count, nor do hatches, 2D solids and
//!   donuts, which are fills.

use std::collections::BTreeMap;

use super::ink::Ink;
use super::{BlockPart, CheckedObject, Finding, Rule, Sides};
use crate::description::SheetRecord;
use crate::draw::{mtext_lines, Bounds, DisplayList, Item, Point2, TextRun};
use crate::model::block::Placement;
use crate::model::{Block, Insert, Object, Paper, Table, TableStyle, LINE_BOX, LINE_STEP};
use crate::query::{measuring_drawing, printing_drawing, written_at, Extents};
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// Checks one sheet against every rule; `tolerance` is in the declared unit
/// `units`.
pub(super) fn check_sheet(
    record: &SheetRecord,
    fonts: &SheetFonts,
    units: Unit,
    tolerance: f64,
) -> Vec<Finding> {
    let list = measuring_drawing(&record.sheet, fonts);
    let contents: Vec<Option<Insert>> = record
        .sheet
        .objects
        .iter()
        .map(|o| match o {
            Object::MLeader(m) => record.sheet.block_content(m),
            _ => None,
        })
        .collect();
    let on = Sheet {
        record,
        list: &list,
        fonts,
        units,
        tolerance,
        contents: &contents,
    };
    let mut findings = Vec::new();
    match &record.sheet.paper {
        None => findings.push(on.no_paper()),
        Some(paper) => {
            findings.extend(on.paper_size(paper));
            findings.extend(on.off_sheet(paper));
        }
    }
    findings.extend(on.text_overflow());
    findings.extend(on.text_on_text());
    let printed = printing_drawing(&record.sheet, fonts);
    findings.extend(on.text_on_line(&printed));
    findings
}

/// One sheet being checked, drawn.
struct Sheet<'a> {
    record: &'a SheetRecord,
    list: &'a DisplayList,
    fonts: &'a SheetFonts,
    units: Unit,
    tolerance: f64,
    /// The insert that draws each multileader's block content, by the
    /// sheet's object it is.
    contents: &'a [Option<Insert>],
}

impl Sheet<'_> {
    fn finding(&self, rule: Rule, message: String) -> Finding {
        Finding {
            sheet: self.record.name.clone(),
            rule,
            message,
            object: None,
            other: None,
            drawn: None,
            other_drawn: None,
            paper: None,
            cell: None,
            off_by: None,
            past: None,
            overlap: None,
            at: None,
        }
    }

    /// The file that declares the sheet's paper, for a message.
    fn declared_in(&self) -> &'static str {
        if self.record.rows_file.is_some() {
            "its template"
        } else {
            "its sheet description"
        }
    }

    /// The one finding of a sheet that declares no paper.
    fn no_paper(&self) -> Finding {
        let extents = self.list.extents();
        let u = self.units;
        let draws = match &extents {
            Some(e) => format!(
                "it draws {} by {} {u}, from {} to {}. Declare its paper under `paper` in {}, \
                 such as `paper: [{}, {}]`",
                num(e.width()),
                num(e.height()),
                point(e.min),
                point(e.max),
                self.declared_in(),
                num(e.width()),
                num(e.height()),
            ),
            None => format!(
                "it draws nothing. Declare its paper under `paper` in {}",
                self.declared_in()
            ),
        };
        let mut finding = self.finding(
            Rule::PaperSize,
            format!(
                "sheet `{}` declares no paper, so neither its size nor what is off it is \
                 checked; {draws}",
                self.record.name
            ),
        );
        finding.drawn = extents.as_ref().map(Extents::of);
        finding
    }

    /// The finding of a sheet whose extents are not its paper's size.
    fn paper_size(&self, paper: &Paper) -> Option<Finding> {
        let u = self.units;
        let named = paper_words(paper);
        let Some(e) = self.list.extents() else {
            let mut finding = self.finding(
                Rule::PaperSize,
                format!(
                    "sheet `{}` draws nothing, so it is not its paper{named}, {} by {} {u}",
                    self.record.name,
                    num(paper.width),
                    num(paper.height)
                ),
            );
            finding.paper = Some(paper.clone());
            return Some(finding);
        };
        let off = [e.width() - paper.width, e.height() - paper.height];
        if off.iter().all(|d| d.abs() <= self.tolerance) {
            return None;
        }
        let mut wrong = Vec::new();
        for (d, more, less) in [
            (off[0], "too wide", "too narrow"),
            (off[1], "too tall", "too short"),
        ] {
            if d.abs() > self.tolerance {
                let how = if d > 0.0 { more } else { less };
                wrong.push(format!("{} {u} {how}", num(d.abs())));
            }
        }
        let mut finding = self.finding(
            Rule::PaperSize,
            format!(
                "sheet `{}` draws {} by {} {u}, from {} to {}, and its paper{named} is {} by {} \
                 {u}: it is {}",
                self.record.name,
                num(e.width()),
                num(e.height()),
                point(e.min),
                point(e.max),
                num(paper.width),
                num(paper.height),
                wrong.join(" and ")
            ),
        );
        finding.drawn = Some(Extents::of(&e));
        finding.paper = Some(paper.clone());
        finding.off_by = Some(off.map(zero_signless));
        Some(finding)
    }

    /// A finding for each object that draws past the paper.
    fn off_sheet(&self, paper: &Paper) -> Vec<Finding> {
        let u = self.units;
        let mut boxes: Vec<Option<Bounds>> = vec![None; self.record.sheet.objects.len()];
        let drawn = self
            .list
            .items
            .iter()
            .filter_map(|item| Some((item.object, Bounds::of(item.points())?)));
        // A patterned hatch's boundary counts, as it does towards the
        // extents, though the hatch's lines may stop short of it.
        let boundaries = self.list.boundaries.iter().copied();
        for (object, b) in drawn.chain(boundaries) {
            let had = boxes[object].into_iter().flat_map(|had| [had.min, had.max]);
            boxes[object] = Bounds::of(had.chain([b.min, b.max]));
        }
        let mut findings = Vec::new();
        for (i, drawn) in boxes.iter().enumerate() {
            let Some(b) = drawn else {
                continue;
            };
            let past = Sides {
                left: beyond(0.0 - b.min[0]),
                bottom: beyond(0.0 - b.min[1]),
                right: beyond(b.max[0] - paper.width),
                top: beyond(b.max[1] - paper.height),
            };
            if past.most() <= self.tolerance {
                continue;
            }
            let mut finding = self.finding(
                Rule::OffSheet,
                format!(
                    "{} draws {} the paper: it draws from {} to {}, and the paper{} runs from \
                     [0, 0] to [{}, {}] {u}",
                    self.object_words(i),
                    past.words(self.tolerance, 1.0, u, "edge"),
                    point(b.min),
                    point(b.max),
                    paper_words(paper),
                    num(paper.width),
                    num(paper.height)
                ),
            );
            finding.object = Some(self.checked(i, &[]));
            finding.drawn = Some(Extents::of(b));
            finding.paper = Some(paper.clone());
            finding.past = Some(past);
            findings.push(finding);
        }
        findings
    }

    /// A finding for each text or attribute an insert draws past the cell
    /// its block declares for it, and each table cell's text that needs
    /// more than its cell.
    fn text_overflow(&self) -> Vec<Finding> {
        let u = self.units;
        let mut findings = Vec::new();
        for (i, object) in self.record.sheet.objects.iter().enumerate() {
            if let Object::Table(table) = object {
                findings.extend(self.table_overflow(i, table));
                continue;
            }
            let Object::Insert(insert) = object else {
                continue;
            };
            let Some(block) = self.block(insert) else {
                continue;
            };
            let placement = Placement::of(insert, block.base_point);
            for cell in &block.cells {
                // What the text draws, taken back into the block's own
                // coordinates.
                let points = self
                    .list
                    .items
                    .iter()
                    .filter(|item| {
                        item.object == i && item.part == Some(cell.object) && item.nested.is_empty()
                    })
                    .flat_map(Item::points)
                    .map(|p| placement.back(p[0], p[1]));
                let Some(b) = Bounds::of(points) else {
                    continue;
                };
                let past = Sides {
                    left: beyond(cell.min[0] - b.min[0]),
                    bottom: beyond(cell.min[1] - b.min[1]),
                    right: beyond(b.max[0] - cell.max[0]),
                    top: beyond(b.max[1] - cell.max[1]),
                };
                if past.most() * insert.scale <= self.tolerance {
                    continue;
                }
                let scaled = if insert.scale == 1.0 {
                    String::new()
                } else {
                    format!(
                        ", which the insert's scale of {} enlarges on the sheet",
                        num(insert.scale)
                    )
                };
                let mut finding = self.finding(
                    Rule::TextOverflow,
                    format!(
                        "{} draws {} its cell: in the block {}'s own coordinates it draws from {} \
                         to {}, and its cell runs from {} to {} {u}{scaled}",
                        self.text_words(i, &[cell.object], ""),
                        past.words(self.tolerance, insert.scale, u, "side"),
                        block.name,
                        point(b.min),
                        point(b.max),
                        point(cell.min),
                        point(cell.max),
                    ),
                );
                finding.object = Some(self.checked(i, &[cell.object]));
                finding.drawn = Some(Extents::of(&b));
                finding.cell = Some(Extents::of(&Bounds {
                    min: cell.min,
                    max: cell.max,
                }));
                finding.past = Some(past);
                findings.push(finding);
            }
        }
        findings
    }

    /// A finding for each cell of the table at `i` whose text needs more
    /// than the cell inside its margins: across, what the text draws; up,
    /// the height AutoCAD sets its lines in, placed as the text is
    /// justified.
    fn table_overflow(&self, i: usize, table: &Table) -> Vec<Finding> {
        let u = self.units;
        let sheet = &self.record.sheet;
        let style = sheet
            .table_style(&table.table_style)
            .unwrap_or_else(|| TableStyle::standard(sheet.units));
        let mut findings = Vec::new();
        for cell in table.laid_out(&style) {
            let Some(text) = &cell.text else {
                continue;
            };
            let mine = |c: Option<[usize; 2]>| c == Some(cell.at);
            let drawn = Bounds::of(
                self.list
                    .items
                    .iter()
                    .filter(|item| item.object == i && mine(item.cell))
                    .flat_map(Item::points),
            );
            let Some(drawn) = drawn else {
                continue;
            };
            // Every line the text is laid out in, a blank one among them,
            // which draws no run of its own.
            let lines = mtext_lines(text, self.fonts).max(1);
            let [h, v] = cell.margins;
            let inside = Bounds {
                min: [cell.x[0] + h, cell.y[0] + v],
                max: [cell.x[1] - h, cell.y[1] - v],
            };
            // The height AutoCAD sets the lines in, placed as the text is
            // justified: from the top margin, about the middle, or up from
            // the bottom margin.
            let needs = text.height * LINE_BOX.max(1.0 + LINE_STEP * (lines - 1) as f64);
            let at = crate::text::Attachment::ALL
                .iter()
                .position(|a| *a == text.justify)
                .unwrap_or(0);
            let (low, high) = match at / 3 {
                0 => (inside.max[1] - needs, inside.max[1]),
                1 => {
                    let middle = (cell.y[0] + cell.y[1]) / 2.0;
                    (middle - needs / 2.0, middle + needs / 2.0)
                }
                _ => (inside.min[1], inside.min[1] + needs),
            };
            let needed = Bounds {
                min: [drawn.min[0], low],
                max: [drawn.max[0], high],
            };
            let past = Sides {
                left: beyond(inside.min[0] - needed.min[0]),
                bottom: beyond(inside.min[1] - needed.min[1]),
                right: beyond(needed.max[0] - inside.max[0]),
                top: beyond(needed.max[1] - inside.max[1]),
            };
            if past.most() <= self.tolerance {
                continue;
            }
            let path = cell.at.to_vec();
            let mut finding = self.finding(
                Rule::TextOverflow,
                format!(
                    "{} needs {} its cell inside its margins: its text draws from x {} to {} and \
                     its {} set {} {u} high, and inside its margins the cell runs from {} to {} \
                     {u}; AutoCAD would make the row taller where the text needs more height",
                    self.text_words(i, &path, ""),
                    past.words(self.tolerance, 1.0, u, "side"),
                    num(needed.min[0]),
                    num(needed.max[0]),
                    if lines == 1 { "line is" } else { "lines are" },
                    num(needs),
                    point(inside.min),
                    point(inside.max),
                ),
            );
            finding.object = Some(self.checked(i, &path));
            finding.drawn = Some(Extents::of(&needed));
            finding.cell = Some(Extents::of(&inside));
            finding.past = Some(past);
            findings.push(finding);
        }
        findings
    }

    /// A finding for each two texts whose drawn boxes overlap by more than
    /// the tolerance, naming the one drawn later as sitting on the other.
    fn text_on_text(&self) -> Vec<Finding> {
        let u = self.units;
        // Each line of text as the pipeline draws it, in its own box.
        let boxes: Vec<TextBox> = self.list.text.iter().filter_map(TextBox::of).collect();
        // Sweep across: two boxes apart across by more than the tolerance
        // are apart.
        let mut order: Vec<usize> = (0..boxes.len()).collect();
        order.sort_by(|&a, &b| boxes[a].seen.min[0].total_cmp(&boxes[b].seen.min[0]));
        // Each two texts once, by the deepest overlap of their lines: the
        // one drawn later, then the one it sits on.
        let mut pairs: BTreeMap<(Key, Key), (usize, usize, f64)> = BTreeMap::new();
        for (k, &a) in order.iter().enumerate() {
            for &b in &order[k + 1..] {
                if boxes[b].seen.min[0] > boxes[a].seen.max[0] + self.tolerance {
                    break;
                }
                if boxes[a].key == boxes[b].key {
                    continue;
                }
                let Some(depth) = boxes[a].sits_on(&boxes[b], self.tolerance) else {
                    continue;
                };
                let (earlier, later) = if boxes[a].key < boxes[b].key {
                    (a, b)
                } else {
                    (b, a)
                };
                let deepest = pairs
                    .entry((boxes[later].key.clone(), boxes[earlier].key.clone()))
                    .or_insert((later, earlier, depth));
                if depth > deepest.2 {
                    *deepest = (later, earlier, depth);
                }
            }
        }
        pairs
            .into_values()
            .map(|(later, earlier, depth)| {
                let (later, earlier) = (&boxes[later], &boxes[earlier]);
                let mut finding = self.finding(
                    Rule::TextOnText,
                    format!(
                        "{} sits on {}: their drawn boxes overlap by {} {u}; it draws from {} to \
                         {}, and the other from {} to {}",
                        self.text_words(later.key.0, &later.key.1, &later.says),
                        self.text_words(earlier.key.0, &earlier.key.1, &earlier.says),
                        num(depth),
                        point(later.seen.min),
                        point(later.seen.max),
                        point(earlier.seen.min),
                        point(earlier.seen.max),
                    ),
                );
                finding.object = Some(self.checked(later.key.0, &later.key.1));
                finding.other = Some(self.checked(earlier.key.0, &earlier.key.1));
                finding.drawn = Some(Extents::of(&later.seen));
                finding.other_drawn = Some(Extents::of(&earlier.seen));
                finding.overlap = Some(depth);
                finding
            })
            .collect()
    }

    /// A finding for each text whose letters a line touches as they print,
    /// once for each text and line, at the first point found where they
    /// touch.
    fn text_on_line(&self, printed: &DisplayList) -> Vec<Finding> {
        let u = self.units;
        let units_per_mm = Unit::Mm.convert(1.0, u);
        let mut letters: BTreeMap<Key, Ink> = BTreeMap::new();
        let mut lines: BTreeMap<Key, Ink> = BTreeMap::new();
        for item in &printed.items {
            let key = (item.object, item_path(item.part, &item.nested, item.cell));
            match self.drawn_as(&key) {
                Drawn::Letters => letters
                    .entry(key)
                    .or_default()
                    .add(&item.shape, units_per_mm),
                Drawn::Line => lines.entry(key).or_default().add(&item.shape, units_per_mm),
                Drawn::Fill => {}
            }
        }
        let mut findings = Vec::new();
        for (text, letter_ink) in &letters {
            for (line, line_ink) in &lines {
                // An insert's own lines, such as its cell lines, never
                // count against its own text.
                if line.0 == text.0 {
                    continue;
                }
                let Some(at) = line_ink.touches(letter_ink, self.tolerance) else {
                    continue;
                };
                let says = printed
                    .text
                    .iter()
                    .filter(|run| {
                        run.object == text.0 && item_path(run.part, &run.nested, run.cell) == text.1
                    })
                    .map(|run| run.characters.iter().map(|&(c, _)| c).collect::<String>())
                    .collect::<Vec<_>>()
                    .join(" ");
                let mut finding = self.finding(
                    Rule::TextOnLine,
                    format!(
                        "{} runs through {}: it touches the letters at {}, as they print",
                        self.line_words(line.0, &line.1),
                        self.text_words(text.0, &text.1, &says),
                        point(at),
                    ),
                );
                finding.object = Some(self.checked(text.0, &text.1));
                finding.other = Some(self.checked(line.0, &line.1));
                finding.drawn = letter_ink.bounds.as_ref().map(Extents::of);
                finding.other_drawn = line_ink.bounds.as_ref().map(Extents::of);
                finding.at = Some(at);
                findings.push(finding);
            }
        }
        findings
    }

    /// What the sheet's object `i` draws, or the object of its block at
    /// `path`: letters, a line, or a fill that no line rule counts.
    fn drawn_as(&self, (i, path): &Key) -> Drawn {
        let i = *i;
        let object = match self.in_block(i, path) {
            Some(inner) => inner.object,
            None => &self.record.sheet.objects[i],
        };
        match object {
            Object::Text(_) | Object::MText(_) | Object::AttributeDefinition(_) => Drawn::Letters,
            Object::Line(_)
            | Object::Polyline(_)
            | Object::Arc(_)
            | Object::Circle(_)
            | Object::Ellipse(_)
            | Object::Spline(_) => Drawn::Line,
            Object::Hatch(_) | Object::Solid(_) | Object::Donut(_) | Object::Insert(_) => {
                Drawn::Fill
            }
            // A table's borders are lines; its cells' text, letters. A
            // leader's lines, landing and arrowhead are lines; its text,
            // drawn as its part 0, letters.
            Object::Table(_) | Object::MLeader(_) | Object::Leader(_) if path.is_empty() => {
                Drawn::Line
            }
            Object::Table(_) | Object::MLeader(_) | Object::Leader(_) => Drawn::Letters,
        }
    }

    /// A line, as a finding names it: the sheet's object `i`, or the
    /// object of its block at `path` and the insert that draws it.
    fn line_words(&self, i: usize, path: &[usize]) -> String {
        let words = self.object_words(i);
        match self.in_block(i, path) {
            Some(inner) => format!(
                "the {} at `{}` of the block {} {}",
                inner.object.type_name(),
                inner.field(),
                inner.block.name,
                inner.inserted_by(&words)
            ),
            None => words,
        }
    }

    /// The block an insert places, as the sheet holds it.
    fn block(&self, insert: &Insert) -> Option<&Block> {
        self.record
            .sheet
            .blocks
            .iter()
            .find(|b| b.name == insert.block)
    }

    /// The object at `path` that the sheet's object `i` draws, when `i` is
    /// an insert: the object of its block at the path's first place, or,
    /// through each insert nested in that block in turn, of the innermost
    /// block.
    fn in_block(&self, i: usize, path: &[usize]) -> Option<InBlock<'_>> {
        // A multileader's block content draws as an insert of its block.
        let outermost = match &self.record.sheet.objects[i] {
            Object::Insert(insert) => insert,
            Object::MLeader(_) => self.contents.get(i)?.as_ref()?,
            _ => return None,
        };
        let (&index, through) = path.split_last()?;
        let mut insert = outermost;
        let mut block = self.block(insert)?;
        let top = &block.name;
        for &at in through {
            let Some(Object::Insert(inner)) = block.objects.get(at) else {
                return None;
            };
            insert = inner;
            block = self.block(insert)?;
        }
        Some(InBlock {
            insert,
            block,
            index,
            object: block.objects.get(index)?,
            nested_in: (!through.is_empty()).then_some(top.as_str()),
        })
    }

    /// The sheet's object `i`, as a finding names it: by its ID, its name
    /// and where it is written, and for `part` of an insert, which object
    /// of its block.
    fn checked(&self, i: usize, path: &[usize]) -> CheckedObject {
        let record = self.record;
        let object = &record.sheet.objects[i];
        if let (Object::Table(table), [row, column]) = (object, path) {
            return CheckedObject {
                id: record.id(i),
                name: record.objects[i].name.clone(),
                kind: object.type_name().to_owned(),
                written_at: written_at(record, i),
                in_block: None,
                table_cell: Some([*row, *column]),
                text: table
                    .cells
                    .get(*row)
                    .and_then(|r| r.get(*column))
                    .map(|c| c.text.clone()),
            };
        }
        let (in_block, text) = match self.in_block(i, path) {
            Some(inner) => {
                let tag = match inner.object {
                    Object::AttributeDefinition(d) => Some(d.tag.clone()),
                    _ => None,
                };
                let part = BlockPart {
                    block: inner.block.name.clone(),
                    file: self.library_file(),
                    field: inner.field(),
                    kind: inner.object.type_name().to_owned(),
                    tag,
                };
                (Some(part), inner.says())
            }
            None => (None, says(object)),
        };
        CheckedObject {
            id: record.id(i),
            name: record.objects[i].name.clone(),
            kind: object.type_name().to_owned(),
            written_at: written_at(record, i),
            in_block,
            table_cell: None,
            text,
        }
    }

    /// The block library's file, as the sheet names it.
    fn library_file(&self) -> String {
        let record = self.record;
        record
            .sheet
            .block_library
            .as_ref()
            .map(|written| {
                record
                    .file
                    .parent()
                    .unwrap_or(std::path::Path::new(""))
                    .join(written)
                    .display()
                    .to_string()
            })
            .unwrap_or_default()
    }

    /// The sheet's object `i` in words, such as "the circle
    /// `DEMO-002/tank`".
    fn object_words(&self, i: usize) -> String {
        let record = self.record;
        let named = match &record.objects[i].name {
            Some(name) => format!(" (named `{name}`)"),
            None => String::new(),
        };
        format!(
            "the {} `{}`{named}",
            record.sheet.objects[i].type_name(),
            record.id(i)
        )
    }

    /// The text the sheet's object `i` draws, at `path` of an insert's
    /// block, in words, such as "the attribute TITLE of the insert
    /// `DEMO-001/objects[2]`". `line` is the line of an mtext meant.
    fn text_words(&self, i: usize, path: &[usize], line: &str) -> String {
        let words = self.object_words(i);
        let on_line = format!(" on its line `{line}`");
        let Some(inner) = self.in_block(i, path) else {
            return match (&self.record.sheet.objects[i], path) {
                (Object::Text(t), _) => format!("{words} saying `{}`", t.contents),
                (Object::MText(_) | Object::MLeader(_) | Object::Leader(_), _) => {
                    format!("{words}{on_line}")
                }
                (Object::Table(table), &[row, column]) => {
                    let says = table
                        .cells
                        .get(row)
                        .and_then(|r| r.get(column))
                        .map(|c| c.text.replace('\n', " "))
                        .unwrap_or_default();
                    let line = if line.is_empty() || line == says {
                        String::new()
                    } else {
                        on_line
                    };
                    format!(
                        "the cell at row {row}, column {column} of {words} saying `{says}`{line}"
                    )
                }
                _ => words,
            };
        };
        match inner.object {
            Object::AttributeDefinition(d) if inner.nested_in.is_none() => format!(
                "the attribute {} of {words} saying `{}`",
                d.tag,
                inner.says().unwrap_or_default()
            ),
            Object::AttributeDefinition(d) => format!(
                "the attribute {} of the block {} saying `{}`, {}",
                d.tag,
                inner.block.name,
                inner.says().unwrap_or_default(),
                inner.inserted_by(&words)
            ),
            Object::Text(t) => format!(
                "the text `{}` of the block {} {}",
                t.contents,
                inner.block.name,
                inner.inserted_by(&words)
            ),
            Object::MText(_) => format!(
                "the mtext at `{}`{on_line}, {}",
                inner.field(),
                inner.inserted_by(&words)
            ),
            _ => words,
        }
    }
}

/// One object of an insert's block, as the insert draws it.
struct InBlock<'a> {
    /// The insert of `block`: the sheet's, or one nested in its block.
    insert: &'a Insert,
    block: &'a Block,
    /// Its place in the block's objects.
    index: usize,
    object: &'a Object,
    /// When `insert` is nested, the block of the sheet's insert, which
    /// holds it.
    nested_in: Option<&'a str>,
}

impl InBlock<'_> {
    /// Which insert puts it on the sheet, in words, given the sheet's
    /// insert's: "that the insert ... inserts", or, nested, "nested in the
    /// block STRIP that the insert ... inserts".
    fn inserted_by(&self, words: &str) -> String {
        match self.nested_in {
            None => format!("that {words} inserts"),
            Some(top) => format!("nested in the block {top} that {words} inserts"),
        }
    }

    /// Its place in the block library, such as
    /// `blocks.TITLE_BLOCK.objects[13]`.
    fn field(&self) -> String {
        format!("blocks.{}.objects[{}]", self.block.name, self.index)
    }

    /// What it says on the insert, when it is text: a text's or an mtext's
    /// contents, or the insert's value of an attribute.
    fn says(&self) -> Option<String> {
        match self.object {
            // Matched by place, since a block may define a tag more than once.
            Object::AttributeDefinition(d) => Some(
                self.block
                    .attribute_value(self.insert, self.index)
                    .unwrap_or(&d.default)
                    .to_owned(),
            ),
            object => says(object),
        }
    }
}

/// What a text, an mtext or a leader's text says.
fn says(object: &Object) -> Option<String> {
    match object {
        Object::Text(t) => Some(t.contents.clone()),
        Object::MText(t) => Some(t.contents.clone()),
        Object::MLeader(l) => l.content.text().map(|t| t.text.clone()),
        Object::Leader(l) => l.annotation.as_ref().map(|a| a.text.clone()),
        _ => None,
    }
}

/// Which object draws a shape: the sheet's object, and when it is an
/// insert, the path to the object of its block, through any nested inserts,
/// as [`path`] gives it.
type Key = (usize, Vec<usize>);

/// The path to the object of an insert's block that draws an item or a
/// line of text: its `part`, then its `nested` places. Empty for an object
/// of the sheet's own.
fn path(part: Option<usize>, nested: &[usize]) -> Vec<usize> {
    part.into_iter().chain(nested.iter().copied()).collect()
}

/// What draws an item or a line of text, past the sheet's object: the path
/// to the object of an insert's block, as [`path`] gives it, or a table's
/// cell, its row and column.
fn item_path(part: Option<usize>, nested: &[usize], cell: Option<[usize; 2]>) -> Vec<usize> {
    match cell {
        Some(cell) => cell.to_vec(),
        None => path(part, nested),
    }
}

/// What an object draws, for `text_on_line`.
enum Drawn {
    Letters,
    Line,
    Fill,
}

/// The box round what one line of text draws, in the line's own direction.
struct TextBox {
    key: Key,
    /// What the line says.
    says: String,
    /// The line's direction: along it, and up from it, each of length 1.
    along: Point2,
    up: Point2,
    /// The box's corners on the sheet.
    corners: [Point2; 4],
    /// The box round its corners, square to the sheet.
    seen: Bounds,
}

impl TextBox {
    /// The box round what `run` draws, as the pipeline draws it; none when
    /// it draws nothing.
    fn of(run: &TextRun) -> Option<TextBox> {
        let [lo, hi] = run.drawn?;
        let at = |u: f64, v: f64| {
            [
                run.origin[0] + u * run.across[0] + v * run.up[0],
                run.origin[1] + u * run.across[1] + v * run.up[1],
            ]
        };
        let corners = [
            at(lo[0], lo[1]),
            at(hi[0], lo[1]),
            at(hi[0], hi[1]),
            at(lo[0], hi[1]),
        ];
        let unit = |v: Point2| {
            let length = v[0].hypot(v[1]);
            (length > 0.0).then(|| [v[0] / length, v[1] / length])
        };
        Some(TextBox {
            key: (run.object, item_path(run.part, &run.nested, run.cell)),
            says: run.characters.iter().map(|&(c, _)| c).collect(),
            along: unit(run.across)?,
            up: unit(run.up)?,
            corners,
            seen: Bounds::of(corners.into_iter())?,
        })
    }

    /// How far this box and `other` overlap, when they sit on each other:
    /// how far one would move to clear the other, the least along the sides
    /// of either. None when they are apart, or overlap by no more than
    /// `tolerance`.
    ///
    /// A box with no breadth along one of those sides, such as the box
    /// round a single upright stroke like `I`, is cleared along it only by
    /// moving it off the other box's breadth there; and two such boxes on
    /// one line along it, such as two `I`s at one place, lie on each other
    /// whichever way that side would part them, so it does not.
    fn sits_on(&self, other: &TextBox, tolerance: f64) -> Option<f64> {
        let mut depth = f64::INFINITY;
        for axis in [self.along, self.up, other.along, other.up] {
            let (a_lo, a_hi) = span(&self.corners, axis);
            let (b_lo, b_hi) = span(&other.corners, axis);
            let clear = (a_hi - b_lo).min(b_hi - a_lo);
            if a_hi - a_lo <= tolerance && b_hi - b_lo <= tolerance {
                if clear < -tolerance {
                    return None;
                }
                continue;
            }
            if clear <= tolerance {
                return None;
            }
            depth = depth.min(clear);
        }
        Some(if depth.is_finite() { depth } else { 0.0 })
    }
}

/// Where `corners` reach along `axis`: from the least to the most.
fn span(corners: &[Point2; 4], axis: Point2) -> (f64, f64) {
    corners
        .iter()
        .map(|c| c[0] * axis[0] + c[1] * axis[1])
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| {
            (lo.min(d), hi.max(d))
        })
}

impl Sides {
    /// The farthest past any side.
    fn most(&self) -> f64 {
        self.left.max(self.bottom).max(self.right).max(self.top)
    }

    /// How far past which sides, in words, such as "0.5 in past the right
    /// edge of": each side past by more than `tolerance` once enlarged by
    /// `scale`. `side` names a side, such as "edge".
    fn words(&self, tolerance: f64, scale: f64, u: Unit, side: &str) -> String {
        let parts: Vec<String> = [
            (self.left, "left"),
            (self.bottom, "bottom"),
            (self.right, "right"),
            (self.top, "top"),
        ]
        .iter()
        .filter(|(by, _)| by * scale > tolerance)
        .map(|(by, which)| format!("{} {u} past the {which} {side}", num(*by)))
        .collect();
        match parts.as_slice() {
            [rest @ .., last] if !rest.is_empty() => {
                format!("{} and {last} of", rest.join(", "))
            }
            parts => format!("{} of", parts.join("")),
        }
    }
}

/// How far past a side `d` is: `d` when it is more than 0, else 0.
fn beyond(d: f64) -> f64 {
    zero_signless(d.max(0.0))
}

/// `x`, with -0 written as 0.
fn zero_signless(x: f64) -> f64 {
    x + 0.0
}

/// ", ANSI B landscape," for a paper declared by name; nothing for one
/// declared by its size.
fn paper_words(paper: &Paper) -> String {
    match &paper.name {
        Some(name) => format!(", {name},"),
        None => String::new(),
    }
}

/// A length for a message, to a millionth of the declared unit.
fn num(x: f64) -> String {
    let rounded = zero_signless((x * 1e6).round() / 1e6);
    format!("{rounded}")
}

/// A point for a message, such as `[17, 11]`.
fn point(p: Point2) -> String {
    format!("[{}, {}]", num(p[0]), num(p[1]))
}
