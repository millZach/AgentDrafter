//! A change set drawn before it lands (#154), as the prototype Zach picked
//! on 2026-11-19 draws it on the drawing: the sheet as the change set would
//! leave it, from `display-list` with the change set, the rest of it faded,
//! as AutoCAD's REFEDIT fades what is not being edited (XFADECTL); what it
//! would add in green and change in amber, each in a dashed box; the old
//! place of what it would change faint, with a dashed guide from it where
//! the object only moves, as Move's ghost; what it would delete faint red
//! and struck through; and a revision cloud, red, round each check the
//! sheet would fail, as AutoCAD's DWG Compare clouds what differs.
//!
//! The core says what is what: the items it adds and changes, what changes
//! and deletes draw now, and the boxes of the checks. This module lays out
//! the lines that mark them, in plain Rust; `web` draws them over the ink.

use agentdrafter_core::{Bounds, DisplayList, Item, Shape};
use serde::Deserialize;
use serde_json::Value;

use crate::overlay::Segment;

/// How much of the ink the fade covers with model space's colour, as
/// REFEDIT's default XFADECTL of 50 to 70 fades what is not edited.
pub const FADE: f32 = 0.62;

/// What a change set would add, as the prototype marks it.
pub const ADDED: [f32; 4] = rgba(0x3d_dc_84, 1.0);
/// What it would change.
pub const CHANGED: [f32; 4] = rgba(0xff_b0_20, 1.0);
/// The old place of what it would change, faint.
pub const OLD_PLACE: [f32; 4] = rgba(0xc8_d0_da, 0.5);
/// What it would delete, faint red, and its strike.
pub const DELETED: [f32; 4] = rgba(0xff_6b_6b, 0.7);
/// A check it would fail: the cloud round it.
pub const FAILS: [f32; 4] = rgba(0xff_5d_5d, 1.0);

const fn rgba(hex: u32, alpha: f32) -> [f32; 4] {
    [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        alpha,
    ]
}

/// The most segments of an object's outline drawn at once.
const MOST_SEGMENTS: usize = 50_000;

/// A segment in drawing units.
pub type Line = [[f64; 2]; 2];

/// What one object would no longer draw: its items as they draw now.
#[derive(Clone, Debug, PartialEq)]
pub struct Was {
    pub id: String,
    pub deleted: bool,
    pub items: Vec<Item>,
}

/// A preview, as the renderer draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    /// The places among the display list's items of what it would add.
    pub added: Vec<usize>,
    /// Those of what it would change.
    pub changed: Vec<usize>,
    pub was: Vec<Was>,
    /// The old clipped viewports' boundaries that `was` items reference under
    /// `clip_boundary` (#188).
    pub was_clip_boundaries: Vec<Vec<[f64; 2]>>,
    /// The boxes of the checks the sheet would fail.
    pub clouds: Vec<Bounds>,
    /// A guide from where each object it would only move is now, to where
    /// it would go.
    pub moves: Vec<Line>,
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    added: Vec<usize>,
    #[serde(default)]
    changed: Vec<usize>,
    #[serde(default)]
    was: Vec<WasReply>,
    #[serde(default)]
    was_clip_boundaries: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    clouds: Vec<CloudReply>,
}

#[derive(Deserialize)]
struct WasReply {
    id: String,
    how: String,
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct CloudReply {
    bounds: Bounds,
}

impl Preview {
    /// The preview a `display-list` reply gives under `preview`, on the
    /// sheet drawn to `list`, whose objects' IDs are `ids` by their place;
    /// none when it gives none, or one that cannot be read.
    pub fn of(preview: Option<&Value>, list: &DisplayList, ids: &[Option<String>]) -> Option<Self> {
        let reply: Reply = serde_json::from_value(preview?.clone()).ok()?;
        let items = |places: &[usize]| -> Vec<usize> {
            places
                .iter()
                .copied()
                .filter(|&i| i < list.items.len())
                .collect()
        };
        let added = items(&reply.added);
        let changed = items(&reply.changed);
        let was: Vec<Was> = reply
            .was
            .into_iter()
            .map(|w| Was {
                deleted: w.how == "deleted",
                id: w.id,
                items: w.items,
            })
            .collect();
        // A changed object whose old place and new are the same size has
        // only moved: a guide joins them.
        let moves = was
            .iter()
            .filter(|w| !w.deleted)
            .filter_map(|w| {
                let old = bounds_of(w.items.iter())?;
                let new = bounds_of(changed.iter().map(|&i| &list.items[i]).filter(|item| {
                    ids.get(item.object)
                        .and_then(Option::as_deref)
                        .is_some_and(|id| id.eq_ignore_ascii_case(&w.id))
                }))?;
                moved(old, new)
            })
            .collect();
        Some(Preview {
            added,
            changed,
            was,
            was_clip_boundaries: reply.was_clip_boundaries,
            clouds: reply.clouds.into_iter().map(|c| c.bounds).collect(),
            moves,
        })
    }

    /// The box round everything the preview marks, in drawing units, for
    /// the view to come to: what it adds and changes, the old places, and
    /// the clouds. None when it marks nothing.
    pub fn extent(&self, list: &DisplayList) -> Option<Bounds> {
        let marked = self
            .added
            .iter()
            .chain(&self.changed)
            .map(|&i| &list.items[i])
            .chain(self.was.iter().flat_map(|w| &w.items));
        let boxes = bounds_of(marked)
            .into_iter()
            .chain(self.clouds.iter().copied());
        boxes.reduce(union)
    }

    /// Each object marked added or changed, its items' places grouped by
    /// the object they draw, in the order they come: for a dashed box round
    /// each.
    pub fn marked_objects(&self, list: &DisplayList) -> Vec<(bool, Bounds)> {
        let mut found: Vec<(usize, bool, Bounds)> = Vec::new();
        for (added, places) in [(true, &self.added), (false, &self.changed)] {
            for &i in places {
                let item = &list.items[i];
                let Some(b) = bounds_of(std::iter::once(item)) else {
                    continue;
                };
                match found
                    .iter_mut()
                    .find(|(o, a, _)| *o == item.object && *a == added)
                {
                    Some((_, _, all)) => *all = union(*all, b),
                    None => found.push((item.object, added, b)),
                }
            }
        }
        found.into_iter().map(|(_, a, b)| (a, b)).collect()
    }
}

/// The box round what `items` draw.
pub fn bounds_of<'a>(items: impl Iterator<Item = &'a Item>) -> Option<Bounds> {
    items
        .flat_map(|item| item.points().collect::<Vec<_>>())
        .fold(None, |b: Option<Bounds>, p| {
            Some(match b {
                None => Bounds { min: p, max: p },
                Some(b) => union(b, Bounds { min: p, max: p }),
            })
        })
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        min: [a.min[0].min(b.min[0]), a.min[1].min(b.min[1])],
        max: [a.max[0].max(b.max[0]), a.max[1].max(b.max[1])],
    }
}

/// The guide from the middle of `old` to the middle of `new`, when the two
/// are the same size, a move, and not at one place.
pub fn moved(old: Bounds, new: Bounds) -> Option<Line> {
    let size = old
        .width()
        .max(old.height())
        .max(new.width())
        .max(new.height());
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-6 * size.max(1.0);
    let middle = |b: Bounds| [(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0];
    let (a, b) = (middle(old), middle(new));
    let same_size = close(old.width(), new.width()) && close(old.height(), new.height());
    (same_size && !(close(a[0], b[0]) && close(a[1], b[1]))).then_some([a, b])
}

/// The lines an item draws through, in drawing units: each run of a
/// stroke, and each contour of a fill, closed.
pub fn outline(item: &Item) -> Vec<Line> {
    let owned_image: Vec<Vec<[f64; 2]>>;
    let (lists, closed) = match &item.shape {
        Shape::Stroke { runs, .. } => (runs, false),
        Shape::Fill { contours, .. } | Shape::Mask { contours } => (contours, true),
        Shape::Image {
            origin,
            u_axis,
            v_axis,
            clip,
            clip_inverted,
            ..
        } => {
            owned_image = vec![Shape::image_outline(
                *origin,
                *u_axis,
                *v_axis,
                clip.as_deref(),
                *clip_inverted,
            )];
            (&owned_image, true)
        }
    };
    let mut lines = Vec::new();
    for run in lists {
        for pair in run.windows(2) {
            lines.push([pair[0], pair[1]]);
        }
        if closed && run.len() > 2 {
            lines.push([run[run.len() - 1], run[0]]);
        }
        if run.len() == 1 {
            lines.push([run[0], run[0]]);
        }
        if lines.len() > MOST_SEGMENTS {
            break;
        }
    }
    lines
}

/// The pen an item is drawn with, in millimetres on paper; none for a fill.
pub fn pen(item: &Item) -> Option<f64> {
    match &item.shape {
        Shape::Stroke { lineweight, .. } => Some(*lineweight),
        Shape::Fill { .. } | Shape::Mask { .. } | Shape::Image { .. } => None,
    }
}

/// The viewport clip polygon for `item` in drawing units, resolved against
/// `boundaries` (#188): its non-rectangular `clip_boundary` when present and
/// valid (`>= 3` vertices), otherwise its rectangular `clip` window as four
/// corners, or `None` when the item is not clipped by a viewport.
pub fn clip_polygon(item: &Item, boundaries: &[Vec<[f64; 2]>]) -> Option<Vec<[f64; 2]>> {
    if let Some(b) = item.clip_boundary {
        if let Some(poly) = boundaries.get(b) {
            if poly.len() >= 3 {
                return Some(poly.clone());
            }
        }
    }
    item.clip.map(|b| {
        vec![
            [b.min[0], b.min[1]],
            [b.max[0], b.min[1]],
            [b.max[0], b.max[1]],
            [b.min[0], b.max[1]],
        ]
    })
}

/// Triangulates `outline` (drawing units) into a device-pixel triangle fan
/// from its first vertex for even-odd stencil filling, matching `mesh.rs` (#188).
pub fn stencil_fan(outline: &[[f64; 2]], device: &dyn Fn(f64, f64) -> (f64, f64)) -> Vec<f32> {
    if outline.len() < 3 {
        return Vec::new();
    }
    let (x0, y0) = device(outline[0][0], outline[0][1]);
    let mut points = Vec::with_capacity((outline.len() - 2) * 6);
    for pair in outline[1..].windows(2) {
        let (x1, y1) = device(pair[0][0], pair[0][1]);
        let (x2, y2) = device(pair[1][0], pair[1][1]);
        points.extend_from_slice(&[
            x0 as f32, y0 as f32, x1 as f32, y1 as f32, x2 as f32, y2 as f32,
        ]);
    }
    points
}

/// `line`, device pixels, cut into dashes `dash` long with gaps of `gap`.
pub fn dashed(line: Segment, dash: f64, gap: f64) -> Vec<Segment> {
    let [x0, y0, x1, y1] = line;
    let length = (x1 - x0).hypot(y1 - y0);
    if length <= dash || dash <= 0.0 {
        return vec![line];
    }
    let at = |d: f64| (x0 + (x1 - x0) * d / length, y0 + (y1 - y0) * d / length);
    let mut dashes = Vec::new();
    let mut t = 0.0;
    while t < length {
        let (ax, ay) = at(t);
        let (bx, by) = at((t + dash).min(length));
        dashes.push([ax, ay, bx, by]);
        t += dash + gap;
    }
    dashes
}

/// The four sides of the box `[left, top, right, bottom]`, device pixels.
pub fn sides([l, t, r, b]: [f64; 4]) -> [Segment; 4] {
    [[l, t, r, t], [r, t, r, b], [r, b, l, b], [l, b, l, t]]
}

/// The two strokes of an X across the box `[left, top, right, bottom]`,
/// device pixels: what strikes an object a change set would delete.
pub fn strike([l, t, r, b]: [f64; 4]) -> [Segment; 2] {
    [[l, b, r, t], [l, t, r, b]]
}

/// A revision cloud round the box `[left, top, right, bottom]`, device
/// pixels, as AutoCAD's REVCLOUD draws one: arcs bulging outwards along
/// each side, each about `arc` long, never more than half a side; each arc
/// is a half circle drawn as chords.
pub fn cloud([l, t, r, b]: [f64; 4], arc: f64) -> Vec<Segment> {
    let mut lines = Vec::new();
    // Clockwise on the screen, so outwards is to the left of each side.
    for [x0, y0, x1, y1] in [[l, t, r, t], [r, t, r, b], [r, b, l, b], [l, b, l, t]] {
        let length = (x1 - x0).hypot(y1 - y0);
        let n = (length / arc.max(1.0)).round().max(2.0) as usize;
        let (dx, dy) = ((x1 - x0) / n as f64, (y1 - y0) / n as f64);
        // Outwards: the side's direction turned a quarter, away from the
        // box's middle (y grows downwards on the screen).
        let (ox, oy) = (dy, -dx);
        for k in 0..n {
            let (sx, sy) = (x0 + dx * k as f64, y0 + dy * k as f64);
            let (cx, cy) = (sx + dx / 2.0, sy + dy / 2.0);
            let steps = 8;
            let mut last = (sx, sy);
            for s in 1..=steps {
                let a = std::f64::consts::PI * f64::from(s) / f64::from(steps);
                // From the arc's start, round through its outward bulge, to
                // its end.
                let (cos, sin) = (a.cos(), a.sin());
                let x = cx - dx / 2.0 * cos + ox / 2.0 * sin;
                let y = cy - dy / 2.0 * cos + oy / 2.0 * sin;
                lines.push([last.0, last.1, x, y]);
                last = (x, y);
            }
        }
    }
    lines
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
