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

use serde::{Deserialize, Serialize};

use super::{is_false, layer_zero, Point3, Properties};

/// The paper-space layout a sheet is drawn in, as AutoCAD's layout tab
/// holds it: its name, and which of the sheet's objects are in paper
/// space. Its paper is the sheet's [`super::Paper`], and its page setup
/// plots that paper whole at full size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// The layout's name, as AutoCAD's tab shows it, such as `Panel`.
    pub name: String,
    /// For each of the sheet's objects, in order, whether it is in paper
    /// space. Every viewport is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paper_space: Vec<bool>,
}

impl Layout {
    /// A layout named `name` with every one of `objects` objects in model
    /// space but its viewports.
    pub fn new(name: impl Into<String>, objects: &[super::Object]) -> Self {
        Self {
            name: name.into(),
            paper_space: objects
                .iter()
                .map(|o| matches!(o, super::Object::Viewport(_)))
                .collect(),
        }
    }
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
            layer: "0".into(),
            properties: Properties::default(),
        };
        assert_eq!(viewport.to_paper([400.0, 300.0]), [120.0, 90.0]);
        assert_eq!(viewport.to_paper([0.0, 20.0]), [20.0, 20.0]);
        assert_eq!(viewport.to_model([20.0, 20.0]), [0.0, 20.0]);
        assert_eq!(viewport.view(), [[0.0, 20.0], [800.0, 580.0]]);
    }
}
