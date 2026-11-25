//! Draws a sheet's layout as it prints, as AutoCAD 2026 plotted made-up
//! layouts: paper space first, as the layout's page setup plots it, then
//! each viewport in turn, model space seen through it and its frame.
//!
//! Model space shows through a viewport moved and scaled from its view
//! centre to the viewport's centre, by its scale, and cut at its edge: its
//! paths are cut there, so that a line whose centre runs outside draws
//! nothing, and each item keeps the window as its clip, so that an output
//! cuts its ink there too, a pen's width and round ends among it, as
//! AutoCAD 2026 plotted a made-up probe of lines 2.11 mm wide.
//! Text, hatch patterns and geometry scale with it. A lineweight is a width
//! on paper, so it does not; and a linetype's dashes are as long on the
//! paper as they are in paper space, as AutoCAD's PSLTSCALE 1, every
//! drawing's own, draws them: model space is drawn for a viewport with its
//! linetypes enlarged by one over its scale. Its curves are drawn as finely
//! on the paper as paper space's.
//!
//! A viewport clipped to a boundary, as AutoCAD's VPCLIP clips one, cuts
//! its lines at that boundary, as its chords on the paper, and leaves out
//! a line along one of its straight edges that runs across or up the
//! paper, as AutoCAD 2026 left one out, but not one along a slanting edge
//! or an arc, which it drew; its fills are cut at the boundary's box,
//! and each item names the boundary, so that an output cuts its ink there,
//! as AutoCAD 2026 plotted made-up clipped viewports. A layer frozen in a
//! viewport, as AutoCAD's VPLAYER freezes one, draws nothing through it.

use super::clip::{box_meets_outline, clip_polygon, clip_run, clip_run_to_outline, ALONG_AN_EDGE};
use super::{curves, polyline, Bounds, DisplayList, Drawer, Item, Point2, Shape, TextRun};
use crate::model::{Clip, Object, Viewport};

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
        let vp_annoscale = sheet.viewport_annotation_scale(viewport).factor();
        let [model_min, model_max] = viewport.view();
        let through = Drawer {
            sheet,
            fonts: drawer.fonts,
            most_dashes: drawer.most_dashes,
            layer_zero: drawer.layer_zero.clone(),
            purpose: drawer.purpose,
            linetype_factor: drawer.linetype_factor / viewport.scale,
            frozen_here: &viewport.frozen_layers,
            annotation_scale: vp_annoscale,
            anno_all_visible: sheet.anno_all_visible.unwrap_or(false),
            view_bounds: Some(Bounds {
                min: model_min,
                max: model_max,
            }),
            view: viewport.view_direction.map_or(
                crate::geometry::solid3d::OrthoView::TOP,
                crate::geometry::solid3d::OrthoView::from_direction,
            ),
        };
        let fine = tolerance / viewport.scale;
        let mut model = DisplayList::default();
        for (i, object) in sheet.objects.iter().enumerate() {
            if sheet.in_model_space(i) {
                through.sheet_object(i, object, fine, &mut model);
            }
        }
        let outline = viewport.clip.as_ref().map(|clip| outline(clip, tolerance));
        list.add_view(index, viewport, outline, fine, model);
        drawer.sheet_object(index, &sheet.objects[index], tolerance, list);
    }
}

/// The boundary `clip` on the paper as a closed outline, its arcs as
/// chords within `tolerance` of them, as its frame draws them, with a mark
/// for each edge, from point `i` to the next, that is one of the
/// boundary's own straight segments running across or up the paper.
pub(super) fn outline(clip: &Clip, tolerance: f64) -> (Vec<Point2>, Vec<bool>) {
    let (mut out, mut square): (Vec<Point2>, Vec<bool>) = match clip {
        Clip::Circle { center, radius } => {
            let out = curves::circle([center.x, center.y], *radius, tolerance);
            let square = vec![false; out.len()];
            (out, square)
        }
        Clip::Polyline { points, .. } => {
            let mut out = Vec::new();
            let mut square = Vec::new();
            for (i, a) in points.iter().enumerate() {
                let b = &points[(i + 1) % points.len()];
                let segment = polyline::segment([a.x, a.y], [b.x, b.y], clip.bulge(i), tolerance);
                let straight = segment.len() == 2 && (a.x == b.x || a.y == b.y);
                out.extend(&segment[..segment.len() - 1]);
                square.extend(std::iter::repeat_n(straight, segment.len() - 1));
            }
            (out, square)
        }
    };
    // Closed: its last point is not its first again.
    if out.len() > 1 && out.first() == out.last() {
        out.pop();
        square.pop();
    }
    (out, square)
}

impl DisplayList {
    /// Adds what model space draws, `model`, as it shows through the
    /// sheet's viewport `viewport`, the sheet's object `index`: moved and
    /// scaled onto the paper and cut at the viewport's edge, or at
    /// `outline`, its boundary on the paper when it is clipped to one, as
    /// [`outline`] gives it, each shape and line of text marked with the
    /// viewport. A line within `fine` of the boundary, model space's
    /// drawing tolerance, lies on it.
    fn add_view(
        &mut self,
        index: usize,
        viewport: &Viewport,
        outline: Option<(Vec<Point2>, Vec<bool>)>,
        fine: f64,
        model: DisplayList,
    ) {
        let [min, max] = viewport.corners();
        let window = Bounds { min, max };
        // The boundary in model space, where lines are cut at it.
        let model_outline: Option<(Vec<Point2>, Vec<bool>)> = match outline.as_ref() {
            Some((o, square)) => Some((
                o.iter().map(|&p| viewport.to_model(p)).collect(),
                square.clone(),
            )),
            None if viewport.rotation != 0.0 => Some((
                viewport
                    .frame()
                    .into_iter()
                    .map(|p| viewport.to_model([p.x, p.y]))
                    .collect(),
                vec![false; 4],
            )),
            None => None,
        };
        let boundary = outline.map(|(o, _)| {
            self.clip_boundaries.push(o);
            self.clip_boundaries.len() - 1
        });
        // Model space is cut at the part of it the viewport shows, then
        // moved onto the paper, so that what lies far off it never
        // overflows on the way.
        let [model_min, model_max] = viewport.view();
        // A line along the viewport's edge, as close on the paper at any
        // scale.
        let along = ALONG_AN_EDGE / viewport.scale;
        let on = |p: Point2| viewport.to_paper(p);
        let placed = |points: Vec<Point2>| points.into_iter().map(on).collect::<Vec<_>>();
        for item in model.items {
            let shape = match item.shape {
                Shape::Stroke { lineweight, runs } => {
                    let runs: Vec<Vec<Point2>> = runs
                        .into_iter()
                        .flat_map(|run| clip_run(&run, model_min, model_max, along))
                        .flat_map(|run| match &model_outline {
                            Some((o, square)) => clip_run_to_outline(&run, o, square, along, fine),
                            None => vec![run],
                        })
                        .map(placed)
                        .collect();
                    if runs.is_empty() {
                        continue;
                    }
                    Shape::Stroke { lineweight, runs }
                }
                Shape::Fill { rule, contours } => {
                    let contours: Vec<Vec<Point2>> = contours
                        .into_iter()
                        .map(|c| clip_polygon(&c, model_min, model_max))
                        .filter(|c| !c.is_empty())
                        .map(placed)
                        .collect();
                    if contours.is_empty() {
                        continue;
                    }
                    Shape::Fill { rule, contours }
                }
                Shape::Mask { contours } => {
                    let contours: Vec<Vec<Point2>> = contours
                        .into_iter()
                        .map(|c| clip_polygon(&c, model_min, model_max))
                        .filter(|c| !c.is_empty())
                        .map(placed)
                        .collect();
                    if contours.is_empty() {
                        continue;
                    }
                    Shape::Mask { contours }
                }
                Shape::Image {
                    image,
                    origin,
                    u_axis,
                    v_axis,
                    clip,
                    clip_inverted,
                    brightness,
                    contrast,
                    contrast_neutral,
                    fade,
                    monochrome,
                    transparent,
                } => {
                    let p0 = on(origin);
                    let pu = on([origin[0] + u_axis[0], origin[1] + u_axis[1]]);
                    let pv = on([origin[0] + v_axis[0], origin[1] + v_axis[1]]);
                    Shape::Image {
                        image,
                        origin: p0,
                        u_axis: [pu[0] - p0[0], pu[1] - p0[1]],
                        v_axis: [pv[0] - p0[0], pv[1] - p0[1]],
                        clip: clip.map(placed),
                        clip_inverted,
                        brightness,
                        contrast,
                        contrast_neutral,
                        fade,
                        monochrome,
                        transparent,
                    }
                }
            };
            self.items.push(Item {
                viewport: Some(index),
                clip: Some(window),
                clip_boundary: boundary,
                shape,
                ..item
            });
        }
        let scale = viewport.scale;
        let (v_sin, v_cos) = if viewport.rotation == 0.0 {
            (0.0, 1.0)
        } else {
            crate::description::sin_cos_degrees(viewport.rotation)
        };
        let rot_vec = |v: [f64; 2]| -> [f64; 2] {
            if viewport.rotation == 0.0 {
                [v[0] * scale, v[1] * scale]
            } else {
                [
                    (v[0] * v_cos - v[1] * v_sin) * scale,
                    (v[0] * v_sin + v[1] * v_cos) * scale,
                ]
            }
        };
        for run in model.text {
            let run = TextRun {
                origin: on(run.origin),
                across: rot_vec(run.across),
                up: rot_vec(run.up),
                viewport: Some(index),
                ..run
            };
            let finite = [run.origin, run.across, run.up]
                .iter()
                .flatten()
                .all(|v| v.is_finite());
            let seen = match boundary {
                Some(b) => shows_in_outline(&run, &self.clip_boundaries[b]),
                None => shows(&run, &window),
            };
            if finite && seen {
                self.text.push(run);
            }
        }
        // A box is cut to the part of model space shown, then moved.
        let within = |b: Bounds| -> Option<Bounds> {
            let lo = [b.min[0].max(model_min[0]), b.min[1].max(model_min[1])];
            let hi = [b.max[0].min(model_max[0]), b.max[1].min(model_max[1])];
            (lo[0] <= hi[0] && lo[1] <= hi[1]).then(|| {
                if viewport.rotation == 0.0 {
                    Bounds {
                        min: on(lo),
                        max: on(hi),
                    }
                } else {
                    Bounds::of([on(lo), on([hi[0], lo[1]]), on(hi), on([lo[0], hi[1]])].into_iter())
                        .expect("four corners")
                }
            })
        };
        if let Some(reach) = model.reach.and_then(within) {
            self.take_in([reach.min, reach.max]);
        }
        for (object, boundary) in model.boundaries {
            if let Some(boundary) = within(boundary) {
                self.boundaries.push((object, boundary));
            }
        }
        for (object, loops) in model.hatch_loops {
            let loops: Vec<Vec<Point2>> = loops
                .into_iter()
                .map(|l| clip_polygon(&l, model_min, model_max))
                .filter(|l| !l.is_empty())
                .map(placed)
                .collect();
            if !loops.is_empty() {
                self.hatch_loops.push((object, loops));
            }
        }
        for (object, band) in model.wide_arcs {
            let band = clip_polygon(&band, model_min, model_max);
            if !band.is_empty() {
                self.wide_arcs.push((object, placed(band)));
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

/// Whether a line of text shows in the closed outline `outline` on the
/// paper: what its characters draw reaches into it, or, when they draw
/// nothing, its start is in it.
fn shows_in_outline(run: &TextRun, outline: &[Point2]) -> bool {
    let at = |[u, v]: Point2| {
        [
            run.origin[0] + u * run.across[0] + v * run.up[0],
            run.origin[1] + u * run.across[1] + v * run.up[1],
        ]
    };
    match run.drawn {
        None => super::clip::inside_outline(run.origin, outline),
        Some([lo, hi]) => {
            let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]].map(at);
            let Some(b) = Bounds::of(corners.into_iter()) else {
                return false;
            };
            box_meets_outline(b.min, b.max, outline)
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
