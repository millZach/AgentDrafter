//! Draws a sheet's layout as it prints, as AutoCAD 2026 plotted made-up
//! layouts: paper space first, as the layout's page setup plots it, then
//! each viewport in turn, model space seen through it and its frame.
//!
//! Model space shows through a viewport moved and scaled from its view
//! centre to the viewport's centre, by its scale, and cut at its edge.
//! Text, hatch patterns and geometry scale with it. A lineweight is a width
//! on paper, so it does not; and a linetype's dashes are as long on the
//! paper as they are in paper space, as AutoCAD's PSLTSCALE 1, every
//! drawing's own, draws them: model space is drawn for a viewport with its
//! linetypes enlarged by one over its scale. Its curves are drawn as finely
//! on the paper as paper space's.

use super::clip::{clip_polygon, clip_run};
use super::{Bounds, DisplayList, Drawer, Item, Point2, Shape, TextRun};
use crate::model::{Object, Viewport};

/// Draws the layout of the sheet `drawer` draws into `list`, its curves
/// within `tolerance` on the paper.
pub(super) fn draw(drawer: &Drawer, tolerance: f64, list: &mut DisplayList) {
    let sheet = drawer.sheet;
    for (index, object) in sheet.objects.iter().enumerate() {
        if sheet.in_paper_space(index) && !matches!(object, Object::Viewport(_)) {
            drawer.sheet_object(index, object, tolerance, list);
        }
    }
    for (index, viewport) in sheet.viewports() {
        let through = Drawer {
            sheet,
            fonts: drawer.fonts,
            most_dashes: drawer.most_dashes,
            layer_zero: drawer.layer_zero.clone(),
            purpose: drawer.purpose,
            linetype_factor: drawer.linetype_factor / viewport.scale,
        };
        let fine = tolerance / viewport.scale;
        let mut model = DisplayList::default();
        for (i, object) in sheet.objects.iter().enumerate() {
            if !sheet.in_paper_space(i) {
                through.sheet_object(i, object, fine, &mut model);
            }
        }
        list.add_view(index, viewport, model);
        drawer.sheet_object(index, &sheet.objects[index], tolerance, list);
    }
}

impl DisplayList {
    /// Adds what model space draws, `model`, as it shows through the
    /// sheet's viewport `viewport`, the sheet's object `index`: moved and
    /// scaled onto the paper and cut at the viewport's edge, each shape and
    /// line of text marked with the viewport.
    fn add_view(&mut self, index: usize, viewport: &Viewport, model: DisplayList) {
        let [min, max] = viewport.corners();
        let on = |p: Point2| viewport.to_paper(p);
        let window = Bounds { min, max };
        for item in model.items {
            let shape = match item.shape {
                Shape::Stroke { lineweight, runs } => {
                    let runs: Vec<Vec<Point2>> = runs
                        .into_iter()
                        .flat_map(|run| {
                            clip_run(&run.into_iter().map(on).collect::<Vec<_>>(), min, max)
                        })
                        .collect();
                    if runs.is_empty() {
                        continue;
                    }
                    Shape::Stroke { lineweight, runs }
                }
                Shape::Fill { rule, contours } => {
                    let contours: Vec<Vec<Point2>> = contours
                        .into_iter()
                        .map(|c| clip_polygon(&c.into_iter().map(on).collect::<Vec<_>>(), min, max))
                        .filter(|c| !c.is_empty())
                        .collect();
                    if contours.is_empty() {
                        continue;
                    }
                    Shape::Fill { rule, contours }
                }
            };
            self.items.push(Item {
                viewport: Some(index),
                shape,
                ..item
            });
        }
        let scale = viewport.scale;
        for run in model.text {
            let run = TextRun {
                origin: on(run.origin),
                across: [run.across[0] * scale, run.across[1] * scale],
                up: [run.up[0] * scale, run.up[1] * scale],
                viewport: Some(index),
                ..run
            };
            if shows(&run, &window) {
                self.text.push(run);
            }
        }
        let placed = |b: Bounds| -> Option<Bounds> {
            let b = Bounds {
                min: on(b.min),
                max: on(b.max),
            };
            let min = [b.min[0].max(min[0]), b.min[1].max(min[1])];
            let max = [b.max[0].min(max[0]), b.max[1].min(max[1])];
            (min[0] <= max[0] && min[1] <= max[1]).then_some(Bounds { min, max })
        };
        if let Some(reach) = model.reach.and_then(placed) {
            self.take_in([reach.min, reach.max]);
        }
        for (object, boundary) in model.boundaries {
            if let Some(boundary) = placed(boundary) {
                self.boundaries.push((object, boundary));
            }
        }
        for (object, loops) in model.hatch_loops {
            let loops: Vec<Vec<Point2>> = loops
                .into_iter()
                .map(|l| clip_polygon(&l.into_iter().map(on).collect::<Vec<_>>(), min, max))
                .filter(|l| !l.is_empty())
                .collect();
            if !loops.is_empty() {
                self.hatch_loops.push((object, loops));
            }
        }
        for (object, band) in model.wide_arcs {
            let band = clip_polygon(&band.into_iter().map(on).collect::<Vec<_>>(), min, max);
            if !band.is_empty() {
                self.wide_arcs.push((object, band));
            }
        }
        for cut in model.hatches_cut_short {
            if !self
                .hatches_cut_short
                .iter()
                .any(|c| c.object == cut.object && c.block == cut.block)
            {
                self.hatches_cut_short.push(cut);
            }
        }
    }
}

/// Whether a line of text shows in `window`: what its characters draw
/// reaches into it, or, when they draw nothing, its start is in it.
fn shows(run: &TextRun, window: &Bounds) -> bool {
    let at = |[u, v]: Point2| {
        [
            run.origin[0] + u * run.across[0] + v * run.up[0],
            run.origin[1] + u * run.across[1] + v * run.up[1],
        ]
    };
    let inside = |p: Point2| {
        p[0] >= window.min[0]
            && p[0] <= window.max[0]
            && p[1] >= window.min[1]
            && p[1] <= window.max[1]
    };
    match run.drawn {
        None => inside(run.origin),
        Some([lo, hi]) => {
            let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]].map(at);
            let Some(b) = Bounds::of(corners.into_iter()) else {
                return false;
            };
            b.min[0] <= window.max[0]
                && b.max[0] >= window.min[0]
                && b.min[1] <= window.max[1]
                && b.max[1] >= window.min[1]
        }
    }
}
