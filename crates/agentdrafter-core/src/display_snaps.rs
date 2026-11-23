//! The object snaps a display list gives with `snaps`, so the viewer can
//! find the snap nearest the cursor with no round trip: each object's own
//! (endpoint, midpoint, centre, quadrant, insertion and node), and each
//! intersection between two objects, worked out by the same snap code as
//! `query snap` (`geometry/snap.rs`), so a snap here and the same object,
//! snap and `which` asked of `query snap` give the same point.
//!
//! Only geometry drawn on the page itself has snaps here: every object of
//! a sheet with no layout, and the paper space of the layout drawn, and of
//! an insert only what its block draws. Model space seen through a
//! viewport, and an object on a layer that is off or frozen, an object of a
//! block among them, draw nothing of their own on the page, and have none.
//!
//! An intersection is listed once, under the earlier of its two objects,
//! `with` the later, named as `query snap --with` names it: its place in
//! the order the two cross along the first, when they cross more than once,
//! counted over all the first's geometry, as `query snap` counts it. Only
//! pairs whose geometry's boxes overlap are looked at, the geometry as it
//! is and not as a linetype dots it, found through a grid so a sheet of
//! many objects is not every pair.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::draw::{DisplayList, Item, Shape};
use crate::geometry::snap::{intersections, own_points};
use crate::geometry::{Part, Reader, SnapKind};
use crate::model::{Object, Point3, Sheet};
use crate::text::placement::SheetFonts;

/// One point an object snap finds on the sheet drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplaySnap {
    /// The object's place in the sheet's objects, as each item of the
    /// display list names it.
    pub object: usize,
    /// The snap.
    pub snap: SnapKind,
    /// Which of the object's points of that kind it is, as `query snap`
    /// names it; none when the object has only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub which: Option<String>,
    /// For an intersection, the other object's place in the sheet's objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<usize>,
    /// Where it is, in drawing units.
    pub at: Point3,
}

/// The snaps of `sheet`'s objects that `list` draws on the page itself,
/// with fonts `fonts`.
pub(crate) fn snaps_of(sheet: &Sheet, fonts: &SheetFonts, list: &DisplayList) -> Vec<DisplaySnap> {
    let reader = Reader { sheet, fonts };
    // The objects the page draws itself, in their order, each with the
    // places in its block of what it draws: `[3, 1]` for the object 1 of
    // the block nested at 3, none for an object of the sheet's own.
    let mut drawn: BTreeSet<usize> = BTreeSet::new();
    let mut chains: HashMap<usize, BTreeSet<Vec<usize>>> = HashMap::new();
    for item in list.items.iter().filter(|item| item.viewport.is_none()) {
        drawn.insert(item.object);
        if let Some(part) = item.part {
            let mut chain = vec![part];
            chain.extend(&item.nested);
            chains.entry(item.object).or_default().insert(chain);
        }
    }
    // Whether the geometry at `place` in object `object` is drawn: an
    // object's own always, as the object is; a block's when it draws.
    let shown = |object: usize, place: Option<&str>| {
        let chain = place.map(chain_of).unwrap_or_default();
        chain.is_empty()
            || chains
                .get(&object)
                .is_some_and(|drawn| drawn.contains(&chain))
    };
    let mut snaps = Vec::new();
    for &i in &drawn {
        let Some(object) = sheet.objects.get(i) else {
            continue;
        };
        for (_, point) in own_points(&reader, object, None) {
            if !shown(i, point.which.as_deref()) {
                continue;
            }
            snaps.push(DisplaySnap {
                object: i,
                snap: point.snap,
                which: point.which,
                with: None,
                at: point.at,
            });
        }
    }
    // Each object's curves, and the box round those the page draws, once.
    let shapes: Vec<(usize, Vec<Part>, Box2)> = drawn
        .iter()
        .filter_map(|&i| {
            let parts = reader.parts(sheet.objects.get(i)?);
            let bounds = box_of(parts.iter().filter(|part| shown(i, Some(&part.path))))?;
            Some((i, parts, bounds))
        })
        .collect();
    let boxes: Vec<Box2> = shapes.iter().map(|(_, _, b)| *b).collect();
    // Each pair is looked at as the grid finds it, and only the crossings
    // kept, so a sheet of many boxes that overlap but never cross holds
    // no list of its pairs; they are put in order at the end.
    let mut crossings: Vec<(usize, usize, Vec<DisplaySnap>)> = Vec::new();
    overlapping(&boxes, &mut |k, l| {
        let (i, parts, _) = &shapes[k];
        let (j, others, _) = &shapes[l];
        let found = drawn_crossings(*i, parts, *j, others, &shown);
        if !found.is_empty() {
            crossings.push((k, l, found));
        }
    });
    crossings.sort_unstable_by_key(|(k, l, _)| (*k, *l));
    snaps.extend(crossings.into_iter().flat_map(|(_, _, found)| found));
    snaps
}

/// Where object `i`'s parts cross object `j`'s on the page: named as
/// `query snap` names them, counted over all of both, and kept where a
/// part the page draws of each crosses there. A crossing merged with one
/// of geometry not drawn, at the same point, is kept by the drawn one.
fn drawn_crossings(
    i: usize,
    parts: &[Part],
    j: usize,
    others: &[Part],
    shown: &dyn Fn(usize, Option<&str>) -> bool,
) -> Vec<DisplaySnap> {
    let all = intersections(parts, others, false);
    if all.is_empty() {
        return Vec::new();
    }
    let drawn = |object: usize, of: &[Part]| -> Vec<Part> {
        of.iter()
            .filter(|part| shown(object, Some(&part.path)))
            .cloned()
            .collect()
    };
    let (seen, seen_others) = (drawn(i, parts), drawn(j, others));
    let every = seen.len() == parts.len() && seen_others.len() == others.len();
    let there = if every {
        Vec::new()
    } else {
        intersections(&seen, &seen_others, false)
    };
    all.into_iter()
        .filter(|point| {
            every
                || there.iter().any(|q| {
                    let size = [point.at.x, point.at.y, point.at.z]
                        .iter()
                        .fold(1.0_f64, |most, v| most.max(v.abs()));
                    (q.at.x - point.at.x)
                        .hypot(q.at.y - point.at.y)
                        .hypot(q.at.z - point.at.z)
                        <= 1e-7 * size
                })
        })
        .map(|point| DisplaySnap {
            object: i,
            snap: SnapKind::Intersection,
            which: point.which,
            with: Some(j),
            at: point.at,
        })
        .collect()
}

/// The places among `list`'s items of the fills on the page itself that a
/// line or a curve draws, a polyline with width, a solid or a donut, whose
/// edges the nearest snap finds a point on as it does along a stroke: not
/// text's, a hatch's, which AutoCAD snaps to none of while `OSNAPHATCH` is
/// 0, or an arrowhead's.
pub(crate) fn edge_fills(sheet: &Sheet, list: &DisplayList) -> Vec<usize> {
    list.items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.viewport.is_none() && matches!(item.shape, Shape::Fill { .. } | Shape::Mask { .. })
        })
        .filter(|(_, item)| {
            matches!(
                drawer(sheet, item),
                Some(
                    Object::Polyline(_)
                        | Object::Solid(_)
                        | Object::Donut(_)
                        | Object::RevisionCloud(_)
                        | Object::Wipeout(_)
                )
            )
        })
        .map(|(k, _)| k)
        .collect()
}

/// The object that draws `item`: the sheet's own, or the one in its
/// insert's block, or in a block nested in it, that it names.
fn drawer<'a>(sheet: &'a Sheet, item: &Item) -> Option<&'a Object> {
    let mut object = sheet.objects.get(item.object)?;
    for k in item.part.into_iter().chain(item.nested.iter().copied()) {
        let Object::Insert(insert) = object else {
            return None;
        };
        let block = sheet.blocks.iter().find(|b| b.name == insert.block)?;
        object = block.objects.get(k)?;
    }
    Some(object)
}

/// The places in its blocks a snap's or a part's path passes through:
/// `objects[3].objects[1].to` is `[3, 1]`; empty for an object's own.
fn chain_of(path: &str) -> Vec<usize> {
    path.split('.')
        .map_while(|step| {
            step.strip_prefix("objects[")?
                .strip_suffix(']')?
                .parse()
                .ok()
        })
        .collect()
}

/// A box on the paper: its least and its greatest corner.
type Box2 = ([f64; 2], [f64; 2]);

/// The box round `parts`' curves, as they are and not as a linetype dots
/// them, a little wider so a crossing on its edge is in it; none for none.
fn box_of<'a>(parts: impl Iterator<Item = &'a Part>) -> Option<Box2> {
    let mut out: Option<Box2> = None;
    for part in parts {
        let (low, high) = part.curve.extents();
        let (min, max) = out.get_or_insert(([low[0], low[1]], [high[0], high[1]]));
        for axis in 0..2 {
            min[axis] = min[axis].min(low[axis]);
            max[axis] = max[axis].max(high[axis]);
        }
    }
    let (min, max) = out?;
    let size = [min[0], min[1], max[0], max[1]]
        .iter()
        .fold(1.0_f64, |most, v| most.max(v.abs()));
    let pad = 1e-9 * size;
    Some(([min[0] - pad, min[1] - pad], [max[0] + pad, max[1] + pad]))
}

fn overlap(a: &Box2, b: &Box2) -> bool {
    a.0[0] <= b.1[0] && b.0[0] <= a.1[0] && a.0[1] <= b.1[1] && b.0[1] <= a.1[1]
}

/// The most cells of the grid a box is put in; a box covering more is
/// looked at against every box.
const MOST_CELLS: usize = 64;

/// The most cells of the grid along a side.
const MOST_SIDE: f64 = 1_048_576.0;

/// Each pair of `boxes` that overlap, by their places, the earlier first,
/// given to `found` as it is found, in no set order. Each box is put in the cells it covers of a grid whose cells
/// are as wide and as tall as a box's middling width and height, so most
/// boxes cover a cell or two, however long and thin they are, and a pair is
/// looked at in the one cell the lower left corner of their overlap is in,
/// so it is found once. A box that covers many cells, a border's, is looked
/// at against every box instead. Only the cells a box is in are held.
fn overlapping(boxes: &[Box2], found: &mut dyn FnMut(usize, usize)) {
    let Some(all) = boxes.iter().copied().reduce(|a, b| {
        (
            [a.0[0].min(b.0[0]), a.0[1].min(b.0[1])],
            [a.1[0].max(b.1[0]), a.1[1].max(b.1[1])],
        )
    }) else {
        return;
    };
    // The cells: a box's middling size along each axis, but no smaller than
    // a share of the whole, so boxes of no width, upright lines, still
    // spread over the grid, and never more than MOST_SIDE along a side.
    let side = (boxes.len() as f64).sqrt().ceil().max(1.0);
    let size = [0, 1].map(|axis| {
        let mut spans: Vec<f64> = boxes.iter().map(|b| b.1[axis] - b.0[axis]).collect();
        let middle = spans.len() / 2;
        let (_, middling, _) = spans.select_nth_unstable_by(middle, f64::total_cmp);
        let whole = all.1[axis] - all.0[axis];
        middling
            .max(whole / (64.0 * side))
            .max(whole / MOST_SIDE)
            .max(f64::MIN_POSITIVE)
    });
    let cells =
        [0, 1].map(|axis| ((all.1[axis] - all.0[axis]) / size[axis]).ceil().max(1.0) as usize);
    let cell = |p: [f64; 2]| {
        let at =
            |axis: usize| (((p[axis] - all.0[axis]) / size[axis]) as usize).min(cells[axis] - 1);
        (at(0), at(1))
    };
    let mut grid: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    let mut large = Vec::new();
    for (k, b) in boxes.iter().enumerate() {
        let ((x0, y0), (x1, y1)) = (cell(b.0), cell(b.1));
        if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > MOST_CELLS {
            large.push(k);
            continue;
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                grid.entry((x, y)).or_default().push(k);
            }
        }
    }
    for (&here, held) in &grid {
        for (n, &a) in held.iter().enumerate() {
            for &b in &held[n + 1..] {
                let (p, q) = (&boxes[a], &boxes[b]);
                if overlap(p, q) && cell([p.0[0].max(q.0[0]), p.0[1].max(q.0[1])]) == here {
                    found(a.min(b), a.max(b));
                }
            }
        }
    }
    let is_large: BTreeSet<usize> = large.iter().copied().collect();
    for &a in &large {
        for b in 0..boxes.len() {
            // Two large boxes are looked at once, from the earlier.
            if b == a || (is_large.contains(&b) && b < a) {
                continue;
            }
            if overlap(&boxes[a], &boxes[b]) {
                found(a.min(b), a.max(b));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pairs `overlapping` finds, in order.
    fn pairs_of(boxes: &[Box2]) -> Vec<(usize, usize)> {
        let mut pairs = Vec::new();
        overlapping(boxes, &mut |a, b| pairs.push((a, b)));
        pairs.sort_unstable();
        pairs
    }

    fn square(x: f64, y: f64, side: f64) -> Box2 {
        ([x, y], [x + side, y + side])
    }

    #[test]
    fn the_grid_finds_every_overlapping_pair_once() {
        // Small boxes in a row, one overlapping the first, two long ones
        // across many cells, and one touching another only at an edge.
        let mut boxes: Vec<Box2> = (0..200)
            .map(|i| square(i as f64 * 10.0, 0.0, 4.0))
            .collect();
        boxes.push(square(3.0, 3.0, 4.0));
        boxes.push(([-1.0, 1.0], [2001.0, 2.0]));
        boxes.push(([500.0, -500.0], [501.0, 500.0]));
        boxes.push(square(14.0, 4.0, 1.0));
        let found = pairs_of(&boxes);
        let mut every = Vec::new();
        for a in 0..boxes.len() {
            for b in a + 1..boxes.len() {
                if overlap(&boxes[a], &boxes[b]) {
                    every.push((a, b));
                }
            }
        }
        assert_eq!(found, every);
        assert!(found.contains(&(0, 200)));
        assert!(found.contains(&(201, 202)));
        assert!(found.contains(&(1, 203)));
    }

    #[test]
    fn long_thin_boxes_are_found_as_short_ones_are() {
        // Long lines one above another, a few upright ones across them, and
        // a box round them all.
        let mut boxes: Vec<Box2> = (0..300)
            .map(|i| ([0.0, i as f64], [1000.0, i as f64 + 0.5]))
            .collect();
        boxes.extend((0..5).map(|i| {
            (
                [i as f64 * 200.0 + 50.0, 10.0],
                [i as f64 * 200.0 + 50.0, 20.2],
            )
        }));
        boxes.push(([-5.0, -5.0], [1005.0, 305.0]));
        let found = pairs_of(&boxes);
        let mut every = Vec::new();
        for a in 0..boxes.len() {
            for b in a + 1..boxes.len() {
                if overlap(&boxes[a], &boxes[b]) {
                    every.push((a, b));
                }
            }
        }
        assert_eq!(found, every);
        assert!(found.contains(&(10, 300)) && found.contains(&(20, 304)));
    }

    #[test]
    fn a_path_names_the_places_in_its_blocks() {
        assert_eq!(chain_of("objects[3].objects[1].to"), vec![3, 1]);
        assert_eq!(chain_of("objects[2]"), vec![2]);
        assert_eq!(chain_of("points[2]"), Vec::<usize>::new());
        assert_eq!(chain_of(""), Vec::<usize>::new());
    }
}
