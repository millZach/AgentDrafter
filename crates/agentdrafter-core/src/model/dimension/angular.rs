//! An angular dimension, laid out as AutoCAD 2026 laid out its own
//! DIMANGULAR of made-up lines, arcs, circles and points in its `acad` and
//! `acadiso` templates, sweeping the angle, the radius and the text:
//!
//! - It measures the sector at its vertex that its location is in: between
//!   two lines, of the four their rays make, or at a vertex between two
//!   points, of the two. Its arc runs round the vertex through the
//!   location, anticlockwise from the sector's first ray to its second.
//! - An extension line runs along each ray from the line or the point
//!   there, any of a line behind the vertex taken as at the vertex, from
//!   the end of it nearer the arc, the style's offset towards
//!   the arc, to the style's extension beyond it; none where the arc meets
//!   the line itself.
//! - Text and arrowheads fit as a linear dimension's do, by the angles they
//!   need on the arc: a length `c` along the arc of radius `r` takes the
//!   angle of the chord `c` long, `2 asin(c / 2r)`, or half a turn when the
//!   circle is not that wide across. Text above the arc, as
//!   ISO-25 has it, needs its length and a gap at each end; centred text,
//!   as Standard has it, its box's width and height across the arc's
//!   tangent at the sector's middle, each with a gap, and an arrowhead's
//!   length more, or three when the arrowheads are inside too. Each
//!   arrowhead needs its length.
//! - An arrowhead's point is on the ray, its body on the arc, or, longer
//!   than the arc's circle is across, through the vertex, its full length
//!   still. Arrowheads outside the sector have tails, arcs as long as themselves; the arc
//!   between the rays is drawn whole when the text above it is inside,
//!   and not at all for centred text, as a linear dimension's line.
//! - Text inside sits at the middle of the arc, above it along its tangent
//!   or centred on it. Centred text breaks the arc wherever its box and a
//!   gap round it cover it, as often as the arc goes in and out of the box,
//!   and so does centred text outside lying across the arc. AutoCAD keeps the arc's point a third of the way round
//!   it, and two thirds when the text is outside and nearer the first ray.
//!   Text outside goes beyond the first ray, clockwise: the
//!   arc runs on past the arrowhead, and past its tail when it has one, and
//!   then the text, above a line along the arc's tangent as long as the
//!   text and its gaps, or centred beside a landing an arrowhead long,
//!   towards the right when the first ray is in the upper half and the left
//!   when it is in the lower. Text above its line reads along it, turned
//!   round when the first ray is in the lower half.
//! - Not matched: an angle over 180° on an arc too small for its centred
//!   text, the text wider than the arc's circle or outside it. Of 489 such
//!   made-up angles measured, AutoCAD laid out 80 otherwise, its arrowheads
//!   outside or its landing the other way, and no rule found fitted them.

use super::{
    add, dot, left, readable, sub, DimensionPart, DimensionStyle, Layout, Measured, Sizes,
    TextPlacement, FIT_TOLERANCE,
};
use crate::model::{Arrowhead, Point3};

type V = [f64; 2];

/// The sector an angular dimension measures: anticlockwise from `start`
/// degrees by `sweep`, round `vertex`, its arc of `radius`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sector {
    pub vertex: Point3,
    /// The first ray's angle, in degrees, `[0, 360)`.
    pub start: f64,
    /// The angle it measures, in degrees, `(0, 360)`.
    pub sweep: f64,
    pub radius: f64,
    /// How far along each ray, the first's and then the second's, the line
    /// or the point it measures from reaches: nearest the vertex, and
    /// farthest.
    pub reach: [[f64; 2]; 2],
    /// The direction of each ray, the first's and then the second's, as
    /// its points give it: not a unit long.
    pub rays: [[f64; 2]; 2],
}

impl Sector {
    /// The second ray's angle, in degrees, more than `start`.
    pub fn end(&self) -> f64 {
        self.start + self.sweep
    }

    /// The point at `angle` degrees, `r` from the vertex.
    pub fn at(&self, angle: f64, r: f64) -> Point3 {
        add(self.vertex, direction(angle), r)
    }

    /// The point a third of the way round the arc: where the model keeps
    /// an angular dimension's location, and AutoCAD its arc point when its
    /// text is inside.
    pub fn arc_point(&self) -> Point3 {
        self.at(self.start + self.sweep / 3.0, self.radius)
    }

    /// Whether `p` lies inside the sector clear of both its rays: so that a
    /// hair's turn of it, as rounding gives, leaves it inside.
    pub fn holds_clear(&self, p: Point3) -> bool {
        let to = [p.x - self.vertex.x, p.y - self.vertex.y];
        if to == [0.0, 0.0] {
            return false;
        }
        let off = (angle_of(to) - self.start).rem_euclid(360.0);
        off > CLEAR && off < self.sweep - CLEAR
    }

    /// The point `r` from the vertex midway round the sector, at height
    /// `z`, worked out with no sine or cosine, so the same on every system.
    pub fn middle(&self, r: f64, z: f64) -> Point3 {
        let unit = |v: V| {
            let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
            [v[0] / l, v[1] / l]
        };
        let (a, b) = (unit(self.rays[0]), unit(self.rays[1]));
        let w = [a[0] + b[0], a[1] + b[1]];
        let l = (w[0] * w[0] + w[1] * w[1]).sqrt();
        // Across a straight angle, midway is a quarter turn anticlockwise
        // from the first ray; past one, the far side of the rays' bisector.
        let w = if l < 1e-6 {
            [-a[1], a[0]]
        } else if self.sweep > 180.0 {
            [-w[0] / l, -w[1] / l]
        } else {
            [w[0] / l, w[1] / l]
        };
        Point3 {
            x: self.vertex.x + w[0] * r,
            y: self.vertex.y + w[1] * r,
            z,
        }
    }

    /// Of a three-point angular dimension's two points, the one on the
    /// first ray.
    pub fn first_point(&self, points: &[Point3; 2]) -> Point3 {
        let d = direction(self.start);
        // How far a point's ray turns from the first ray, either way: an
        // angle, which, unlike a sine, tells the ray opposite apart.
        let off = |p: Point3| {
            let v = sub(p, self.vertex);
            cross(v, d).abs().atan2(dot(v, d))
        };
        if off(points[0]) <= off(points[1]) {
            points[0]
        } else {
            points[1]
        }
    }
}

/// How far, in degrees, a point must lie from a sector's rays to be clear
/// of them.
const CLEAR: f64 = 1e-6;

fn direction(degrees: f64) -> V {
    let (s, c) = degrees.to_radians().sin_cos();
    [c, s]
}

fn angle_of(v: V) -> f64 {
    v[1].atan2(v[0]).to_degrees().rem_euclid(360.0)
}

fn cross(a: V, b: V) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// How far along a ray at `angle` from `vertex` the points reach, nearest
/// and farthest.
fn reach(vertex: Point3, angle: f64, points: &[Point3]) -> [f64; 2] {
    let d = direction(angle);
    let along: Vec<f64> = points.iter().map(|p| dot(sub(*p, vertex), d)).collect();
    [
        along.iter().copied().fold(f64::INFINITY, f64::min),
        along.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    ]
}

/// The sector `measured` measures with its arc through `location`; none
/// for a dimension that measures no angle, lines that never meet, a point
/// at the vertex, or a location there.
pub(super) fn sector(measured: &Measured, location: Point3) -> Option<Sector> {
    match measured {
        Measured::Angular { lines } => {
            let u1 = sub(lines[0][1], lines[0][0]);
            let u2 = sub(lines[1][1], lines[1][0]);
            let (l1, l2) = (u1[0].hypot(u1[1]), u2[0].hypot(u2[1]));
            let c = cross(u1, u2);
            if l1 == 0.0 || l2 == 0.0 || c.abs() <= 1e-12 * l1 * l2 {
                return None;
            }
            let t = cross(sub(lines[1][0], lines[0][0]), u2) / c;
            let vertex = add(lines[0][0], u1, t);
            let to = sub(location, vertex);
            let radius = to[0].hypot(to[1]);
            if radius == 0.0 {
                return None;
            }
            let theta = angle_of(to);
            let rays = [
                (angle_of(u1), 0, u1),
                (
                    (angle_of(u1) + 180.0).rem_euclid(360.0),
                    0,
                    [-u1[0], -u1[1]],
                ),
                (angle_of(u2), 1, u2),
                (
                    (angle_of(u2) + 180.0).rem_euclid(360.0),
                    1,
                    [-u2[0], -u2[1]],
                ),
            ];
            let behind = |a: f64| (theta - a).rem_euclid(360.0);
            let ahead = |a: f64| (a - theta).rem_euclid(360.0);
            let first = rays
                .iter()
                .min_by(|x, y| behind(x.0).total_cmp(&behind(y.0)))?;
            let second = rays
                .iter()
                .filter(|r| r.1 != first.1)
                .min_by(|x, y| ahead(x.0).total_cmp(&ahead(y.0)))?;
            let sweep = (second.0 - first.0).rem_euclid(360.0);
            if sweep == 0.0 {
                return None;
            }
            // AutoCAD takes any of a line behind the vertex, where the two
            // meet, as at the vertex, so an extension line runs from the
            // vertex along a ray the line lies behind, and from a line's
            // near end back to an arc inside the gap a line leaves short of
            // the vertex.
            let met = |[near, far]: [f64; 2]| [near.max(0.0), far.max(0.0)];
            Some(Sector {
                vertex,
                start: first.0,
                sweep,
                radius,
                reach: [
                    met(reach(vertex, first.0, &lines[first.1])),
                    met(reach(vertex, second.0, &lines[second.1])),
                ],
                rays: [first.2, second.2],
            })
        }
        Measured::ThreePointAngular { center, points } => {
            let (v1, v2) = (sub(points[0], *center), sub(points[1], *center));
            let (d1, d2) = (v1[0].hypot(v1[1]), v2[0].hypot(v2[1]));
            let to = sub(location, *center);
            let radius = to[0].hypot(to[1]);
            if d1 == 0.0 || d2 == 0.0 || radius == 0.0 {
                return None;
            }
            let (a1, a2) = (angle_of(v1), angle_of(v2));
            let theta = angle_of(to);
            let (start, end, reach, rays) =
                if (theta - a1).rem_euclid(360.0) <= (a2 - a1).rem_euclid(360.0) {
                    (a1, a2, [[d1, d1], [d2, d2]], [v1, v2])
                } else {
                    (a2, a1, [[d2, d2], [d1, d1]], [v2, v1])
                };
            let sweep = (end - start).rem_euclid(360.0);
            if sweep == 0.0 {
                return None;
            }
            Some(Sector {
                vertex: *center,
                start,
                sweep,
                radius,
                reach,
                rays,
            })
        }
        _ => None,
    }
}

/// The angle, in degrees, of a chord `c` long on a circle of radius `r`;
/// half a turn for a chord as long as the circle is across or longer, as
/// AutoCAD takes it, so that text fits an arc of more than half a turn.
fn chord_angle(c: f64, r: f64) -> f64 {
    if c >= 2.0 * r {
        180.0
    } else {
        2.0 * (c / (2.0 * r)).asin().to_degrees()
    }
}

/// The pieces of the arc round `vertex` of radius `r`, anticlockwise from
/// `from` to `to` degrees, that lie outside the axis-aligned box round
/// `center` reaching `half` each way: AutoCAD breaks an angular
/// dimension's arc wherever centred text covers it, as often as the arc
/// goes in and out of the text's box.
fn uncovered(
    vertex: Point3,
    r: f64,
    from: f64,
    to: f64,
    center: Point3,
    half: V,
) -> Vec<(f64, f64)> {
    let mut angles = Vec::new();
    for x in [center.x - half[0], center.x + half[0]] {
        let c = (x - vertex.x) / r;
        if c.abs() <= 1.0 {
            for a in [c.acos(), -c.acos()] {
                let y = vertex.y + r * a.sin();
                if (y - center.y).abs() <= half[1] + 1e-12 {
                    angles.push(a.to_degrees());
                }
            }
        }
    }
    for y in [center.y - half[1], center.y + half[1]] {
        let s = (y - vertex.y) / r;
        if s.abs() <= 1.0 {
            for a in [s.asin(), std::f64::consts::PI - s.asin()] {
                let x = vertex.x + r * a.cos();
                if (x - center.x).abs() <= half[0] + 1e-12 {
                    angles.push(a.to_degrees());
                }
            }
        }
    }
    let span = to - from;
    let mut cuts: Vec<f64> = angles
        .iter()
        .map(|a| (a - from).rem_euclid(360.0))
        .filter(|t| *t > 0.0 && *t < span)
        .collect();
    cuts.sort_by(f64::total_cmp);
    let mut ends = vec![0.0];
    ends.extend(cuts);
    ends.push(span);
    let outside = |t: f64| {
        let p = add(vertex, direction(from + t), r);
        (p.x - center.x).abs() > half[0] || (p.y - center.y).abs() > half[1]
    };
    ends.windows(2)
        .filter(|w| w[1] > w[0] && outside((w[0] + w[1]) / 2.0))
        .map(|w| (from + w[0], from + w[1]))
        .collect()
}

fn arc(sector: &Sector, start: f64, end: f64) -> DimensionPart {
    DimensionPart::Arc {
        center: sector.vertex,
        radius: sector.radius,
        start,
        end,
    }
}

/// An angular dimension's layout, of `sector`, its text's box `w` by `h`.
pub(super) fn angular(
    sector: &Sector,
    style: &DimensionStyle,
    s: Sizes,
    (w, h): (f64, f64),
) -> Layout {
    let r = sector.radius;
    let (a, g) = (s.arrow, s.gap);
    let (start, sweep, end) = (sector.start, sector.sweep, sector.end());
    let mid = start + sweep / 2.0;
    let aligned = style.placement == TextPlacement::Above;
    let bare = style.arrowhead == Arrowhead::None;
    let alpha = chord_angle(a, r);
    let fits = |need: f64| need <= sweep + FIT_TOLERANCE * (1.0 + sweep);
    let arrows_fit = bare || fits(2.0 * alpha);
    let (text_needs, both) = if aligned {
        let text = chord_angle(w + 2.0 * g, r);
        (
            text,
            if bare {
                fits(text)
            } else {
                fits(text + 2.0 * alpha)
            },
        )
    } else {
        let t = (mid + 90.0).to_radians();
        let across = (w + 2.0 * g) * t.cos().abs() + (h + 2.0 * g) * t.sin().abs();
        let text = chord_angle(across, r);
        if bare {
            (text, fits(text))
        } else {
            (text + alpha, fits(text + 3.0 * alpha))
        }
    };
    let text_fits = fits(text_needs);
    // When each fits alone but not both, AutoCAD keeps inside the one that
    // needs more room, the text when they need the same.
    let (text_in, arrows_in) = if both {
        (true, true)
    } else if text_fits && arrows_fit && !aligned {
        let text_first = text_needs >= 2.0 * alpha;
        (text_first, !text_first)
    } else if text_fits {
        (true, false)
    } else if arrows_fit {
        (false, true)
    } else {
        (false, false)
    };
    let mut parts = Vec::new();
    for (ray, [near, far]) in [(start, sector.reach[0]), (end, sector.reach[1])] {
        let d = direction(ray);
        let from = |at: f64, towards: f64| {
            DimensionPart::Line([
                add(sector.vertex, d, at + towards * s.offset),
                add(sector.vertex, d, r + towards * s.beyond),
            ])
        };
        if r > far {
            parts.push(from(far, 1.0));
        } else if r < near {
            parts.push(from(near, -1.0));
        }
    }
    let upper = start.rem_euclid(360.0) < 180.0;
    let (center, rotation);
    if text_in {
        let on = sector.at(mid, r);
        if aligned {
            rotation = readable(mid + 90.0);
            center = add(on, left(direction(rotation)), g + h / 2.0);
        } else {
            rotation = 0.0;
            center = on;
        }
    } else {
        let past = if arrows_in {
            alpha
        } else {
            chord_angle(2.0 * a, r)
        };
        let from = start - past;
        parts.push(arc(sector, from, start));
        let at = sector.at(from, r);
        if aligned {
            let d = direction(from - 90.0);
            let reads = if upper { d } else { [-d[0], -d[1]] };
            parts.push(DimensionPart::Line([at, add(at, d, w + 2.0 * g)]));
            center = add(add(at, d, g + w / 2.0), left(reads), g + h / 2.0);
            rotation = reads[1].atan2(reads[0]).to_degrees();
        } else {
            let side = [if upper { 1.0 } else { -1.0 }, 0.0];
            parts.push(DimensionPart::Line([at, add(at, side, a)]));
            center = add(at, side, a + g + w / 2.0);
            rotation = 0.0;
        }
    }
    if !arrows_in {
        parts.push(arc(sector, start - 2.0 * alpha, start - alpha));
        parts.push(arc(sector, end + alpha, end + 2.0 * alpha));
    }
    let trim = if style.arrowhead.trims_line() {
        alpha
    } else {
        0.0
    };
    if arrows_in {
        let (from, to) = (start + trim, end - trim);
        if aligned {
            parts.push(arc(sector, from, to));
        } else {
            // Centred text breaks the arc wherever its box covers it, inside
            // or, wider than the arc's circle, lying across it from outside.
            let half = [w / 2.0 + g, h / 2.0 + g];
            for (a, b) in uncovered(sector.vertex, r, from, to, center, half) {
                parts.push(arc(sector, a, b));
            }
        }
    } else if aligned {
        parts.push(arc(sector, start, end));
    }
    if !bare {
        let inward = if arrows_in { 1.0 } else { -1.0 };
        // An arrowhead longer than the arc's circle is across keeps its
        // length, from its tip through the vertex, as AutoCAD draws it.
        let base = |tip: f64, body: f64| {
            if a < 2.0 * r {
                sector.at(body, r)
            } else {
                sector.at(tip, r - a)
            }
        };
        parts.push(DimensionPart::Arrow {
            tip: sector.at(start, r),
            base: base(start, start + inward * alpha),
        });
        parts.push(DimensionPart::Arrow {
            tip: sector.at(end, r),
            base: base(end, end - inward * alpha),
        });
    }
    parts.push(DimensionPart::Text);
    // AutoCAD keeps the arc's point a third of the way round, and two
    // thirds when the text is outside and nearer the first ray than the
    // second.
    let beyond = {
        let at = sub(center, sector.vertex);
        let t = (at[1].atan2(at[0]).to_degrees() - start).rem_euclid(360.0);
        (t > sweep).then_some(360.0 - t < t - sweep)
    };
    let arc_point = if !text_in && beyond == Some(true) {
        Some(sector.at(start + sweep * 2.0 / 3.0, r))
    } else {
        None
    };
    Layout {
        parts,
        center,
        rotation,
        text_point: center,
        placed: false,
        arc_point,
        defpoints: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dimension::{Dimension, ISO_DIMENSION_STYLE};
    use crate::model::{MTextBox, Properties};
    use crate::text::MText;
    use crate::units::Unit;

    fn p(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    fn close(a: Point3, b: Point3) {
        assert!(
            (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3,
            "{a:?} is not {b:?}"
        );
    }

    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-3, "{a} is not {b}");
    }

    fn boxed(width: f64) -> impl Fn(&MText) -> MTextBox {
        move |t: &MText| MTextBox {
            width,
            height: t.height,
            ..MTextBox::default()
        }
    }

    fn two_lines(lines: [[Point3; 2]; 2], location: Point3, style: &str) -> Dimension {
        Dimension {
            measured: Measured::Angular { lines },
            location,
            dimension_style: style.to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        }
    }

    fn arcs(laid: &crate::model::LaidDimension) -> Vec<(f64, f64)> {
        laid.arcs()
            .map(|(_, _, a, b)| (a.rem_euclid(360.0), b.rem_euclid(360.0)))
            .collect()
    }

    /// AutoCAD's `ang2-0` in ISO-25: lines from (0,0) to (40,0) and to
    /// (30,30), the arc through (25,10).
    #[test]
    fn an_iso_angle_between_two_lines_is_laid_out_as_autocad_laid_out_its_own() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let d = two_lines(
            [[p(0.0, 0.0), p(40.0, 0.0)], [p(0.0, 0.0), p(30.0, 30.0)]],
            p(25.0, 10.0),
            ISO_DIMENSION_STYLE,
        );
        near(d.measurement(), 45.0);
        let sector = d.sector().unwrap();
        close(sector.arc_point(), p(26.0083, 6.9689));
        let laid = d.laid_out(&style, Unit::Mm, &boxed(5.8333));
        assert_eq!(laid.text.contents, "45\u{b0}");
        assert_eq!(laid.lines().count(), 0);
        let a = arcs(&laid);
        assert_eq!(a.len(), 1);
        near(a[0].0, 5.3217);
        near(a[0].1, 39.6783);
        close(laid.text.position, p(26.6085, 11.0216));
        near(laid.text.rotation, 292.5);
    }

    /// AutoCAD's `ang2-1` in ISO-25: the arc through (-20,5) from (120,0),
    /// in the sector from the second line's ray round to the first's
    /// opposite, which an extension line reaches from the vertex.
    #[test]
    fn an_extension_line_reaches_a_ray_its_line_does_not_lie_on() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let d = two_lines(
            [
                [p(120.0, 0.0), p(160.0, 0.0)],
                [p(120.0, 0.0), p(150.0, 30.0)],
            ],
            p(100.0, 5.0),
            ISO_DIMENSION_STYLE,
        );
        near(d.measurement(), 135.0);
        let laid = d.laid_out(&style, Unit::Mm, &boxed(7.5));
        let l: Vec<_> = laid.lines().collect();
        assert_eq!(l.len(), 1);
        close(l[0][0], p(119.375, 0.0));
        close(l[0][1], p(98.1345, 0.0));
        close(laid.text.position, p(111.3932, 20.7785));
    }

    /// AutoCAD's `ang2-narrow` in ISO-25: 4.29° at radius 30, the text
    /// and arrowheads outside.
    #[test]
    fn a_narrow_iso_angle_puts_its_text_beyond_the_first_ray() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let d = two_lines(
            [
                [p(120.0, 60.0), p(160.0, 60.0)],
                [p(120.0, 60.0), p(160.0, 63.0)],
            ],
            p(149.9793, 61.4974),
            ISO_DIMENSION_STYLE,
        );
        let laid = d.laid_out(&style, Unit::Mm, &boxed(3.3333));
        let a = arcs(&laid);
        assert_eq!(a.len(), 4);
        near(a[0].0, 350.4449);
        near(a[1].0, 350.4532);
        near(a[1].1, 355.2266);
        near(a[2].0, 9.0625);
        near(a[3].0, 0.0);
        near(a[3].1, 4.2892);
        let l: Vec<_> = laid.lines().collect();
        close(l[0][0], p(149.6002, 55.0174));
        close(l[0][1], p(148.8394, 50.4976));
        close(laid.text.position, p(151.0688, 52.4463));
        near(laid.text.rotation, 260.4449);
    }

    /// AutoCAD's `ang-3pt-reflex` in Standard: the vertex (8,14), the
    /// points (11,14) and (9,17), the arc through (7,13): 288°, centred
    /// text breaking the arc.
    #[test]
    fn a_standard_reflex_angle_breaks_its_arc_round_its_text() {
        let style = DimensionStyle::standard(Unit::In);
        let d = Dimension {
            measured: Measured::ThreePointAngular {
                center: p(8.0, 14.0),
                points: [p(11.0, 14.0), p(9.0, 17.0)],
            },
            location: p(7.0, 13.0),
            dimension_style: "Standard".to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        };
        near(d.measurement(), 288.4349);
        close(d.sector().unwrap().arc_point(), p(6.6182, 14.301));
        let laid = d.laid_out(&style, Unit::In, &boxed(0.6));
        let l: Vec<_> = laid.lines().collect();
        close(l[0][0], p(8.9802, 16.9407));
        close(l[0][1], p(8.3903, 15.1709));
        close(l[1][0], p(10.9375, 14.0));
        close(l[1][1], p(9.2342, 14.0));
        let a = arcs(&laid);
        near(a[0].0, 78.8625);
        near(a[0].1, 207.2215);
        near(a[1].0, 225.397);
        near(a[1].1, 352.7025);
        close(laid.text.position, p(6.8527, 13.1731));
        assert_eq!(laid.text.contents, "288\u{b0}");
    }

    /// AutoCAD's `turn-200` in Standard: 4° from 200°, outside, landing to
    /// the left.
    #[test]
    fn standard_text_outside_lands_left_of_a_first_ray_in_the_lower_half() {
        let style = DimensionStyle::standard(Unit::In);
        let at = |a: f64, r: f64| {
            let (s, c) = a.to_radians().sin_cos();
            p(32.0 + r * c, 24.0 + r * s)
        };
        let d = Dimension {
            measured: Measured::ThreePointAngular {
                center: p(32.0, 24.0),
                points: [at(200.0, 4.0), at(204.0, 4.0)],
            },
            location: at(202.0, 2.0),
            dimension_style: "Standard".to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        };
        let laid = d.laid_out(&style, Unit::In, &boxed(0.24));
        let l: Vec<_> = laid.lines().collect();
        close(l[2][0], p(30.0284, 23.664));
        close(l[2][1], p(29.8484, 23.664));
        close(laid.text.position, p(29.6384, 23.664));
        let a = arcs(&laid);
        near(a[0].0, 189.6728);
        assert_eq!(a.len(), 3);
    }

    /// AutoCAD 2026 in ISO-25: a line lying behind the vertex, on the ray
    /// opposite the one the angle is measured from, gets an extension line
    /// from the vertex, as though it reached it.
    #[test]
    fn a_line_behind_the_vertex_gets_an_extension_line_from_the_vertex() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let d = two_lines(
            [
                [p(2340.0, 720.0), p(2380.0, 720.0)],
                [p(2343.0, 723.0), p(2370.0, 750.0)],
            ],
            p(2328.0, 714.0),
            ISO_DIMENSION_STYLE,
        );
        let laid = d.laid_out(&style, Unit::Mm, &boxed(5.8333));
        let l: Vec<_> = laid.lines().collect();
        close(l[1][0], p(2339.5581, 719.5581));
        close(l[1][1], p(2329.6293, 709.6293));
    }

    /// AutoCAD 2026 in ISO-25, scaled 1.5 with 5 high text: at radius 4,
    /// a 15° angle's text goes outside and round past the second ray, and
    /// AutoCAD keeps the arc's point a third of the way round, not two.
    #[test]
    fn the_arc_point_is_two_thirds_round_only_for_text_nearer_the_first_ray() {
        let style = DimensionStyle {
            text_height: 5.0,
            gap: 1.5,
            scale: 1.5,
            ..DimensionStyle::iso_25(Unit::Mm)
        };
        let d = Dimension {
            measured: Measured::ThreePointAngular {
                center: p(540.0, 90.0),
                points: [p(533.0541, 129.3923), p(531.5476, 108.1262)],
            },
            location: p(538.6319, 93.7588),
            dimension_style: "big".to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        };
        let laid = d.laid_out(&style, Unit::Mm, &boxed(15.0));
        close(laid.text.position, p(541.5698, 76.122));
        assert_eq!(laid.arc_point, None);
    }

    /// AutoCAD 2026 in ISO-25: lines that stop short of where they meet,
    /// the arc inside the gap, get extension lines from their near ends
    /// back to the arc.
    #[test]
    fn lines_short_of_the_vertex_get_extension_lines_back_to_the_arc() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let d = two_lines(
            [
                [p(910.0, 780.0), p(940.0, 780.0)],
                [p(910.0, 790.0), p(930.0, 810.0)],
            ],
            p(906.0, 782.5),
            ISO_DIMENSION_STYLE,
        );
        let laid = d.laid_out(&style, Unit::Mm, &boxed(5.8333));
        let l: Vec<_> = laid.lines().collect();
        close(l[0][0], p(909.375, 780.0));
        close(l[0][1], p(905.25, 780.0));
        close(l[1][0], p(909.5581, 789.5581));
        close(l[1][1], p(903.7123, 783.7123));
    }

    /// AutoCAD 2026 in the acad template's Standard: a 300° angle at radius
    /// 0.4 whose centred text is wider than the arc's circle breaks the arc
    /// wherever the text's box covers it, three pieces here, not two.
    #[test]
    fn centred_text_breaks_the_arc_everywhere_its_box_covers_it() {
        let style = DimensionStyle::standard(Unit::In);
        let d = Dimension {
            measured: Measured::ThreePointAngular {
                center: p(153.0, 18.0),
                points: [p(157.0, 18.0), p(154.0, 16.267949)],
            },
            location: p(152.9305, 18.393923),
            dimension_style: "Standard".to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        };
        let laid = d.laid_out(&style, Unit::In, &boxed(1.05));
        let a = arcs(&laid);
        // The text's width is AutoCAD's to two decimals, so its box's edges
        // land within a hundredth of a degree.
        let near = |a: f64, b: f64| assert!((a - b).abs() < 0.01, "{a} is not {b}");
        assert_eq!(a.len(), 3);
        near(a[0].0, 26.0058);
        near(a[0].1, 47.819);
        near(a[1].0, 71.8052);
        near(a[1].1, 108.1948);
        near(a[2].0, 177.134);
        near(a[2].1, 273.9942);
    }
}
