//! Proposals waiting (#155), drawn dashed on the sheet as it is, as the
//! prototype Zach picked on 2026-11-19 draws them on the drawing (its
//! variant B): what each would add or change in the proposal's purple, its
//! strokes dashed and its text solid, each object in a dashed box; what it
//! would delete boxed and struck through; and a dashed guide from where an
//! object it only moves is now to where it would go. A proposal the set
//! would refuse now is drawn fainter.
//!
//! The core says what is what: each proposal's `display-list` with its
//! change set gives the sheet as it would leave it, with its `preview`.
//! This module keeps, of each, only what it marks, in drawing units; `web`
//! draws them over the ink of the sheet as it is, and the page puts each
//! proposal's card by its extent.

use agentdrafter_core::{Bounds, Shape};

use crate::mesh::read_reply;
use crate::overlay::Segment;
use crate::preview::{self, bounds_of, Line, Preview};

/// A proposal's colour, as the prototype draws it.
pub const PROPOSAL: [f32; 4] = [0xc9 as f32 / 255.0, 0xa8 as f32 / 255.0, 1.0, 1.0];
/// The boxes and guides round what it marks, dimmer.
pub const PROPOSAL_DIM: [f32; 4] = [
    0x8c as f32 / 255.0,
    0x6b as f32 / 255.0,
    0xd6 as f32 / 255.0,
    0.9,
];
/// How much fainter a proposal the set would refuse now is drawn.
pub const REFUSED_ALPHA: f32 = 0.6;

/// One run of a proposal's marks: the lines one item draws through, in
/// drawing units, its pen in millimetres on paper (none for a fill), and
/// whether it is a fill, whose outline is drawn solid, as text is.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub lines: Vec<Line>,
    pub pen: Option<f64>,
    pub fill: bool,
}

/// A waiting proposal, as the renderer draws it over one sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub proposal: u32,
    /// The sheet and the layout it was drawn on, which it is drawn over.
    pub sheet: String,
    pub layout: Option<String>,
    /// Whether the set would refuse it now: drawn fainter.
    pub refused: bool,
    /// What it would add and change, item by item.
    pub runs: Vec<Run>,
    /// A box round each object it would add or change.
    pub boxes: Vec<Bounds>,
    /// A box round each object it would delete, struck through.
    pub struck: Vec<Bounds>,
    /// From where each object it only moves is, to where it would go.
    pub moves: Vec<Line>,
}

impl Ghost {
    /// Proposal `proposal`'s marks, from the text of `display-list`'s reply
    /// with its change set.
    pub fn of(proposal: u32, reply: &str, refused: bool) -> Result<Ghost, String> {
        let sheet = read_reply(reply)?;
        let ids: Vec<Option<String>> = sheet
            .objects
            .iter()
            .map(|o| o.get("id").and_then(|id| id.as_str()).map(str::to_owned))
            .collect();
        let list = &sheet.display_list;
        let marked = Preview::of(sheet.preview.as_ref(), list, &ids)
            .ok_or_else(|| "the reply holds no preview of the proposal".to_owned())?;
        let runs = marked
            .added
            .iter()
            .chain(&marked.changed)
            .map(|&i| {
                let item = &list.items[i];
                Run {
                    lines: preview::outline(item),
                    pen: preview::pen(item),
                    fill: matches!(item.shape, Shape::Fill { .. }),
                }
            })
            .collect();
        let boxes = marked
            .marked_objects(list)
            .into_iter()
            .map(|(_, b)| b)
            .collect();
        let struck = marked
            .was
            .iter()
            .filter(|w| w.deleted)
            .filter_map(|w| bounds_of(w.items.iter()))
            .collect();
        Ok(Ghost {
            proposal,
            sheet: sheet.sheet,
            layout: sheet.layout,
            refused,
            runs,
            boxes,
            struck,
            moves: marked.moves,
        })
    }

    /// Whether it is drawn over `sheet` in `layout`.
    pub fn on(&self, sheet: &str, layout: Option<&str>) -> bool {
        self.sheet.eq_ignore_ascii_case(sheet)
            && self.layout.as_deref().map(str::to_ascii_uppercase)
                == layout.map(str::to_ascii_uppercase)
    }

    /// The box round everything it marks, in drawing units, which its card
    /// is put by; none when it marks nothing on the sheet.
    pub fn extent(&self) -> Option<Bounds> {
        self.boxes
            .iter()
            .chain(&self.struck)
            .copied()
            .reduce(|a, b| Bounds {
                min: [a.min[0].min(b.min[0]), a.min[1].min(b.min[1])],
                max: [a.max[0].max(b.max[0]), a.max[1].max(b.max[1])],
            })
    }
}

/// `run`, device pixels, one item's lines in order, cut into dashes `dash`
/// long with gaps of `gap`, the pattern carried on where one line joins
/// the next, as a dashed pen runs along a polyline. A run shorter than
/// three dashes, such as a letter's stroke, is drawn whole.
///
/// The pen steps from one end of a dash or a gap to the next, each step
/// at least a hair long, so it always moves on, whatever the rounding of a
/// fractional device-pixel ratio's lengths.
pub fn dash_run(run: &[Segment], dash: f64, gap: f64) -> Vec<Segment> {
    let length: f64 = run
        .iter()
        .map(|[x0, y0, x1, y1]| (x1 - x0).hypot(y1 - y0))
        .sum();
    if !(dash > 0.0 && gap > 0.0 && length.is_finite()) || length < 3.0 * dash {
        return run.to_vec();
    }
    // Shorter than this is no length at all.
    let hair = 1e-9 * (dash + gap).max(1.0);
    let mut out = Vec::new();
    // Whether the pen is down, and how much of this dash or gap is left.
    let (mut down, mut left) = (true, dash);
    let mut last: Option<(f64, f64)> = None;
    for &[x0, y0, x1, y1] in run {
        // A line that does not start where the last ended starts the
        // pattern again.
        if last.is_some_and(|(x, y)| (x - x0).abs() > 0.5 || (y - y0).abs() > 0.5) {
            (down, left) = (true, dash);
        }
        last = Some((x1, y1));
        let l = (x1 - x0).hypot(y1 - y0);
        let at = |d: f64| (x0 + (x1 - x0) * d / l, y0 + (y1 - y0) * d / l);
        let mut t = 0.0;
        while l - t > hair {
            let step = left.min(l - t);
            if down {
                let (ax, ay) = at(t);
                let (bx, by) = at(t + step);
                out.push([ax, ay, bx, by]);
            }
            t += step;
            left -= step;
            if left <= hair {
                down = !down;
                left = if down { dash } else { gap };
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "ghosts_tests.rs"]
mod tests;
