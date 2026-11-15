//! An ordinate dimension, laid out as AutoCAD 2026 laid out its own
//! DIMORDINATE of made-up features in its `acad` and `acadiso` templates,
//! the leader's end above, below and to each side, near and far:
//!
//! - It measures its feature's X, or its Y, from the origin: the world's,
//!   which is AutoCAD's UCS origin in the drawings the core writes. The
//!   value shows without its sign.
//! - Its leader runs from the style's offset past the feature, across the
//!   axis it measures, to its location, backwards when its location is
//!   level with the feature across the axis: straight when the two are in line;
//!   otherwise kinked, running from the feature to four arrowheads short of
//!   the location, or two past the feature when that is nearer it, then
//!   across to two short of the location, in line with it, and on.
//! - Its text reads along the leader, up for an X and across for a Y,
//!   whichever way the leader runs. Text above its line, as ISO-25 has it,
//!   sits a gap above the leader past its end, which runs on under it; the
//!   leader of centred text, as Standard has it, stops at its end, and the
//!   text sits on in line with it, a gap away.

use super::{add, left, DimensionPart, DimensionStyle, Layout, OrdinateAxis, Sizes};
use crate::model::Point3;

pub(super) fn ordinate(
    feature: Point3,
    axis: OrdinateAxis,
    end: Point3,
    style: &DimensionStyle,
    s: Sizes,
    (w, h): (f64, f64),
) -> Layout {
    let (a, g) = (s.arrow, s.gap);
    // The leader runs across the axis it measures.
    let (leader, rotation) = match axis {
        OrdinateAxis::X => ([0.0, 1.0], 90.0),
        OrdinateAxis::Y => ([1.0, 0.0], 0.0),
    };
    let lateral = [leader[1], leader[0]];
    let along = |p: Point3| p.x * leader[0] + p.y * leader[1];
    let across = |p: Point3| p.x * lateral[0] + p.y * lateral[1];
    // A leader ending level with its feature, across the axis, runs back
    // from it, as AutoCAD runs it.
    let sign = if along(end) <= along(feature) {
        -1.0
    } else {
        1.0
    };
    let dir = [leader[0] * sign, leader[1] * sign];
    let reads = [leader[0], leader[1]];
    let above = style.placement == super::TextPlacement::Above;
    let start = add(feature, dir, s.offset);
    let last = if above { add(end, dir, w + g) } else { end };
    let mut parts = Vec::new();
    let reach = [feature, end]
        .iter()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(1.0, f64::max);
    if (across(end) - across(feature)).abs() <= 1e-12 * reach {
        parts.push(DimensionPart::Line([start, last]));
    } else {
        let mut first = along(end) - sign * 4.0 * a;
        let least = along(feature) + sign * 2.0 * a;
        if (first - least) * sign < 0.0 {
            first = least;
        }
        // In line with the feature, then with the location.
        let kink = add(feature, dir, (first - along(feature)) * sign);
        let second = add(end, dir, -2.0 * a);
        parts.push(DimensionPart::Line([start, kink]));
        parts.push(DimensionPart::Line([kink, second]));
        parts.push(DimensionPart::Line([second, last]));
    }
    parts.push(DimensionPart::Text);
    let center = if above {
        add(add(end, dir, g + w / 2.0), left(reads), g + h / 2.0)
    } else {
        add(end, dir, g + w / 2.0)
    };
    Layout {
        parts,
        center,
        rotation,
        text_point: center,
        placed: false,
        arc_point: None,
        defpoints: vec![feature, end, Point3::new(0.0, 0.0, feature.z)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dimension::{Dimension, Measured, ISO_DIMENSION_STYLE};
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

    fn ordinate(feature: Point3, axis: OrdinateAxis, end: Point3, style: &str) -> Dimension {
        Dimension {
            measured: Measured::Ordinate { feature, axis },
            location: end,
            dimension_style: style.to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        }
    }

    fn boxed(width: f64) -> impl Fn(&MText) -> MTextBox {
        move |t: &MText| MTextBox {
            width,
            height: t.height,
            ..MTextBox::default()
        }
    }

    /// AutoCAD's `ord-1`, `ord-3` and `ord-6` in ISO-25.
    #[test]
    fn iso_ordinates_are_laid_out_as_autocad_laid_out_its_own() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let kinked = ordinate(
            p(410.0, 220.0),
            OrdinateAxis::X,
            p(425.0, 250.0),
            ISO_DIMENSION_STYLE,
        )
        .laid_out(&style, Unit::Mm, &boxed(5.4167));
        assert_eq!(kinked.text.contents, "410");
        let l: Vec<_> = kinked.lines().collect();
        close(l[0][0], p(410.0, 220.625));
        close(l[0][1], p(410.0, 240.0));
        close(l[1][1], p(425.0, 245.0));
        close(l[2][1], p(425.0, 256.0417));
        close(kinked.text.position, p(423.125, 253.3333));
        assert_eq!(kinked.text.rotation, 90.0);
        let down = ordinate(
            p(610.0, 220.0),
            OrdinateAxis::X,
            p(610.0, 190.0),
            ISO_DIMENSION_STYLE,
        )
        .laid_out(&style, Unit::Mm, &boxed(5.4167));
        let l: Vec<_> = down.lines().collect();
        assert_eq!(l.len(), 1);
        close(l[0][0], p(610.0, 219.375));
        close(l[0][1], p(610.0, 183.9583));
        close(down.text.position, p(608.125, 186.6667));
        let left = ordinate(
            p(910.0, 220.0),
            OrdinateAxis::Y,
            p(870.0, 205.0),
            ISO_DIMENSION_STYLE,
        )
        .laid_out(&style, Unit::Mm, &boxed(6.25));
        let l: Vec<_> = left.lines().collect();
        close(l[0][1], p(880.0, 220.0));
        close(l[1][1], p(875.0, 205.0));
        close(l[2][1], p(863.125, 205.0));
        close(left.text.position, p(866.25, 206.875));
        assert_eq!(left.text.rotation, 0.0);
    }

    /// AutoCAD's `ord-9` in Standard: a leader too short for its kink
    /// keeps the kink two arrowheads past the feature.
    #[test]
    fn a_short_standard_leader_keeps_its_kink_two_arrowheads_past_the_feature() {
        let style = DimensionStyle::standard(Unit::In);
        let laid = ordinate(p(121.0, 22.0), OrdinateAxis::X, p(121.1, 22.5), "Standard").laid_out(
            &style,
            Unit::In,
            &boxed(1.02),
        );
        let l: Vec<_> = laid.lines().collect();
        close(l[0][0], p(121.0, 22.0625));
        close(l[0][1], p(121.0, 22.36));
        close(l[1][1], p(121.1, 22.14));
        close(l[2][1], p(121.1, 22.5));
        close(laid.text.position, p(121.1, 23.1));
        assert_eq!(
            laid.defpoints,
            vec![p(121.0, 22.0), p(121.1, 22.5), p(0.0, 0.0)]
        );
    }

    /// AutoCAD 2026 in ISO-25: a Y ordinate whose location is straight
    /// below its feature runs its leader back, to the left.
    #[test]
    fn a_leader_ending_level_with_its_feature_runs_back() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let laid = ordinate(
            p(727.25, 980.0),
            OrdinateAxis::Y,
            p(727.25, 950.0),
            ISO_DIMENSION_STYLE,
        )
        .laid_out(&style, Unit::Mm, &boxed(6.25));
        let l: Vec<_> = laid.lines().collect();
        close(l[0][0], p(726.625, 980.0));
        close(l[0][1], p(722.25, 980.0));
        close(l[1][1], p(732.25, 950.0));
        close(l[2][1], p(720.375, 950.0));
        close(laid.text.position, p(723.5, 951.875));
    }
}
