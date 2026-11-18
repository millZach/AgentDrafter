//! The grid behind the drawing on the dark model space, as AutoCAD shows
//! one: minor lines and, every few of them, a stronger major line, which
//! the drafter turns on and off with Grid (F7).
//!
//! The spacing is a length on paper: a quarter inch with a line each inch
//! on a sheet drawn in inches or feet, and 10 mm with a line each 50 mm on
//! any other. Zoomed out so far that the major lines would crowd closer
//! than [`CLOSEST`], the grid steps out ten times, as often as it needs; the
//! minor lines are left out while they are closer than that. This is plain
//! arithmetic, tested natively; `web` draws the lines.

/// How close, in CSS pixels, grid lines may come before they are left out.
pub const CLOSEST: f64 = 8.0;

/// The grid at one zoom: its spacings in drawing units, and which are shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// Between minor lines, in drawing units.
    pub minor: f64,
    /// Between major lines, in drawing units: a whole number of minor ones.
    pub major: f64,
    /// Whether the minor lines are far enough apart to draw.
    pub minor_shown: bool,
}

/// The grid of a sheet whose drawing unit spans `millimetres_per_unit` of
/// paper, shown at `pixels_per_unit` CSS pixels to a drawing unit; none when
/// either is not a length.
pub fn grid(millimetres_per_unit: f64, pixels_per_unit: f64) -> Option<Grid> {
    if !(millimetres_per_unit.is_finite()
        && millimetres_per_unit > 0.0
        && pixels_per_unit.is_finite()
        && pixels_per_unit > 0.0)
    {
        return None;
    }
    // Inches and feet are both whole numbers of 25.4 mm.
    let in_inches = {
        let inches = millimetres_per_unit / 25.4;
        (inches - inches.round()).abs() < 1e-9 && inches.round() >= 1.0
    };
    let (minor_mm, major_mm) = if in_inches {
        (25.4 / 4.0, 25.4)
    } else {
        (10.0, 50.0)
    };
    let mut minor = minor_mm / millimetres_per_unit;
    let mut major = major_mm / millimetres_per_unit;
    // Stepped out ten times at a time, never past a reasonable bound.
    for _ in 0..12 {
        if major * pixels_per_unit >= CLOSEST {
            break;
        }
        minor *= 10.0;
        major *= 10.0;
    }
    if major * pixels_per_unit < CLOSEST {
        return None;
    }
    Some(Grid {
        minor,
        major,
        minor_shown: minor * pixels_per_unit >= CLOSEST,
    })
}

/// Where the first grid line at or after the canvas's edge falls, in device
/// pixels from it, for lines every `step` drawing units: `from` is the
/// drawing coordinate at the edge, `pixels` device pixels to a drawing unit,
/// and `up` true along Y, where the canvas counts down as the drawing counts
/// up. Worked out in 64 bits, so a drawing far from its origin keeps its
/// lines in place.
pub fn first_line(from: f64, step: f64, pixels: f64, up: bool) -> f64 {
    let into = if up {
        from.rem_euclid(step)
    } else {
        (-from).rem_euclid(step)
    };
    into * pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_in_inches_has_a_line_each_quarter_inch_and_a_stronger_one_each_inch() {
        let g = grid(25.4, 60.0).unwrap();
        assert_eq!((g.minor, g.major, g.minor_shown), (0.25, 1.0, true));
    }

    #[test]
    fn a_sheet_in_millimetres_has_a_line_each_10_mm_and_a_stronger_one_each_50() {
        let g = grid(1.0, 3.0).unwrap();
        assert_eq!((g.minor, g.major, g.minor_shown), (10.0, 50.0, true));
        // In metres, the same lengths of paper.
        let g = grid(1000.0, 3000.0).unwrap();
        assert!((g.minor - 0.01).abs() < 1e-12 && (g.major - 0.05).abs() < 1e-12);
    }

    #[test]
    fn minor_lines_too_close_together_are_left_out() {
        // A quarter inch at 20 px to the inch is 5 px.
        let g = grid(25.4, 20.0).unwrap();
        assert_eq!((g.minor, g.major, g.minor_shown), (0.25, 1.0, false));
    }

    #[test]
    fn zoomed_far_out_the_grid_steps_out_ten_times() {
        // An inch at 0.5 px: ten inches is 5 px, a hundred 50 px.
        let g = grid(25.4, 0.5).unwrap();
        assert_eq!((g.minor, g.major, g.minor_shown), (25.0, 100.0, true));
    }

    #[test]
    fn no_grid_without_a_length() {
        assert_eq!(grid(0.0, 10.0), None);
        assert_eq!(grid(25.4, 0.0), None);
        assert_eq!(grid(f64::NAN, 10.0), None);
    }

    #[test]
    fn the_first_line_is_found_from_the_edge_in_both_directions() {
        // Across: the edge at x = 0.1, lines each 0.25: the next at 0.25,
        // 0.15 on at 100 px to the unit.
        assert!((first_line(0.1, 0.25, 100.0, false) - 15.0).abs() < 1e-9);
        // Down from the top at y = 11.1: the next line down is at 11.0.
        assert!((first_line(11.1, 0.25, 100.0, true) - 10.0).abs() < 1e-9);
        // Far from the origin, still in place.
        assert!((first_line(1_000_000.1, 0.25, 100.0, false) - 15.0).abs() < 1e-3);
    }
}
