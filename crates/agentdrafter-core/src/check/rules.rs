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
//!   tolerance on the sheet. Text with no cell is not checked.
//! - **`text_on_text`**: the box round what each line of text draws, seen
//!   in the line's own direction, as the pipeline draws the line, overlaps
//!   no other text's by more than the tolerance: how far one would move to
//!   clear the other, the least along the sides of either box. A box with
//!   no breadth, round a single upright stroke such as `I`, counts too.
//!   Every text on the sheet counts, each line of a multiline text on its
//!   own, and each text and visible attribute an insert draws, against
//!   those of the same insert too.

use std::collections::BTreeMap;

use super::{BlockPart, CheckedObject, Finding, Rule, Sides};
use crate::description::SheetRecord;
use crate::draw::{Bounds, DisplayList, Item, Point2, TextRun};
use crate::model::block::Placement;
use crate::model::{Block, Insert, Object, Paper};
use crate::query::{measuring_drawing, written_at, Extents};
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
    let on = Sheet {
        record,
        list: &list,
        units,
        tolerance,
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
    findings
}

/// One sheet being checked, drawn.
struct Sheet<'a> {
    record: &'a SheetRecord,
    list: &'a DisplayList,
    units: Unit,
    tolerance: f64,
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
            finding.object = Some(self.checked(i, None));
            finding.drawn = Some(Extents::of(b));
            finding.paper = Some(paper.clone());
            finding.past = Some(past);
            findings.push(finding);
        }
        findings
    }

    /// A finding for each text or attribute an insert draws past the cell
    /// its block declares for it.
    fn text_overflow(&self) -> Vec<Finding> {
        let u = self.units;
        let mut findings = Vec::new();
        for (i, object) in self.record.sheet.objects.iter().enumerate() {
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
                    .filter(|item| item.object == i && item.part == Some(cell.object))
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
                        self.text_words(i, Some(cell.object), ""),
                        past.words(self.tolerance, insert.scale, u, "side"),
                        block.name,
                        point(b.min),
                        point(b.max),
                        point(cell.min),
                        point(cell.max),
                    ),
                );
                finding.object = Some(self.checked(i, Some(cell.object)));
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
                    .entry((boxes[later].key, boxes[earlier].key))
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
                        self.text_words(later.key.0, later.key.1, &later.says),
                        self.text_words(earlier.key.0, earlier.key.1, &earlier.says),
                        num(depth),
                        point(later.seen.min),
                        point(later.seen.max),
                        point(earlier.seen.min),
                        point(earlier.seen.max),
                    ),
                );
                finding.object = Some(self.checked(later.key.0, later.key.1));
                finding.other = Some(self.checked(earlier.key.0, earlier.key.1));
                finding.drawn = Some(Extents::of(&later.seen));
                finding.other_drawn = Some(Extents::of(&earlier.seen));
                finding.overlap = Some(depth);
                finding
            })
            .collect()
    }

    /// The block an insert places, as the sheet holds it.
    fn block(&self, insert: &Insert) -> Option<&Block> {
        self.record
            .sheet
            .blocks
            .iter()
            .find(|b| b.name == insert.block)
    }

    /// The object of an insert's block at `part` that the sheet's object
    /// `i` draws, when `i` is an insert.
    fn in_block(&self, i: usize, part: Option<usize>) -> Option<InBlock<'_>> {
        let Object::Insert(insert) = &self.record.sheet.objects[i] else {
            return None;
        };
        let block = self.block(insert)?;
        let index = part?;
        Some(InBlock {
            insert,
            block,
            index,
            object: block.objects.get(index)?,
        })
    }

    /// The sheet's object `i`, as a finding names it: by its ID, its name
    /// and where it is written, and for `part` of an insert, which object
    /// of its block.
    fn checked(&self, i: usize, part: Option<usize>) -> CheckedObject {
        let record = self.record;
        let object = &record.sheet.objects[i];
        let (in_block, text) = match self.in_block(i, part) {
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

    /// The text the sheet's object `i` draws, at `part` of an insert's
    /// block, in words, such as "the attribute TITLE of the insert
    /// `DEMO-001/objects[2]`". `line` is the line of an mtext meant.
    fn text_words(&self, i: usize, part: Option<usize>, line: &str) -> String {
        let words = self.object_words(i);
        let on_line = format!(" on its line `{line}`");
        let Some(inner) = self.in_block(i, part) else {
            return match &self.record.sheet.objects[i] {
                Object::Text(t) => format!("{words} saying `{}`", t.contents),
                Object::MText(_) => format!("{words}{on_line}"),
                _ => words,
            };
        };
        match inner.object {
            Object::AttributeDefinition(d) => format!(
                "the attribute {} of {words} saying `{}`",
                d.tag,
                inner.says().unwrap_or_default()
            ),
            Object::Text(t) => format!(
                "the text `{}` of the block {} that {words} inserts",
                t.contents, inner.block.name
            ),
            Object::MText(_) => format!(
                "the mtext at `{}`{on_line}, that {words} inserts",
                inner.field()
            ),
            _ => words,
        }
    }
}

/// One object of an insert's block, as the insert draws it.
struct InBlock<'a> {
    insert: &'a Insert,
    block: &'a Block,
    /// Its place in the block's objects.
    index: usize,
    object: &'a Object,
}

impl InBlock<'_> {
    /// Its place in the block library, such as
    /// `blocks.TITLE_BLOCK.objects[13]`.
    fn field(&self) -> String {
        format!("blocks.{}.objects[{}]", self.block.name, self.index)
    }

    /// What it says on the insert, when it is text: a text's or an mtext's
    /// contents, or the insert's value of an attribute.
    fn says(&self) -> Option<String> {
        match self.object {
            Object::AttributeDefinition(d) => Some(
                self.insert
                    .attributes
                    .iter()
                    .find(|a| a.tag == d.tag)
                    .map_or(d.default.clone(), |a| a.value.clone()),
            ),
            object => says(object),
        }
    }
}

/// What a text or an mtext says.
fn says(object: &Object) -> Option<String> {
    match object {
        Object::Text(t) => Some(t.contents.clone()),
        Object::MText(t) => Some(t.contents.clone()),
        _ => None,
    }
}

/// Which text draws a line: the sheet's object, and the object of its block
/// when it is an insert's.
type Key = (usize, Option<usize>);

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
            key: (run.object, run.part),
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
