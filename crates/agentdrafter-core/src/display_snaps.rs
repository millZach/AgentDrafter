//! The object snaps a display list gives with `snaps`, so the viewer can
//! find the snap nearest the cursor with no round trip: each object's own
//! (endpoint, midpoint, centre, quadrant, insertion and node), and each
//! intersection between two objects, worked out by the same snap code as
//! `query snap` (`geometry/snap.rs`), so a snap here and the same object,
//! snap and `which` asked of `query snap` give the same point.
//!
//! Only objects drawn on the page itself have snaps here: every object of a
//! sheet with no layout, and the paper space of the layout drawn. Model
//! space seen through a viewport, and an object on a layer that is off or
//! frozen, which draw nothing of their own on the page, have none.
//!
//! An intersection is listed once, under the earlier of its two objects,
//! `with` the later, named as `query snap --with` names it: its place in
//! the order the two cross along the first, when they cross more than once.
//! Only pairs whose drawings overlap are looked at.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::draw::{object_bounds, Bounds, DisplayList};
use crate::geometry::snap::{intersections, own_points};
use crate::geometry::{Part, Reader, SnapKind};
use crate::model::{Point3, Sheet};
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
/// with fonts `fonts`; the list's chords stray at most `tolerance` from its
/// curves.
pub(crate) fn snaps_of(
    sheet: &Sheet,
    fonts: &SheetFonts,
    list: &DisplayList,
    tolerance: f64,
) -> Vec<DisplaySnap> {
    let reader = Reader { sheet, fonts };
    // The objects the page draws itself, in their order.
    let drawn: BTreeSet<usize> = list
        .items
        .iter()
        .filter(|item| item.viewport.is_none())
        .map(|item| item.object)
        .collect();
    let mut snaps = Vec::new();
    for &i in &drawn {
        let Some(object) = sheet.objects.get(i) else {
            continue;
        };
        for (_, point) in own_points(&reader, object, None) {
            snaps.push(DisplaySnap {
                object: i,
                snap: point.snap,
                which: point.which,
                with: None,
                at: point.at,
            });
        }
    }
    // Each object's curves and the box round its drawing, once.
    let shapes: Vec<(usize, Vec<Part>, Bounds)> = drawn
        .iter()
        .filter_map(|&i| {
            let parts = reader.parts(sheet.objects.get(i)?);
            if parts.is_empty() {
                return None;
            }
            let bounds = object_bounds(list, i)?;
            // A chord lies inside its curve by up to the tolerance: the box
            // is widened so a crossing on the curve is not missed.
            let pad = 2.0 * tolerance + 1e-9 * bounds.width().max(bounds.height());
            Some((i, parts, widened(bounds, pad)))
        })
        .collect();
    for (k, (i, parts, bounds)) in shapes.iter().enumerate() {
        for (j, others, other_bounds) in &shapes[k + 1..] {
            if !overlap(bounds, other_bounds) {
                continue;
            }
            for point in intersections(parts, others, false) {
                snaps.push(DisplaySnap {
                    object: *i,
                    snap: SnapKind::Intersection,
                    which: point.which,
                    with: Some(*j),
                    at: point.at,
                });
            }
        }
    }
    snaps
}

fn widened(b: Bounds, pad: f64) -> Bounds {
    Bounds {
        min: [b.min[0] - pad, b.min[1] - pad],
        max: [b.max[0] + pad, b.max[1] + pad],
    }
}

fn overlap(a: &Bounds, b: &Bounds) -> bool {
    a.min[0] <= b.max[0] && b.min[0] <= a.max[0] && a.min[1] <= b.max[1] && b.min[1] <= a.max[1]
}
