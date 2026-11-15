//! A sheet drawn in a paper-space layout, and the viewports that look from
//! it onto model space.
//!
//! A sheet with a layout is two drawings in one DWG, as in AutoCAD. Model
//! space holds the drawing at full size; paper space holds the sheet's
//! border and title block at the paper's size, and the viewports, each a
//! window on the paper that shows model space at a scale, centred on a
//! model point. A sheet prints its layout: paper space as it is drawn, and
//! model space through each viewport, cut at its edge.
//!
//! Both spaces are in the sheet's declared unit, and every object of the
//! sheet says which it is in: model space unless it says paper. A viewport
//! is always in paper space.
//!
//! A sheet may be drawn in several layouts of one DWG, which share its
//! model space, as AutoCAD's layout tabs do. Each has its own name, paper,
//! paper-space objects and viewports, and prints on its own paper.

use serde::{Deserialize, Serialize};

use super::{is_false, layer_zero, Paper, Point3, Properties};

/// A paper-space layout a sheet is drawn in, as AutoCAD's layout tab holds
/// it: its name, its paper, and which of the sheet's objects are in its
/// paper space. Its page setup plots its paper whole at full size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// The layout's name, as AutoCAD's tab shows it, such as `Panel`.
    pub name: String,
    /// The paper its page setup plots, a standard size by name. The
    /// sheet's own [`Paper`] is the paper of the layout it draws.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    /// For each of the sheet's objects, in order, whether it is in this
    /// layout's paper space. Every viewport is in one layout's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paper_space: Vec<bool>,
}

impl Layout {
    /// A layout named `name` with every one of `objects` objects in model
    /// space but its viewports, which are in its paper space.
    pub fn new(name: impl Into<String>, objects: &[super::Object]) -> Self {
        Self {
            name: name.into(),
            paper: None,
            paper_space: objects
                .iter()
                .map(|o| matches!(o, super::Object::Viewport(_)))
                .collect(),
        }
    }

    /// Whether the sheet's object at `index` is in this layout's paper
    /// space.
    pub fn holds(&self, index: usize) -> bool {
        self.paper_space.get(index).copied().unwrap_or(false)
    }
}

/// The boundary a viewport is clipped to instead of its rectangle, on the
/// paper, as AutoCAD's VPCLIP and MVIEW's Object and Polygonal options
/// clip one: model space shows inside it alone, and it is the viewport's
/// frame. The viewport's window is the box round it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Clip {
    /// A closed boundary through `points`, each segment straight or an arc
    /// by its bulge, as a closed polyline is: AutoCAD's polygonal clip.
    Polyline {
        points: Vec<Point3>,
        /// For each point, the bulge of the segment that starts there, as
        /// a polyline's; empty when every segment is straight.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bulges: Vec<f64>,
    },
    /// A circle.
    Circle { center: Point3, radius: f64 },
}

impl Clip {
    /// The bulge of the segment that starts at point `i`.
    pub fn bulge(&self, i: usize) -> f64 {
        match self {
            Clip::Polyline { bulges, .. } => bulges.get(i).copied().unwrap_or(0.0),
            Clip::Circle { .. } => 0.0,
        }
    }

    /// The box round the boundary on the paper: its lower left and upper
    /// right corners, an arc's farthest reach among it.
    pub fn bounds(&self) -> [[f64; 2]; 2] {
        match self {
            Clip::Circle { center, radius } => [
                [center.x - radius, center.y - radius],
                [center.x + radius, center.y + radius],
            ],
            Clip::Polyline { points, .. } => {
                let mut low = [f64::INFINITY; 2];
                let mut high = [f64::NEG_INFINITY; 2];
                let mut take = |p: [f64; 2]| {
                    for axis in 0..2 {
                        low[axis] = low[axis].min(p[axis]);
                        high[axis] = high[axis].max(p[axis]);
                    }
                };
                for (i, a) in points.iter().enumerate() {
                    let b = &points[(i + 1) % points.len()];
                    take([a.x, a.y]);
                    for p in arc_reach([a.x, a.y], [b.x, b.y], self.bulge(i)) {
                        take(p);
                    }
                }
                [low, high]
            }
        }
    }

    /// The boundary moved by `by` on the paper.
    pub fn moved(&self, by: [f64; 2]) -> Clip {
        let shift = |p: &Point3| Point3::new(p.x + by[0], p.y + by[1], p.z);
        match self {
            Clip::Polyline { points, bulges } => Clip::Polyline {
                points: points.iter().map(shift).collect(),
                bulges: bulges.clone(),
            },
            Clip::Circle { center, radius } => Clip::Circle {
                center: shift(center),
                radius: *radius,
            },
        }
    }
}

/// The points at 0°, 90°, 180° and 270° round its centre that the arc
/// from `a` to `b` of `bulge` passes through, which reach past its ends;
/// none for a straight segment.
fn arc_reach(a: [f64; 2], b: [f64; 2], bulge: f64) -> Vec<[f64; 2]> {
    let chord = [b[0] - a[0], b[1] - a[1]];
    let length = chord[0].hypot(chord[1]);
    if bulge == 0.0 || !bulge.is_finite() || length == 0.0 {
        return Vec::new();
    }
    let half = length / 2.0;
    let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let left = [-chord[1] / length, chord[0] / length];
    let off = half * (1.0 - bulge * bulge) / (2.0 * bulge);
    let center = [middle[0] + left[0] * off, middle[1] + left[1] * off];
    let radius = half * (1.0 + bulge * bulge) / (2.0 * bulge.abs());
    let start = (a[1] - center[1]).atan2(a[0] - center[0]);
    let sweep = 4.0 * bulge.atan();
    let tau = std::f64::consts::TAU;
    (0..4)
        .filter_map(|quarter| {
            let angle = f64::from(quarter) * tau / 4.0;
            // How far round from the start, the way the arc turns.
            let along = if sweep > 0.0 {
                (angle - start).rem_euclid(tau)
            } else {
                (start - angle).rem_euclid(tau)
            };
            (along <= sweep.abs()).then(|| {
                // A quarter point exactly, so that a box reads back whole.
                match quarter {
                    0 => [center[0] + radius, center[1]],
                    1 => [center[0], center[1] + radius],
                    2 => [center[0] - radius, center[1]],
                    _ => [center[0], center[1] - radius],
                }
            })
        })
        .collect()
}

/// A rectangular window on the paper that shows model space, at a scale,
/// centred on a model point, as AutoCAD's MVIEW makes one. It sits in
/// paper space, and its frame draws there with its layer's pen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// The middle of the window, on the paper. Its `z` is 0.
    pub center: Point3,
    /// How wide the window is on the paper, more than 0.
    pub width: f64,
    /// How tall the window is on the paper, more than 0.
    pub height: f64,
    /// The point of model space at the middle of the window. Its `z` is 0.
    pub view_center: Point3,
    /// How long a length of model space draws on the paper, per unit:
    /// 0.25 for 1:4. More than 0.
    pub scale: f64,
    /// Whether its view is locked, as AutoCAD's display lock keeps it from
    /// being zoomed or panned. It changes nothing it draws.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// The boundary it is clipped to, when it is not rectangular: its
    /// window is then the box round it, as AutoCAD keeps a clipped
    /// viewport's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Clip>,
    /// The layers frozen in it alone, as AutoCAD's VPLAYER freezes them:
    /// their objects neither draw nor plot through it, and do through the
    /// sheet's other viewports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frozen_layers: Vec<String>,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl Viewport {
    /// The window's lower left and upper right corners on the paper.
    pub fn corners(&self) -> [[f64; 2]; 2] {
        let (w, h) = (self.width / 2.0, self.height / 2.0);
        [
            [self.center.x - w, self.center.y - h],
            [self.center.x + w, self.center.y + h],
        ]
    }

    /// The window's frame on the paper, from its lower left corner
    /// anticlockwise: lower left, lower right, upper right, upper left.
    pub fn frame(&self) -> [Point3; 4] {
        let [a, b] = self.corners();
        let z = self.center.z;
        [
            Point3::new(a[0], a[1], z),
            Point3::new(b[0], a[1], z),
            Point3::new(b[0], b[1], z),
            Point3::new(a[0], b[1], z),
        ]
    }

    /// The part of model space the window shows: its lower left and upper
    /// right corners in model space.
    pub fn view(&self) -> [[f64; 2]; 2] {
        let (w, h) = (
            self.width / 2.0 / self.scale,
            self.height / 2.0 / self.scale,
        );
        [
            [self.view_center.x - w, self.view_center.y - h],
            [self.view_center.x + w, self.view_center.y + h],
        ]
    }

    /// Where the model point `p` shows on the paper.
    pub fn to_paper(&self, p: [f64; 2]) -> [f64; 2] {
        [
            self.center.x + (p[0] - self.view_center.x) * self.scale,
            self.center.y + (p[1] - self.view_center.y) * self.scale,
        ]
    }

    /// The model point that shows at the paper point `p`.
    pub fn to_model(&self, p: [f64; 2]) -> [f64; 2] {
        [
            self.view_center.x + (p[0] - self.center.x) / self.scale,
            self.view_center.y + (p[1] - self.center.y) / self.scale,
        ]
    }

    /// The scale as a ratio, paper to model, such as `1:4` or `2:1`: the
    /// ratio with the smallest model side, up to [`SIMPLEST_RATIO`], that is
    /// the scale; otherwise the scale itself.
    pub fn ratio(&self) -> String {
        let s = self.scale;
        (1..=SIMPLEST_RATIO)
            .find_map(|q| {
                let p = (s * f64::from(q)).round();
                (p >= 1.0 && p / f64::from(q) == s).then(|| format!("{p}:{q}"))
            })
            .unwrap_or_else(|| s.to_string())
    }

    /// What is wrong with the viewport's scale for a DWG to hold it and the
    /// pipeline to draw it, if anything: the scale, one over it, the model
    /// height and width the window shows, and the corners of that part of
    /// model space are each a finite number, and none of the first four is
    /// 0 or too small to be held whole.
    pub fn scale_problem(&self) -> Option<String> {
        let scale = self.scale;
        let [low, high] = self.view();
        let values = [
            ("the scale", scale),
            ("one over the scale", 1.0 / scale),
            (
                "the model height it shows, its height over its scale",
                self.height / scale,
            ),
            (
                "the model width it shows, its width over its scale",
                self.width / scale,
            ),
        ];
        if let Some((what, value)) = values.iter().find(|(_, v)| !v.is_normal() || *v < 0.0) {
            return Some(format!(
                "{what} is {value:e}, which a DWG cannot hold; a viewport's scale, one over it, and \
                 its height and width over it are each a finite number more than 0, not too \
                 small to hold whole"
            ));
        }
        if !low.iter().chain(&high).all(|v| v.is_finite()) {
            return Some(
                "the part of model space it shows, round its view centre, reaches past the \
                 largest number a DWG holds"
                    .to_owned(),
            );
        }
        None
    }

    /// Whether the layer named `layer` is frozen in it, as AutoCAD matches
    /// a layer's name, whatever its case.
    pub fn freezes(&self, layer: &str) -> bool {
        self.frozen_layers
            .iter()
            .any(|l| l.eq_ignore_ascii_case(layer))
    }

    /// The viewport clipped to `clip`, as AutoCAD's VPCLIP clips one: its
    /// window becomes the box round the boundary, and its view centre the
    /// model point there, so that model space stays where it showed.
    pub fn clipped_to(&self, clip: Clip) -> Viewport {
        let [low, high] = clip.bounds();
        let center = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0];
        let view = self.to_model(center);
        Viewport {
            center: Point3::new(center[0], center[1], 0.0),
            width: high[0] - low[0],
            height: high[1] - low[1],
            view_center: Point3::new(view[0], view[1], 0.0),
            clip: Some(clip),
            ..self.clone()
        }
    }

    /// The model height the window shows, as AutoCAD's DWG keeps it: its
    /// height on the paper over its scale.
    pub fn view_height(&self) -> f64 {
        self.height / self.scale
    }

    /// The scale a viewport `height` tall shows when it shows
    /// `view_height` of model space, as AutoCAD keeps the two: the ratio
    /// `p:q` with the smallest `q`, up to [`SIMPLEST_RATIO`], that gives
    /// back exactly that view height, so that 1:4 reads back as 0.25, 3:8
    /// as 0.375 and 3:1 as 3; otherwise the height over the view height.
    pub fn scale_of(height: f64, view_height: f64) -> f64 {
        let ratio = height / view_height;
        if !(ratio.is_finite() && ratio > 0.0) {
            return ratio;
        }
        (1..=SIMPLEST_RATIO)
            .find_map(|q| {
                let q = f64::from(q);
                let p = (ratio * q).round();
                let s = p / q;
                (p >= 1.0 && height / s == view_height).then_some(s)
            })
            .unwrap_or(ratio)
    }
}

/// The largest model side `q` of a ratio `p:q` that a viewport's scale is
/// read back as: 1:10000 at the most.
pub const SIMPLEST_RATIO: u32 = 10_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scale_reads_back_as_written() {
        for (height, scale) in [
            (140.0, 0.25),
            (140.0, 1.0 / 3.0),
            (257.0, 0.2),
            (3.7, 3.0),
            (10.0, 1.0 / 48.0),
            (99.9, 0.375),
        ] {
            let view_height = height / scale;
            assert_eq!(
                Viewport::scale_of(height, view_height).to_bits(),
                scale.to_bits(),
                "{height} at {scale}"
            );
        }
    }

    #[test]
    fn a_model_point_shows_where_the_scale_puts_it() {
        let viewport = Viewport {
            center: Point3::new(120.0, 90.0, 0.0),
            width: 200.0,
            height: 140.0,
            view_center: Point3::new(400.0, 300.0, 0.0),
            scale: 0.25,
            locked: false,
            clip: None,
            frozen_layers: Vec::new(),
            layer: "0".into(),
            properties: Properties::default(),
        };
        assert_eq!(viewport.to_paper([400.0, 300.0]), [120.0, 90.0]);
        assert_eq!(viewport.to_paper([0.0, 20.0]), [20.0, 20.0]);
        assert_eq!(viewport.to_model([20.0, 20.0]), [0.0, 20.0]);
        assert_eq!(viewport.view(), [[0.0, 20.0], [800.0, 580.0]]);
    }

    #[test]
    fn a_clipped_viewport_keeps_model_space_where_it_showed() {
        // As AutoCAD 2026's VPCLIP clipped a made-up 1:4 viewport to a
        // polyline whose half circle reaches 280 across.
        let viewport = Viewport {
            center: Point3::new(120.0, 150.0, 0.0),
            width: 200.0,
            height: 200.0,
            view_center: Point3::new(400.0, 300.0, 0.0),
            scale: 0.25,
            locked: false,
            clip: None,
            frozen_layers: Vec::new(),
            layer: "0".into(),
            properties: Properties::default(),
        };
        let clip = Clip::Polyline {
            points: [[30.0, 60.0], [190.0, 60.0], [190.0, 240.0], [30.0, 240.0]]
                .map(|[x, y]| Point3::new(x, y, 0.0))
                .to_vec(),
            bulges: vec![0.0, 1.0, 0.0, 0.0],
        };
        assert_eq!(clip.bounds(), [[30.0, 60.0], [280.0, 240.0]]);
        let clipped = viewport.clipped_to(clip);
        assert_eq!(clipped.center, Point3::new(155.0, 150.0, 0.0));
        assert_eq!((clipped.width, clipped.height), (250.0, 180.0));
        assert_eq!(clipped.view_center, Point3::new(540.0, 300.0, 0.0));
        assert_eq!(clipped.view_height(), 720.0);
        let circle = Clip::Circle {
            center: Point3::new(330.0, 250.0, 0.0),
            radius: 30.0,
        };
        assert_eq!(circle.bounds(), [[300.0, 220.0], [360.0, 280.0]]);
    }
}
