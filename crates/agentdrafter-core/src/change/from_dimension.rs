//! A dimension added from another, as AutoCAD 2026's DIMBASELINE and
//! DIMCONTINUE added its own from made-up linear, aligned, angular and
//! ordinate dimensions in its `acad` and `acadiso` templates:
//!
//! - A baseline dimension measures from the other's first extension line,
//!   its dimension line the style's baseline spacing, DIMDLI, further out
//!   from the points than the other's; an angular one's arc that much
//!   further from the vertex.
//! - A continued dimension measures from the other's second extension line,
//!   on the same dimension line or arc. AutoCAD marks it continued, with
//!   DIMTMOVE overridden.
//! - A dimension from an aligned one is linear, at the aligned one's angle
//!   turned half round.
//! - An ordinate dimension's, baseline or continued alike, measures the new
//!   feature on the same axis, its leader straight and ending level with
//!   the other's; AutoCAD marks neither.
//!
//! The new dimension is written as a sheet description writes it, with the
//! other's style and layer, its points in the sheet's unit.

use serde_json::{json, Map, Value};

use super::operations::FromOp;
use crate::description::number_text;
use crate::model::{Dimension, DimensionStyle, Measured, OrdinateAxis, Point3};

/// `v` as a sheet description writes it: its shortest form.
fn number_value(v: f64) -> Value {
    let text = number_text(v);
    match text.parse::<i64>() {
        Ok(whole) => Value::from(whole),
        Err(_) => Value::from(text.parse::<f64>().unwrap_or(v)),
    }
}

fn xyz(p: Point3) -> Value {
    json!([number_value(p.x), number_value(p.y), number_value(p.z)])
}

fn angle_of(v: [f64; 2]) -> f64 {
    v[1].atan2(v[0]).to_degrees().rem_euclid(360.0)
}

/// The keys of the dimension added `from` `base`, drawn with `style`, to
/// `point`; or why it cannot be.
pub(crate) fn dimension_from(
    base: &Dimension,
    style: &DimensionStyle,
    from: FromOp,
    point: Point3,
) -> Result<Map<String, Value>, String> {
    let spacing = style.baseline_spacing * style.scale;
    let mut keys = Map::new();
    let continued = from == FromOp::Continued;
    match &base.measured {
        Measured::Linear { points, .. } | Measured::Aligned { points } => {
            let angle = match &base.measured {
                Measured::Linear { angle, .. } => *angle,
                // AutoCAD measures from an aligned dimension along its
                // angle turned half round.
                _ => (angle_of([points[1].x - points[0].x, points[1].y - points[0].y]) + 180.0)
                    .rem_euclid(360.0),
            };
            let (s, c) = angle.to_radians().sin_cos();
            let n = [-s, c];
            let across =
                |p: Point3| (base.location.x - p.x) * n[0] + (base.location.y - p.y) * n[1];
            let out = if across(points[0]) != 0.0 {
                across(points[0]).signum()
            } else {
                across(points[1]).signum()
            };
            let (first, location) = match from {
                FromOp::Baseline => (
                    points[0],
                    Point3::new(
                        base.location.x + n[0] * out * spacing,
                        base.location.y + n[1] * out * spacing,
                        base.location.z,
                    ),
                ),
                FromOp::Continued => (points[1], base.location),
            };
            keys.insert("kind".into(), json!("linear"));
            keys.insert("points".into(), json!([xyz(first), xyz(point)]));
            if angle != 0.0 {
                keys.insert("angle".into(), number_value(angle));
            }
            keys.insert("location".into(), xyz(location));
        }
        Measured::ThreePointAngular { center, points } => {
            let sector = base
                .sector()
                .ok_or("the dimension it is added from measures no angle")?;
            let v = *center;
            let at = |p: Point3| angle_of([p.x - v.x, p.y - v.y]);
            // Whether the first point is on the arc's first ray: the new
            // arc runs the same way round from it.
            let first_starts = sector.first_point(points) == points[0];
            let kept = match from {
                FromOp::Baseline => points[0],
                FromOp::Continued => points[1],
            };
            let (start, end) = if first_starts {
                (at(kept), at(point))
            } else {
                (at(point), at(kept))
            };
            let sweep = (end - start).rem_euclid(360.0);
            if sweep == 0.0 || (point.x == v.x && point.y == v.y) {
                return Err(
                    "the point lies on the ray of the point it measures from; it measures no \
                     angle"
                        .to_owned(),
                );
            }
            let radius = sector.radius
                + match from {
                    FromOp::Baseline => spacing,
                    FromOp::Continued => 0.0,
                };
            let middle = (start + sweep / 3.0).to_radians();
            let location = Point3::new(
                v.x + radius * middle.cos(),
                v.y + radius * middle.sin(),
                v.z,
            );
            keys.insert("kind".into(), json!("three_point_angular"));
            keys.insert("center".into(), xyz(v));
            keys.insert("points".into(), json!([xyz(kept), xyz(point)]));
            keys.insert("location".into(), xyz(location));
        }
        Measured::Ordinate { axis, .. } => {
            // The leader ends level with the other's, straight from the new
            // feature.
            let end = match axis {
                OrdinateAxis::X => Point3::new(point.x, base.location.y, point.z),
                OrdinateAxis::Y => Point3::new(base.location.x, point.y, point.z),
            };
            keys.insert("kind".into(), json!("ordinate"));
            keys.insert("feature".into(), xyz(point));
            keys.insert("axis".into(), json!(axis.name()));
            keys.insert("location".into(), xyz(end));
            return Ok(finish(keys, base, false));
        }
        Measured::Angular { .. } => {
            return Err(
                "a dimension is added from a linear, aligned, three-point angular or ordinate \
                 dimension; this one is angular between two lines: add the next from a \
                 three-point angular dimension at their vertex"
                    .to_owned(),
            )
        }
        Measured::Radius { .. } | Measured::Diameter { .. } => {
            return Err(
                "a dimension is added from a linear, aligned, three-point angular or ordinate \
                 dimension, as AutoCAD's DIMBASELINE and DIMCONTINUE take one; this one is a \
                 radius or diameter dimension"
                    .to_owned(),
            )
        }
    }
    Ok(finish(keys, base, continued))
}

/// `keys` with the other's style and layer, and marked continued.
fn finish(mut keys: Map<String, Value>, base: &Dimension, continued: bool) -> Map<String, Value> {
    keys.insert("dimension_style".into(), json!(base.dimension_style));
    if base.layer != "0" {
        keys.insert("layer".into(), json!(base.layer));
    }
    if continued {
        keys.insert("continued".into(), json!(true));
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Properties;
    use crate::units::Unit;

    fn p(x: f64, y: f64) -> Point3 {
        Point3::new(x, y, 0.0)
    }

    fn dimension(measured: Measured, location: Point3) -> Dimension {
        Dimension {
            measured,
            location,
            dimension_style: "ISO-25".to_owned(),
            text: None,
            continued: false,
            layer: "0".to_owned(),
            properties: Properties::default(),
        }
    }

    fn point_of(v: &Value) -> [f64; 3] {
        let a = v.as_array().unwrap();
        [0, 1, 2].map(|i| a[i].as_f64().unwrap())
    }

    /// AutoCAD's `lin-base` and `lin-cont` in ISO-25.
    #[test]
    fn a_linear_baseline_is_further_out_and_a_continued_one_on_the_same_line() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let base = dimension(
            Measured::Linear {
                points: [p(0.0, 320.0), p(30.0, 320.0)],
                angle: 0.0,
            },
            p(30.0, 310.0),
        );
        let baseline = dimension_from(&base, &style, FromOp::Baseline, p(50.0, 320.0)).unwrap();
        assert_eq!(point_of(&baseline["points"][0]), [0.0, 320.0, 0.0]);
        assert_eq!(point_of(&baseline["location"]), [30.0, 306.25, 0.0]);
        assert!(baseline.get("continued").is_none());
        let continued = dimension_from(&base, &style, FromOp::Continued, p(50.0, 320.0)).unwrap();
        assert_eq!(point_of(&continued["points"][0]), [30.0, 320.0, 0.0]);
        assert_eq!(point_of(&continued["location"]), [30.0, 310.0, 0.0]);
        assert_eq!(continued["continued"], json!(true));
    }

    /// AutoCAD's `ali-base`: an aligned dimension's baseline is linear at
    /// its angle turned half round.
    #[test]
    fn a_baseline_from_an_aligned_dimension_is_linear_turned_half_round() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let base = dimension(
            Measured::Aligned {
                points: [p(200.0, 320.0), p(230.0, 340.0)],
            },
            p(220.0, 355.0),
        );
        let keys = dimension_from(&base, &style, FromOp::Baseline, p(260.0, 360.0)).unwrap();
        assert_eq!(keys["kind"], json!("linear"));
        assert!((keys["angle"].as_f64().unwrap() - 213.690_067_525_979_8).abs() < 1e-9);
        let at = point_of(&keys["location"]);
        // On the line AutoCAD drew, through (247.9199, 378.1202).
        let n = [
            -(213.690_067_5f64.to_radians().sin()),
            213.690_067_5f64.to_radians().cos(),
        ];
        let off = (at[0] - 247.9199) * n[0] + (at[1] - 378.1202) * n[1];
        assert!(off.abs() < 1e-3, "{at:?}");
    }

    /// AutoCAD's `ang-base` and `ang-cont`: the vertex (400,320), the
    /// points (430,320) and (425,335).
    #[test]
    fn an_angular_baseline_reaches_further_out_and_a_continued_one_runs_on() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let base = dimension(
            Measured::ThreePointAngular {
                center: p(400.0, 320.0),
                points: [p(430.0, 320.0), p(425.0, 335.0)],
            },
            p(420.2819, 323.6936),
        );
        let baseline = dimension_from(&base, &style, FromOp::Baseline, p(410.0, 345.0)).unwrap();
        assert_eq!(point_of(&baseline["points"][0]), [430.0, 320.0, 0.0]);
        let at = point_of(&baseline["location"]);
        assert!(
            (at[0] - 422.4727).abs() < 1e-3 && (at[1] - 329.4157).abs() < 1e-3,
            "{at:?}"
        );
        let continued = dimension_from(&base, &style, FromOp::Continued, p(410.0, 345.0)).unwrap();
        assert_eq!(point_of(&continued["points"][0]), [425.0, 335.0, 0.0]);
        assert_eq!(continued["continued"], json!(true));
    }

    /// AutoCAD's `ord-base`: the leader ends level with the other's.
    #[test]
    fn an_ordinate_from_another_ends_level_with_it() {
        let style = DimensionStyle::iso_25(Unit::Mm);
        let base = dimension(
            Measured::Ordinate {
                feature: p(560.0, 320.0),
                axis: OrdinateAxis::X,
            },
            p(560.0, 350.0),
        );
        let keys = dimension_from(&base, &style, FromOp::Continued, p(580.0, 330.0)).unwrap();
        assert_eq!(point_of(&keys["location"]), [580.0, 350.0, 0.0]);
        assert!(keys.get("continued").is_none());
    }
}
