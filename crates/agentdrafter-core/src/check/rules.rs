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
//!   attributes together.
//! - **`text_overflow`**: each text and attribute an insert draws whose
//!   block declares its cell draws within the cell, placed by the insert's
//!   position, scale and rotation: what it draws is taken back into the
//!   block's own coordinates and held to the cell there, and how far it
//!   reaches past, enlarged by the insert's scale, is held to the
//!   tolerance on the sheet. Text with no cell is not checked.
//! - **`text_on_text`**: the box round what each text draws, seen in the
//!   text's own direction, overlaps no other text's by more than the
//!   tolerance: how far one would move to clear the other, the least
//!   overlap along the two boxes' sides. Every text on the sheet counts,
//!   each multiline text as one box round all its lines, and each text and
//!   visible attribute an insert draws, against those of the same insert
//!   too.

use std::collections::BTreeMap;

use super::{BlockPart, CheckedObject, Finding, Rule, Sides};
use crate::description::SheetRecord;
use crate::draw::{Bounds, DisplayList, Item, Point2};
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
        for item in &self.list.items {
            let had = boxes[item.object].into_iter().flat_map(|b| [b.min, b.max]);
            boxes[item.object] = Bounds::of(had.chain(item.points()));
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
                        self.text_words(i, Some(cell.object)),
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
        let mut points: BTreeMap<(usize, Option<usize>), (f64, Vec<Point2>)> = BTreeMap::new();
        for item in &self.list.items {
            if let Some(rotation) = self.text_rotation(item.object, item.part) {
                let entry = points
                    .entry((item.object, item.part))
                    .or_insert_with(|| (rotation, Vec::new()));
                entry.1.extend(item.points());
            }
        }
        let boxes: Vec<TextBox> = points
            .into_iter()
            .filter_map(|(key, (rotation, points))| TextBox::of(key, rotation, &points))
            .collect();
        // Sweep across: two boxes that overlap across by no more than the
        // tolerance overlap no more than that in any direction.
        let mut order: Vec<usize> = (0..boxes.len()).collect();
        order.sort_by(|&a, &b| boxes[a].seen.min[0].total_cmp(&boxes[b].seen.min[0]));
        let mut pairs: Vec<(usize, usize, f64)> = Vec::new();
        for (k, &a) in order.iter().enumerate() {
            for &b in &order[k + 1..] {
                if boxes[b].seen.min[0] >= boxes[a].seen.max[0] - self.tolerance {
                    break;
                }
                let depth = boxes[a].overlap(&boxes[b]);
                if depth > self.tolerance {
                    let (earlier, later) = if boxes[a].key < boxes[b].key {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    pairs.push((later, earlier, depth));
                }
            }
        }
        pairs.sort_by_key(|&(later, earlier, _)| (boxes[later].key, boxes[earlier].key));
        pairs
            .into_iter()
            .map(|(later, earlier, depth)| {
                let (later, earlier) = (&boxes[later], &boxes[earlier]);
                let mut finding = self.finding(
                    Rule::TextOnText,
                    format!(
                        "{} sits on {}: their drawn boxes overlap by {} {u}; it draws from {} to \
                         {}, and the other from {} to {}",
                        self.text_words(later.key.0, later.key.1),
                        self.text_words(earlier.key.0, earlier.key.1),
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

    /// The direction the text the sheet's object `i` draws runs in, in
    /// degrees, when it is text: a text or an mtext of the sheet's own, or
    /// for an insert the text, mtext or attribute of its block at `part`.
    /// None for anything else.
    fn text_rotation(&self, i: usize, part: Option<usize>) -> Option<f64> {
        match (&self.record.sheet.objects[i], part) {
            (Object::Text(t), None) => Some(t.rotation),
            (Object::MText(t), None) => Some(t.rotation),
            (Object::Insert(insert), Some(j)) => {
                let own = match self.block(insert)?.objects.get(j)? {
                    Object::Text(t) => t.rotation,
                    Object::MText(t) => t.rotation,
                    Object::AttributeDefinition(d) => d.rotation,
                    _ => return None,
                };
                Some(own + insert.rotation)
            }
            _ => None,
        }
    }

    /// The sheet's object `i`, as a finding names it: by its ID, its name
    /// and where it is written, and for `part` of an insert, which object
    /// of its block.
    fn checked(&self, i: usize, part: Option<usize>) -> CheckedObject {
        let record = self.record;
        let object = &record.sheet.objects[i];
        let mut in_block = None;
        let mut text = None;
        match (object, part) {
            (Object::Text(t), None) => text = Some(t.contents.clone()),
            (Object::MText(t), None) => text = Some(t.contents.clone()),
            (Object::Insert(insert), Some(j)) => {
                if let Some((block, inner)) = self
                    .block(insert)
                    .and_then(|block| block.objects.get(j).map(|o| (block, o)))
                {
                    let tag = match inner {
                        Object::AttributeDefinition(d) => Some(d.tag.clone()),
                        _ => None,
                    };
                    text = self.says(insert, inner);
                    in_block = Some(BlockPart {
                        block: block.name.clone(),
                        file: self.library_file(),
                        field: format!("blocks.{}.objects[{j}]", block.name),
                        kind: inner.type_name().to_owned(),
                        tag,
                    });
                }
            }
            _ => {}
        }
        CheckedObject {
            id: record.id(i),
            name: record.objects[i].name.clone(),
            kind: object.type_name().to_owned(),
            written_at: written_at(record, i),
            in_block,
            text,
        }
    }

    /// What a text, an mtext or an attribute of an insert's block says on
    /// the insert.
    fn says(&self, insert: &Insert, inner: &Object) -> Option<String> {
        match inner {
            Object::Text(t) => Some(t.contents.clone()),
            Object::MText(t) => Some(t.contents.clone()),
            Object::AttributeDefinition(d) => Some(
                insert
                    .attributes
                    .iter()
                    .find(|a| a.tag == d.tag)
                    .map_or(d.default.clone(), |a| a.value.clone()),
            ),
            _ => None,
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
    /// `DEMO-001/objects[2]`".
    fn text_words(&self, i: usize, part: Option<usize>) -> String {
        let object = &self.record.sheet.objects[i];
        let insert_words = self.object_words(i);
        match (object, part) {
            (Object::Text(t), None) => format!("{insert_words} saying `{}`", t.contents),
            (Object::Insert(insert), Some(j)) => {
                let Some(block) = self.block(insert) else {
                    return insert_words;
                };
                match block.objects.get(j) {
                    Some(Object::AttributeDefinition(d)) => format!(
                        "the attribute {} of {insert_words} saying `{}`",
                        d.tag,
                        self.says(insert, &block.objects[j]).unwrap_or_default()
                    ),
                    Some(Object::Text(t)) => format!(
                        "the text `{}` of the block {} that {insert_words} inserts",
                        t.contents, block.name
                    ),
                    Some(Object::MText(_)) => format!(
                        "the mtext at `blocks.{}.objects[{j}]` that {insert_words} inserts",
                        block.name
                    ),
                    _ => insert_words,
                }
            }
            _ => insert_words,
        }
    }
}

/// The box round what one text draws, in its own direction.
struct TextBox {
    /// The sheet's object that draws it, and the object of its block when
    /// it is an insert's.
    key: (usize, Option<usize>),
    /// The text's direction: along it, and up from it.
    along: Point2,
    up: Point2,
    /// The box's corners on the sheet.
    corners: [Point2; 4],
    /// The box round what it draws, seen from above, square to the sheet.
    seen: Bounds,
}

impl TextBox {
    /// The box round `points`, drawn by `key` running at `rotation`
    /// degrees; none when it draws nothing.
    fn of(key: (usize, Option<usize>), rotation: f64, points: &[Point2]) -> Option<TextBox> {
        let seen = Bounds::of(points.iter().copied())?;
        let (sin, cos) = rotation.to_radians().sin_cos();
        let along = [cos, sin];
        let up = [-sin, cos];
        let own = Bounds::of(points.iter().map(|&p| [dot(p, along), dot(p, up)]))?;
        let corner = |a: f64, b: f64| [a * along[0] + b * up[0], a * along[1] + b * up[1]];
        Some(TextBox {
            key,
            along,
            up,
            corners: [
                corner(own.min[0], own.min[1]),
                corner(own.max[0], own.min[1]),
                corner(own.max[0], own.max[1]),
                corner(own.min[0], own.max[1]),
            ],
            seen,
        })
    }

    /// How far this box and `other` overlap: the least overlap along the
    /// sides of either, which is how far one would move to clear the
    /// other. 0 or less when they do not overlap.
    fn overlap(&self, other: &TextBox) -> f64 {
        [self.along, self.up, other.along, other.up]
            .iter()
            .map(|&axis| {
                let span = |corners: &[Point2; 4]| {
                    corners
                        .iter()
                        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &c| {
                            let d = dot(c, axis);
                            (lo.min(d), hi.max(d))
                        })
                };
                let (a_lo, a_hi) = span(&self.corners);
                let (b_lo, b_hi) = span(&other.corners);
                a_hi.min(b_hi) - a_lo.max(b_lo)
            })
            .fold(f64::INFINITY, f64::min)
    }
}

fn dot(p: Point2, axis: Point2) -> f64 {
    p[0] * axis[0] + p[1] * axis[1]
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
