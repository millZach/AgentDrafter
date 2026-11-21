//! Where a copy or an array puts each copy: how far it moves, how far it
//! then turns, and about which point.
//!
//! Every pattern keeps its objects where they are, as its first item, and
//! places copies from there. A copy is moved by its offset. A rectangular
//! array moves each copy by its row times the row spacing up and its
//! column times the column spacing across. A polar array turns each copy
//! about the centre, or, unturned, moves it so that its base point goes
//! round. A path array moves each copy by how far the path's point at its
//! distance along is from the path's start, and, aligned, turns it about
//! that point by how far the path's direction there is from its direction
//! at its start: the objects ride along the path as they sit at its start.
//! Each is worked out here from the pattern and the geometry the change
//! set read, so it can be checked alone.

use crate::geometry::curve::{add, length_of, sub, Curve, P3};

/// One place a copy goes: moved by `by`, then turned by `turn` degrees,
/// anticlockwise, about `about`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Place {
    pub by: P3,
    pub turn: f64,
    pub about: P3,
    pub at: At,
}

/// Which place of its pattern a copy is at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum At {
    /// A copy's offset, counted from 0.
    Offset(usize),
    /// A rectangular array's row and column, counted from 0: the objects
    /// are at row 0, column 0.
    Grid { row: usize, column: usize },
    /// A polar array's item, counted from 0: the objects are item 0.
    Item(usize),
    /// A path array's item, counted from 0, and how far along the path it
    /// is.
    Along { item: usize, distance: f64 },
}

/// `x` with the noise of adding binary fractions and of sines taken off,
/// to a trillionth of the larger of it and 1.
fn tidy(x: f64) -> f64 {
    let scale = x.abs().max(1.0);
    let step = scale * 1e-12;
    let rounded = (x / step).round() * step;
    let whole = x.round();
    if (x - whole).abs() <= step {
        whole + 0.0
    } else {
        rounded + 0.0
    }
}

fn tidy3(p: P3) -> P3 {
    [tidy(p[0]), tidy(p[1]), tidy(p[2])]
}

/// `v` turned by `degrees` anticlockwise about the Z axis.
fn turned(v: P3, degrees: f64) -> P3 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [v[0] * cos - v[1] * sin, v[0] * sin + v[1] * cos, v[2]]
}

/// A copy's places: one moved by each offset.
pub(crate) fn offsets(by: &[P3]) -> Vec<Place> {
    by.iter()
        .enumerate()
        .map(|(i, by)| Place {
            by: tidy3(*by),
            turn: 0.0,
            about: [0.0; 3],
            at: At::Offset(i),
        })
        .collect()
}

/// A rectangular array's places, row by row from the objects' own: each
/// row `row_spacing` up from the one before, each column `column_spacing`
/// across.
pub(crate) fn rectangular(
    rows: usize,
    columns: usize,
    row_spacing: f64,
    column_spacing: f64,
) -> Vec<Place> {
    let mut places = Vec::with_capacity(rows * columns);
    for row in 0..rows {
        for column in 0..columns {
            if row == 0 && column == 0 {
                continue;
            }
            places.push(Place {
                by: tidy3([
                    column as f64 * column_spacing,
                    row as f64 * row_spacing,
                    0.0,
                ]),
                turn: 0.0,
                about: [0.0; 3],
                at: At::Grid { row, column },
            });
        }
    }
    places
}

/// The angle between a polar array's items: a whole turn spaces `count`
/// items evenly round it, and less puts the last at its end, as AutoCAD's
/// ARRAYPOLAR fills an angle.
pub(crate) fn polar_step(count: usize, angle: f64) -> f64 {
    if angle.abs() >= 360.0 {
        angle / count as f64
    } else {
        angle / (count - 1) as f64
    }
}

/// A polar array's places round `center`: `count` items over `angle`
/// degrees, the objects the first. Turned, each copy turns about the
/// centre; unturned, it moves so that `base`, its base point, goes round.
pub(crate) fn polar(center: P3, count: usize, angle: f64, rotate: bool, base: P3) -> Vec<Place> {
    let step = polar_step(count, angle);
    (1..count)
        .map(|i| {
            let degrees = step * i as f64;
            if rotate {
                Place {
                    by: [0.0; 3],
                    turn: tidy(degrees),
                    about: tidy3(center),
                    at: At::Item(i),
                }
            } else {
                let from_center = sub(base, center);
                let landed = add(center, turned(from_center, degrees));
                Place {
                    by: tidy3(sub(landed, base)),
                    turn: 0.0,
                    about: [0.0; 3],
                    at: At::Item(i),
                }
            }
        })
        .collect()
}

/// A path, its curves end to end, as a path array runs along it.
struct Path<'a> {
    curves: &'a [Curve],
    /// How far along the path each curve starts, and its length.
    starts: Vec<(f64, f64)>,
    length: f64,
}

impl<'a> Path<'a> {
    fn new(curves: &'a [Curve]) -> Self {
        let mut starts = Vec::with_capacity(curves.len());
        let mut length = 0.0;
        for curve in curves {
            let l = curve.length();
            starts.push((length, l));
            length += l;
        }
        Path {
            curves,
            starts,
            length,
        }
    }

    /// Whether it ends where it starts, as a circle or a closed polyline
    /// does.
    fn closed(&self) -> bool {
        let (Some(first), Some(last)) = (self.curves.first(), self.curves.last()) else {
            return false;
        };
        let (a, b) = (first.start(), last.end());
        let scale = a.iter().chain(&b).fold(1.0_f64, |m, c| m.max(c.abs()));
        length_of(sub(a, b)) <= scale * 1e-9
    }

    /// The point `distance` along it, and its direction there, in radians
    /// anticlockwise from the X axis. A point where two curves meet takes
    /// the direction of the curve after it, and the path's end the last
    /// curve's. A distance within a billionth of the path's length of the
    /// end of a curve is at that end, so that the rounding of adding
    /// lengths up does not put a point that falls where two curves meet
    /// just inside the first, and turn it the first one's way.
    fn at(&self, distance: f64) -> (P3, f64) {
        let n = self.curves.len();
        let near = self.length * 1e-9;
        let j = (0..n)
            .find(|&j| {
                let (start, length) = self.starts[j];
                distance < start + length - near && length > 0.0
            })
            .or_else(|| (0..n).rev().find(|&j| self.starts[j].1 > 0.0))
            .unwrap_or(0);
        let curve = &self.curves[j];
        let (start, length) = self.starts[j];
        let into = (distance - start).clamp(0.0, length);
        let t = parameter_at(curve, into, length);
        let tangent = curve.tangent(t);
        (curve.point(t), tangent[1].atan2(tangent[0]))
    }
}

/// The parameter `distance` along `curve`, whose length is `length`.
fn parameter_at(curve: &Curve, distance: f64, length: f64) -> f64 {
    let (a, b) = curve.domain();
    if length <= 0.0 {
        return a;
    }
    match curve {
        // Along a segment or an arc of a circle, distance grows evenly
        // with the parameter.
        Curve::Segment { .. } | Curve::Arc { .. } => a + (b - a) * (distance / length),
        _ => {
            let (mut lo, mut hi) = (a, b);
            for _ in 0..80 {
                let mid = (lo + hi) / 2.0;
                if curve.length_between(a, mid) < distance {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            (lo + hi) / 2.0
        }
    }
}

/// A path array's places along `curves`, the path's curves end to end:
/// `count` items, the objects the first, at the path's start, `spacing`
/// apart, or spaced evenly from its start to its end, or round it when it
/// is closed. Aligned, each copy turns with the path's direction. The
/// error, in words, when the path has no length or the items run past its
/// end.
pub(crate) fn along(
    curves: &[Curve],
    count: usize,
    spacing: Option<f64>,
    align: bool,
) -> Result<Vec<Place>, String> {
    let path = Path::new(curves);
    if path.length <= 0.0 || !path.length.is_finite() {
        return Err("the path has no length to run along".to_owned());
    }
    let step = match spacing {
        Some(spacing) => {
            let run = spacing * (count - 1) as f64;
            if run > path.length * (1.0 + 1e-9) {
                return Err(format!(
                    "{count} items {} apart run {} along the path, which is {} long; give a \
                     smaller count or spacing, or leave spacing out to space them evenly",
                    crate::description::number_text(spacing),
                    crate::description::number_text(run),
                    crate::description::number_text(path.length)
                ));
            }
            spacing
        }
        None if path.closed() => path.length / count as f64,
        None => path.length / (count - 1) as f64,
    };
    let (start, direction) = path.at(0.0);
    Ok((1..count)
        .map(|i| {
            let distance = (step * i as f64).min(path.length);
            let (point, here) = path.at(distance);
            let turn = if align {
                crate::description::normalize_degrees(tidy((here - direction).to_degrees()))
            } else {
                0.0
            };
            Place {
                by: tidy3(sub(point, start)),
                turn: if turn >= 360.0 - 1e-9 { 0.0 } else { turn },
                about: tidy3(point),
                at: At::Along {
                    item: i,
                    distance: tidy(distance),
                },
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{PI, TAU};

    #[test]
    fn a_rectangular_array_counts_rows_up_and_columns_across_from_the_objects() {
        let places = rectangular(2, 3, 10.0, -5.0);
        let by: Vec<P3> = places.iter().map(|p| p.by).collect();
        assert_eq!(
            by,
            [
                [-5.0, 0.0, 0.0],
                [-10.0, 0.0, 0.0],
                [0.0, 10.0, 0.0],
                [-5.0, 10.0, 0.0],
                [-10.0, 10.0, 0.0]
            ]
        );
        assert_eq!(places[2].at, At::Grid { row: 1, column: 0 });
    }

    #[test]
    fn a_polar_array_spaces_a_whole_turn_evenly_and_a_part_to_its_end() {
        assert_eq!(polar_step(4, 360.0), 90.0);
        assert_eq!(polar_step(4, 180.0), 60.0);
        assert_eq!(polar_step(3, -90.0), -45.0);
        let turned = polar([1.0, 1.0, 0.0], 4, 360.0, true, [0.0; 3]);
        let turns: Vec<f64> = turned.iter().map(|p| p.turn).collect();
        assert_eq!(turns, [90.0, 180.0, 270.0]);
        assert!(turned
            .iter()
            .all(|p| p.by == [0.0; 3] && p.about == [1.0, 1.0, 0.0]));
        // Unturned, the base point goes round: from [2, 1] to [1, 2].
        let moved = polar([1.0, 1.0, 0.0], 4, 360.0, false, [2.0, 1.0, 0.0]);
        assert_eq!(moved[0].by, [-1.0, 1.0, 0.0]);
        assert_eq!(moved[1].by, [-2.0, 0.0, 0.0]);
        assert_eq!(moved[0].turn, 0.0);
    }

    #[test]
    fn a_path_array_spaces_items_evenly_or_at_a_spacing_and_turns_with_the_path() {
        // An L: 10 along X, then 10 up Y.
        let curves = [
            Curve::Segment {
                a: [0.0, 0.0, 0.0],
                b: [10.0, 0.0, 0.0],
            },
            Curve::Segment {
                a: [10.0, 0.0, 0.0],
                b: [10.0, 10.0, 0.0],
            },
        ];
        let even = along(&curves, 5, None, true).unwrap();
        let by: Vec<P3> = even.iter().map(|p| p.by).collect();
        assert_eq!(
            by,
            [
                [5.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 5.0, 0.0],
                [10.0, 10.0, 0.0]
            ]
        );
        let turns: Vec<f64> = even.iter().map(|p| p.turn).collect();
        // The corner takes the next segment's direction.
        assert_eq!(turns, [0.0, 90.0, 90.0, 90.0]);
        assert_eq!(even[1].about, [10.0, 0.0, 0.0]);
        let spaced = along(&curves, 3, Some(4.0), false).unwrap();
        assert_eq!(spaced[1].by, [8.0, 0.0, 0.0]);
        assert_eq!(spaced[1].turn, 0.0);
        assert_eq!(
            spaced[1].at,
            At::Along {
                item: 2,
                distance: 8.0
            }
        );
        let too_far = along(&curves, 3, Some(11.0), true).unwrap_err();
        assert!(
            too_far.contains("22 along the path, which is 20 long"),
            "{too_far}"
        );
    }

    #[test]
    fn an_item_where_two_segments_meet_takes_the_next_ones_direction_despite_rounding() {
        // 0.1 + 0.2 adds up to a hair past 0.3, where the item falls.
        let segment = |a: [f64; 2], b: [f64; 2]| Curve::Segment {
            a: [a[0], a[1], 0.0],
            b: [b[0], b[1], 0.0],
        };
        let curves = [
            segment([0.0, 0.0], [0.1, 0.0]),
            segment([0.1, 0.0], [0.1, 0.2]),
            segment([0.1, 0.2], [0.3, 0.2]),
        ];
        let places = along(&curves, 2, Some(0.3), true).unwrap();
        assert_eq!(places[0].turn, 0.0);
        let near = |a: P3, b: P3| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-12);
        assert!(
            near(places[0].about, [0.1, 0.2, 0.0]),
            "{:?}",
            places[0].about
        );
        assert!(near(places[0].by, [0.1, 0.2, 0.0]), "{:?}", places[0].by);
    }

    #[test]
    fn a_closed_path_spaces_its_items_round_it() {
        let circle = [Curve::Arc {
            center: [0.0, 0.0, 0.0],
            radius: 2.0,
            start: 0.0,
            sweep: TAU,
        }];
        let places = along(&circle, 4, None, true).unwrap();
        assert_eq!(places[0].by, [-2.0, 2.0, 0.0]);
        assert_eq!(places[1].by, [-4.0, 0.0, 0.0]);
        assert_eq!(places[0].turn, 90.0);
        assert_eq!(places[2].turn, 270.0);
        // Along an arc a quarter round, both ends are items.
        let arc = [Curve::Arc {
            center: [0.0, 0.0, 0.0],
            radius: 1.0,
            start: 0.0,
            sweep: PI / 2.0,
        }];
        let places = along(&arc, 2, None, false).unwrap();
        assert_eq!(places[0].by, [-1.0, 1.0, 0.0]);
    }
}
