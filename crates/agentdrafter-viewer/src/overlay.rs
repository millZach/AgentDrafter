//! What the renderer draws over the drawing for the drafter's cursor, as
//! line segments in device pixels: AutoCAD's model-space crosshair, with
//! the pick box in its middle when no tool asks for a point, and the
//! AutoSnap marker of the snap the cursor caught, in its shape for its
//! kind. Plain computation, tested natively; `web` turns the segments into
//! triangles and draws them.

use agentdrafter_core::SnapKind;

/// A line segment, `[x0, y0, x1, y1]`, in device pixels from the canvas's
/// top left, y down.
pub type Segment = [f64; 4];

/// The crosshair's arms, from its middle, in CSS pixels, as the frame's
/// prototype draws AutoCAD's.
pub const ARM: f64 = 40.0;

/// Half the pick box's side, in CSS pixels.
pub const PICK_BOX: f64 = 5.0;

/// Half a marker's size, in CSS pixels.
pub const MARKER: f64 = 7.0;

/// The middle of the device pixel `v` falls in, so a line one pixel wide
/// lies on one row or column of pixels.
fn on_pixel(v: f64) -> f64 {
    v.floor() + 0.5
}

/// The crosshair at `(x, y)`, device pixels, `ratio` device pixels to a CSS
/// pixel: four arms, and with `pick_box` a gap round the middle and the
/// pick box there.
pub fn crosshair(x: f64, y: f64, ratio: f64, pick_box: bool) -> Vec<Segment> {
    let (x, y) = (on_pixel(x), on_pixel(y));
    let arm = ARM * ratio;
    let gap = if pick_box { PICK_BOX * ratio } else { 0.0 };
    let mut lines = vec![
        [x - arm, y, x - gap, y],
        [x + gap, y, x + arm, y],
        [x, y - arm, x, y - gap],
        [x, y + gap, x, y + arm],
    ];
    if pick_box {
        let b = PICK_BOX * ratio;
        lines.extend(closed(&[
            [x - b, y - b],
            [x + b, y - b],
            [x + b, y + b],
            [x - b, y + b],
        ]));
    }
    lines
}

fn closed(points: &[[f64; 2]]) -> Vec<Segment> {
    (0..points.len())
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            [a[0], a[1], b[0], b[1]]
        })
        .collect()
}

fn open(points: &[[f64; 2]]) -> Vec<Segment> {
    points
        .windows(2)
        .map(|w| [w[0][0], w[0][1], w[1][0], w[1][1]])
        .collect()
}

fn circle(cx: f64, cy: f64, r: f64) -> Vec<Segment> {
    let points: Vec<[f64; 2]> = (0..24)
        .map(|i| {
            let a = f64::from(i) * std::f64::consts::TAU / 24.0;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    closed(&points)
}

/// The AutoSnap marker of `kind` round `(x, y)`, device pixels, `ratio`
/// device pixels to a CSS pixel: a square for an endpoint, a triangle for
/// a midpoint, a circle for a centre, a diamond for a quadrant, an X for an
/// intersection, two squares for an insertion point, a circle with an X for
/// a node, an hourglass for the nearest point.
pub fn marker(kind: SnapKind, x: f64, y: f64, ratio: f64) -> Vec<Segment> {
    let (x, y) = (on_pixel(x), on_pixel(y));
    let s = MARKER * ratio;
    let at = |dx: f64, dy: f64| [x + dx * s, y + dy * s];
    let cross = |r: f64| {
        vec![
            [x - r * s, y - r * s, x + r * s, y + r * s],
            [x + r * s, y - r * s, x - r * s, y + r * s],
        ]
    };
    match kind {
        SnapKind::Endpoint => closed(&[at(-1.0, -1.0), at(1.0, -1.0), at(1.0, 1.0), at(-1.0, 1.0)]),
        SnapKind::Midpoint => closed(&[at(0.0, -1.0), at(1.0, 0.8), at(-1.0, 0.8)]),
        SnapKind::Center => circle(x, y, s),
        SnapKind::Quadrant => closed(&[at(0.0, -1.0), at(1.0, 0.0), at(0.0, 1.0), at(-1.0, 0.0)]),
        SnapKind::Insertion => {
            let mut lines = closed(&[at(-1.0, -0.2), at(0.2, -0.2), at(0.2, 1.0), at(-1.0, 1.0)]);
            lines.extend(open(&[
                at(-0.2, -0.2),
                at(-0.2, -1.0),
                at(1.0, -1.0),
                at(1.0, 0.2),
                at(0.2, 0.2),
            ]));
            lines
        }
        SnapKind::Node => {
            let mut lines = circle(x, y, s);
            lines.extend(cross(0.7));
            lines
        }
        SnapKind::Nearest => closed(&[at(-1.0, -1.0), at(1.0, -1.0), at(-1.0, 1.0), at(1.0, 1.0)]),
        _ => cross(1.0),
    }
}

/// The triangles of `lines`, each `width` device pixels wide with square
/// ends, so the corners of a closed shape are whole: six points to each,
/// `[x, y]` in device pixels.
pub fn triangles(lines: &[Segment], width: f64) -> Vec<f32> {
    let half = width / 2.0;
    let mut out = Vec::with_capacity(lines.len() * 12);
    for &[x0, y0, x1, y1] in lines {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let length = dx.hypot(dy);
        let (ux, uy) = if length > 0.0 {
            (dx / length, dy / length)
        } else {
            (1.0, 0.0)
        };
        // Along by half the width past each end; across by half the width.
        let (ax, ay) = (ux * half, uy * half);
        let (nx, ny) = (-uy * half, ux * half);
        let a = [x0 - ax, y0 - ay];
        let b = [x1 + ax, y1 + ay];
        let corners = [
            [a[0] + nx, a[1] + ny],
            [a[0] - nx, a[1] - ny],
            [b[0] + nx, b[1] + ny],
            [b[0] - nx, b[1] - ny],
        ];
        for i in [0, 1, 2, 2, 1, 3] {
            out.push(corners[i][0] as f32);
            out.push(corners[i][1] as f32);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crosshair_has_four_arms_and_the_pick_box() {
        let lines = crosshair(100.2, 50.7, 1.0, true);
        assert_eq!(lines.len(), 8);
        // Centred on the pixel the cursor is in; arms 40 px; a gap of 5.
        assert_eq!(lines[0], [60.5, 50.5, 95.5, 50.5]);
        assert_eq!(lines[3], [100.5, 55.5, 100.5, 90.5]);
        // Without the pick box, the arms meet in the middle.
        let bare = crosshair(100.0, 50.0, 2.0, false);
        assert_eq!(bare.len(), 4);
        assert_eq!(bare[0], [20.5, 50.5, 100.5, 50.5]);
    }

    #[test]
    fn each_snap_has_its_marker_shape() {
        assert_eq!(marker(SnapKind::Endpoint, 10.0, 10.0, 1.0).len(), 4);
        assert_eq!(marker(SnapKind::Midpoint, 10.0, 10.0, 1.0).len(), 3);
        assert_eq!(marker(SnapKind::Quadrant, 10.0, 10.0, 1.0).len(), 4);
        assert_eq!(marker(SnapKind::Intersection, 10.0, 10.0, 1.0).len(), 2);
        assert_eq!(marker(SnapKind::Center, 10.0, 10.0, 1.0).len(), 24);
        assert_eq!(marker(SnapKind::Node, 10.0, 10.0, 1.0).len(), 26);
        assert_eq!(marker(SnapKind::Insertion, 10.0, 10.0, 1.0).len(), 8);
        // The endpoint's square is 14 CSS pixels across, twice at 2 device
        // pixels to one.
        let square = marker(SnapKind::Endpoint, 10.0, 10.0, 2.0);
        assert_eq!(square[0], [-3.5, -3.5, 24.5, -3.5]);
    }

    #[test]
    fn a_line_is_two_triangles_with_square_ends() {
        let t = triangles(&[[0.0, 0.0, 10.0, 0.0]], 2.0);
        assert_eq!(t.len(), 12);
        let xs: Vec<f32> = t.chunks(2).map(|p| p[0]).collect();
        let ys: Vec<f32> = t.chunks(2).map(|p| p[1]).collect();
        assert_eq!(xs.iter().copied().fold(f32::INFINITY, f32::min), -1.0);
        assert_eq!(xs.iter().copied().fold(f32::NEG_INFINITY, f32::max), 11.0);
        assert_eq!(ys.iter().copied().fold(f32::INFINITY, f32::min), -1.0);
        assert_eq!(ys.iter().copied().fold(f32::NEG_INFINITY, f32::max), 1.0);
    }
}
