//! Drawing routines for Rank 40 2D objects: `point` (`PDMODE`/`PDSIZE`),
//! `xline`, `ray`, `mline` (`MLSTYLE`), `tolerance` (GD&T feature control
//! frame), `center_mark`, `center_line`, and `shape` (SHX shape files).
//!
//! Pure computation (no filesystem/GPU/clock), WASM32-safe (ADR-0011).

use std::f64::consts::PI;

use super::{
    add_stroke, curves, flat, linetype, text, Bounds, Color, DisplayList, Drawer, Drawn, FillRule,
    Pen, Placed, Point2, Shape, Shown,
};
use crate::description::sin_cos_degrees;
use crate::model::{
    CenterLine, CenterMark, DimensionStyle, MLine, MLineStyle, ObjectColor, ObjectLinetype, Point,
    Ray, ShxShape, Tolerance, XLine,
};
use crate::text::{Justify, Text, STANDARD};

/// Draws a [`Point`] according to the sheet's `pdmode` and `pdsize`.
pub(super) fn draw_point(
    pt: &Point,
    pdmode: i16,
    pdsize: f64,
    view_height: f64,
    pen: Pen<'_>,
    tolerance: f64,
    placed: Option<&Placed>,
) -> Drawn {
    let p = flat(pt.position);
    let place = |q: Point2| match placed {
        Some(pl) => pl.point(q),
        None => q,
    };
    let placed_center = place(p);
    let base = pdmode & 0x1F;
    let ring = pdmode & !0x1F;
    let size = if pdsize > 0.0 {
        pdsize
    } else if pdsize < 0.0 {
        (-pdsize / 100.0) * view_height.max(1.0)
    } else {
        0.05 * view_height.max(1.0)
    };
    let half = size * 0.5;
    let ring_r = half * 0.5;
    let (sin, cos) = sin_cos_degrees(pt.angle);
    let local = |lx: f64, ly: f64| -> Point2 {
        place([p[0] + lx * cos - ly * sin, p[1] + lx * sin + ly * cos])
    };

    let mut runs: Vec<Vec<Point2>> = Vec::new();
    match base {
        0 => runs.push(vec![placed_center]),
        1 => {} // invisible point
        2 => {
            // Plus (+)
            runs.push(vec![local(-half, 0.0), local(half, 0.0)]);
            runs.push(vec![local(0.0, -half), local(0.0, half)]);
        }
        3 => {
            // Cross (X)
            let d = half * std::f64::consts::FRAC_1_SQRT_2;
            runs.push(vec![local(-d, -d), local(d, d)]);
            runs.push(vec![local(-d, d), local(d, -d)]);
        }
        4 => {
            // Vertical tick (|)
            runs.push(vec![local(0.0, 0.0), local(0.0, ring_r)]);
        }
        _ => runs.push(vec![placed_center]),
    }

    if (ring & 32) != 0 && ring_r > 1e-9 {
        let mut circ: Vec<Point2> = curves::circle([0.0, 0.0], ring_r, tolerance)
            .into_iter()
            .map(|[x, y]| local(x, y))
            .collect();
        if let Some(&first) = circ.first() {
            if circ.last() != Some(&first) {
                circ.push(first);
            }
        }
        runs.push(circ);
    }
    if (ring & 64) != 0 && ring_r > 1e-9 {
        runs.push(vec![
            local(-ring_r, -ring_r),
            local(ring_r, -ring_r),
            local(ring_r, ring_r),
            local(-ring_r, ring_r),
            local(-ring_r, -ring_r),
        ]);
    }

    if runs.is_empty() {
        Drawn {
            reach: vec![placed_center],
            ..Drawn::default()
        }
    } else {
        Drawn {
            shapes: vec![Shape::Stroke {
                lineweight: pen.lineweight,
                runs,
            }],
            reach: vec![placed_center],
            ..Drawn::default()
        }
    }
}

/// Clips an infinite line `p + t * d` (`t_min..=t_max`) against `bounds` using
/// Liang-Barsky, returning the clipped segment `[a, b]` if it intersects.
fn clip_parametric_line(
    p: Point2,
    d: Point2,
    mut t_min: f64,
    mut t_max: f64,
    bounds: Bounds,
) -> Option<[Point2; 2]> {
    let checks = [
        (-d[0], p[0] - bounds.min[0]),
        (d[0], bounds.max[0] - p[0]),
        (-d[1], p[1] - bounds.min[1]),
        (d[1], bounds.max[1] - p[1]),
    ];
    for (denom, numer) in checks {
        if denom.abs() <= 1e-12 {
            if numer < 0.0 {
                return None;
            }
        } else {
            let t = numer / denom;
            if denom < 0.0 {
                if t > t_min {
                    t_min = t;
                }
            } else if t < t_max {
                t_max = t;
            }
            if t_min > t_max {
                return None;
            }
        }
    }
    if !t_min.is_finite() || !t_max.is_finite() || (t_max - t_min).abs() <= 1e-12 {
        return None;
    }
    Some([
        [p[0] + t_min * d[0], p[1] + t_min * d[1]],
        [p[0] + t_max * d[0], p[1] + t_max * d[1]],
    ])
}

/// Draws an [`XLine`] clipped cleanly to `clip_bounds`.
pub(super) fn draw_xline(
    xl: &XLine,
    clip_bounds: Bounds,
    pen: Pen<'_>,
    placed: Option<&Placed>,
) -> Drawn {
    let p0 = flat(xl.through);
    let u = xl.unit_direction();
    let p1 = [p0[0] + u.x, p0[1] + u.y];
    let (bp, dir) = match placed {
        Some(pl) => {
            let a = pl.point(p0);
            let b = pl.point(p1);
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let len = dx.hypot(dy);
            if len <= 1e-12 {
                return Drawn::default();
            }
            (a, [dx / len, dy / len])
        }
        None => {
            let len = u.x.hypot(u.y);
            if len <= 1e-12 {
                return Drawn::default();
            }
            (p0, [u.x / len, u.y / len])
        }
    };
    let Some(seg) = clip_parametric_line(bp, dir, f64::NEG_INFINITY, f64::INFINITY, clip_bounds)
    else {
        return Drawn::default();
    };
    let dashes = pen.dashes();
    let runs = linetype::along(&seg, dashes.as_deref(), false);
    Drawn {
        shapes: if runs.is_empty() {
            Vec::new()
        } else {
            vec![Shape::Stroke {
                lineweight: pen.lineweight,
                runs,
            }]
        },
        ..Drawn::default()
    }
}

/// Draws a [`Ray`] clipped cleanly to `clip_bounds`.
pub(super) fn draw_ray(
    ray: &Ray,
    clip_bounds: Bounds,
    pen: Pen<'_>,
    placed: Option<&Placed>,
) -> Drawn {
    let p0 = flat(ray.from);
    let u = ray.unit_direction();
    let p1 = [p0[0] + u.x, p0[1] + u.y];
    let (bp, dir) = match placed {
        Some(pl) => {
            let a = pl.point(p0);
            let b = pl.point(p1);
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let len = dx.hypot(dy);
            if len <= 1e-12 {
                return Drawn::default();
            }
            (a, [dx / len, dy / len])
        }
        None => {
            let len = u.x.hypot(u.y);
            if len <= 1e-12 {
                return Drawn::default();
            }
            (p0, [u.x / len, u.y / len])
        }
    };
    let Some(seg) = clip_parametric_line(bp, dir, 0.0, f64::INFINITY, clip_bounds) else {
        return Drawn::default();
    };
    let dashes = pen.dashes();
    let runs = linetype::along(&seg, dashes.as_deref(), false);
    Drawn {
        shapes: if runs.is_empty() {
            Vec::new()
        } else {
            vec![Shape::Stroke {
                lineweight: pen.lineweight,
                runs,
            }]
        },
        ..Drawn::default()
    }
}

/// Computes the 2D mitered offset chains for an [`MLine`] and its [`MLineStyle`].
/// Returns `(element_chains, min_idx, max_idx)` where `element_chains[e][v]` is
/// the 2D point of element `e` at vertex `v`.
pub(crate) fn mline_geometry(
    mline: &MLine,
    style: &MLineStyle,
) -> (Vec<Vec<Point2>>, usize, usize) {
    let n = mline.points.len();
    if n < 2 || style.elements.is_empty() {
        return (Vec::new(), 0, 0);
    }
    let pts: Vec<Point2> = mline.points.iter().map(|&p| flat(p)).collect();
    let closed = mline.closed && n >= 3;

    let seg_dir = |a: Point2, b: Point2| -> Option<Point2> {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = dx.hypot(dy);
        (len > 1e-12).then_some([dx / len, dy / len])
    };
    let perp_left = |d: Point2| -> Point2 { [-d[1], d[0]] };

    // Compute per-vertex miter direction and miter scale factor `1 / (perp · miter)`.
    let mut miters: Vec<(Point2, f64)> = Vec::with_capacity(n);
    for i in 0..n {
        let prev = if i > 0 {
            seg_dir(pts[i - 1], pts[i])
        } else if closed {
            seg_dir(pts[n - 1], pts[0])
        } else {
            None
        };
        let next = if i + 1 < n {
            seg_dir(pts[i], pts[i + 1])
        } else if closed {
            seg_dir(pts[n - 1], pts[0])
        } else {
            None
        };
        let (miter, factor) = match (prev, next) {
            (Some(d1), Some(d2)) => {
                let n1 = perp_left(d1);
                let n2 = perp_left(d2);
                let sx = n1[0] + n2[0];
                let sy = n1[1] + n2[1];
                let slen = sx.hypot(sy);
                if slen > 1e-9 {
                    let m = [sx / slen, sy / slen];
                    let dot = (n2[0] * m[0] + n2[1] * m[1]).clamp(-1.0, 1.0);
                    let f = if dot.abs() > 0.1 { 1.0 / dot } else { 10.0 };
                    (m, f)
                } else {
                    (n2, 1.0)
                }
            }
            (None, Some(d2)) => {
                // Start cap angle (degrees from segment direction, default 90°).
                let n2 = perp_left(d2);
                let ang = style.start_angle.to_radians();
                let (sin, cos) = ang.sin_cos();
                let m = [d2[0] * cos - d2[1] * sin, d2[0] * sin + d2[1] * cos];
                let dot = n2[0] * m[0] + n2[1] * m[1];
                let f = if dot.abs() > 0.1 { 1.0 / dot } else { 1.0 };
                (m, f)
            }
            (Some(d1), None) => {
                let n1 = perp_left(d1);
                let ang = style.end_angle.to_radians();
                let (sin, cos) = ang.sin_cos();
                let m = [d1[0] * cos - d1[1] * sin, d1[0] * sin + d1[1] * cos];
                let dot = n1[0] * m[0] + n1[1] * m[1];
                let f = if dot.abs() > 0.1 { 1.0 / dot } else { 1.0 };
                (m, f)
            }
            (None, None) => ([0.0, 1.0], 1.0),
        };
        miters.push((miter, factor));
    }

    let mut min_idx = 0usize;
    let mut max_idx = 0usize;
    let mut min_off = f64::INFINITY;
    let mut max_off = f64::NEG_INFINITY;
    let mut chains = Vec::with_capacity(style.elements.len());
    for (ei, elem) in style.elements.iter().enumerate() {
        let eff = mline.effective_offset(elem.offset, style);
        if eff < min_off {
            min_off = eff;
            min_idx = ei;
        }
        if eff > max_off {
            max_off = eff;
            max_idx = ei;
        }
        let mut chain = Vec::with_capacity(n);
        for (vi, &p) in pts.iter().enumerate() {
            let (m, f) = miters[vi];
            let dist = eff * f;
            chain.push([p[0] + m[0] * dist, p[1] + m[1] * dist]);
        }
        chains.push(chain);
    }
    (chains, min_idx, max_idx)
}

/// Draws a semicircular cap arc from `a` to `b` bulging outward along `outward`.
fn cap_arc(a: Point2, b: Point2, outward: Point2, tolerance: f64) -> Vec<Point2> {
    let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let dx = a[0] - mid[0];
    let dy = a[1] - mid[1];
    let r = dx.hypot(dy);
    if r <= 1e-9 {
        return vec![a, b];
    }
    let start_ang = dy.atan2(dx);
    // Test midpoint of CCW vs CW semicircle against `outward`.
    let ccw_mid = start_ang + PI * 0.5;
    let dot = ccw_mid.cos() * outward[0] + ccw_mid.sin() * outward[1];
    let sweep = if dot >= 0.0 { PI } else { -PI };
    let steps = curves::arc_steps(r, sweep.abs(), tolerance).max(8);
    (0..=steps)
        .map(|k| {
            let ang = start_ang + sweep * (k as f64 / steps as f64);
            [mid[0] + r * ang.cos(), mid[1] + r * ang.sin()]
        })
        .collect()
}

/// Draws an [`MLine`] directly onto `list` (supporting per-element colours,
/// linetypes, fill polygon, joints, and caps).
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_mline(
    drawer: &Drawer<'_>,
    index: usize,
    path: &[usize],
    mline: &MLine,
    base_layer: &str,
    by_block: Option<Pen<'_>>,
    tolerance: f64,
    placed: Option<&Placed>,
    list: &mut DisplayList,
) {
    let shown = drawer.shown(base_layer);
    if shown == Shown::Hidden {
        return;
    }
    let pen = drawer.pen(base_layer, &mline.properties, by_block);
    let style = drawer
        .sheet
        .mline_style(&mline.style)
        .unwrap_or_else(MLineStyle::standard);
    let (mut chains, min_idx, max_idx) = mline_geometry(mline, &style);
    if chains.is_empty() {
        return;
    }
    if let Some(pl) = placed {
        for chain in &mut chains {
            for p in chain.iter_mut() {
                *p = pl.point(*p);
            }
        }
    }
    let n = chains[0].len();
    let closed = mline.closed && n >= 3;

    // 1. Optional background fill between outermost chains.
    if let Some(fill_col) = style.fill_color {
        if min_idx != max_idx {
            let fill_color = match fill_col {
                ObjectColor::ByLayer => drawer.layer_pen(base_layer).color,
                ObjectColor::ByBlock => pen.color,
                ObjectColor::Index(i) => Color(i),
            };
            let fill_pen = Pen {
                color: fill_color,
                ..pen
            };
            let mut contours = Vec::new();
            let seg_count = if closed { n } else { n - 1 };
            for i in 0..seg_count {
                let j = (i + 1) % n;
                contours.push(vec![
                    chains[min_idx][i],
                    chains[min_idx][j],
                    chains[max_idx][j],
                    chains[max_idx][i],
                ]);
            }
            let drawn = Drawn {
                shapes: vec![Shape::Fill {
                    rule: FillRule::EvenOdd,
                    contours,
                }],
                ..Drawn::default()
            };
            list.add_shown(shown, index, path, fill_pen, drawn);
        }
    }

    // 2. Parallel element lines, each in its own colour and linetype.
    let by_layer = drawer.layer_pen(base_layer);
    for (ei, elem) in style.elements.iter().enumerate() {
        let elem_color = match elem.color {
            ObjectColor::ByLayer => by_layer.color,
            ObjectColor::ByBlock => pen.color,
            ObjectColor::Index(i) => Color(i),
        };
        let elem_pattern = match &elem.linetype {
            ObjectLinetype::ByLayer => by_layer.pattern,
            ObjectLinetype::ByBlock => pen.pattern,
            ObjectLinetype::Named(name) => drawer.pattern(name),
        };
        let elem_pen = Pen {
            color: elem_color,
            pattern: elem_pattern,
            ..pen
        };
        let mut pts = chains[ei].clone();
        if closed {
            pts.push(pts[0]);
        }
        let dashes = elem_pen.dashes();
        let runs = linetype::along(&pts, dashes.as_deref(), closed);
        let mut drawn = Drawn::default();
        add_stroke(&mut drawn, elem_pen.lineweight, runs);
        list.add_shown(shown, index, path, elem_pen, drawn);
    }

    // 3. Joints and caps (drawn in the multiline's own pen).
    let mut cap_runs: Vec<Vec<Point2>> = Vec::new();
    if style.display_joints && min_idx != max_idx {
        let start = usize::from(!closed);
        let end = if closed { n } else { n.saturating_sub(1) };
        for (&a, &b) in chains[min_idx][start..end]
            .iter()
            .zip(&chains[max_idx][start..end])
        {
            cap_runs.push(vec![a, b]);
        }
    }
    if !closed && min_idx != max_idx {
        // Order elements by effective offset for inner-arc pairing.
        let mut order: Vec<usize> = (0..style.elements.len()).collect();
        order.sort_by(|&a, &b| {
            let oa = mline.effective_offset(style.elements[a].offset, &style);
            let ob = mline.effective_offset(style.elements[b].offset, &style);
            oa.partial_cmp(&ob).unwrap_or(std::cmp::Ordering::Equal)
        });

        let start_out = {
            let dx = chains[min_idx][0][0] - chains[min_idx][1][0];
            let dy = chains[min_idx][0][1] - chains[min_idx][1][1];
            let l = dx.hypot(dy).max(1e-12);
            [dx / l, dy / l]
        };
        if !mline.suppress_start_caps {
            if style.start_square_cap {
                cap_runs.push(vec![chains[min_idx][0], chains[max_idx][0]]);
            }
            if style.start_round_cap {
                cap_runs.push(cap_arc(
                    chains[min_idx][0],
                    chains[max_idx][0],
                    start_out,
                    tolerance,
                ));
            }
            if style.start_inner_arcs_cap && order.len() >= 4 {
                let pairs = (order.len() - 2) / 2;
                for k in 0..pairs {
                    let lo = order[1 + k];
                    let hi = order[order.len() - 2 - k];
                    cap_runs.push(cap_arc(chains[lo][0], chains[hi][0], start_out, tolerance));
                }
            }
        }

        let last = n - 1;
        let end_out = {
            let dx = chains[min_idx][last][0] - chains[min_idx][last - 1][0];
            let dy = chains[min_idx][last][1] - chains[min_idx][last - 1][1];
            let l = dx.hypot(dy).max(1e-12);
            [dx / l, dy / l]
        };
        if !mline.suppress_end_caps {
            if style.end_square_cap {
                cap_runs.push(vec![chains[min_idx][last], chains[max_idx][last]]);
            }
            if style.end_round_cap {
                cap_runs.push(cap_arc(
                    chains[min_idx][last],
                    chains[max_idx][last],
                    end_out,
                    tolerance,
                ));
            }
            if style.end_inner_arcs_cap && order.len() >= 4 {
                let pairs = (order.len() - 2) / 2;
                for k in 0..pairs {
                    let lo = order[1 + k];
                    let hi = order[order.len() - 2 - k];
                    cap_runs.push(cap_arc(
                        chains[lo][last],
                        chains[hi][last],
                        end_out,
                        tolerance,
                    ));
                }
            }
        }
    }

    if !cap_runs.is_empty() {
        let mut drawn = Drawn::default();
        add_stroke(&mut drawn, pen.lineweight, cap_runs);
        list.add_shown(shown, index, path, pen, drawn);
    }
}

/// One piece inside a feature control frame compartment.
#[derive(Clone, Debug)]
enum FcfToken {
    Symbol(char),
    Text(String),
}

/// Parses an AutoCAD TOLERANCE string (`^J` / `\n` rows, `%%v` compartment
/// dividers, `{\Fgdt;<letter>}` symbols) into rows of compartments.
fn parse_fcf(raw: &str) -> Vec<Vec<Vec<FcfToken>>> {
    let normalized = raw.replace("^J", "\n").replace("\\P", "\n");
    normalized
        .split('\n')
        .map(|row| {
            let mut cells: Vec<Vec<FcfToken>> = row.split("%%v").map(parse_fcf_cell).collect();
            // Trim empty trailing separator cells produced by AutoCAD's trailing `%%v%%v`.
            while cells.len() > 1 && cells.last().is_some_and(Vec::is_empty) {
                cells.pop();
            }
            cells
        })
        .collect()
}

fn parse_fcf_cell(cell: &str) -> Vec<FcfToken> {
    let mut tokens = Vec::new();
    let mut rest = cell;
    let mut buf = String::new();
    let flush = |buf: &mut String, tokens: &mut Vec<FcfToken>| {
        if !buf.is_empty() {
            tokens.push(FcfToken::Text(std::mem::take(buf)));
        }
    };
    while !rest.is_empty() {
        if let Some(after) = rest
            .strip_prefix("{\\Fgdt;")
            .or_else(|| rest.strip_prefix("{\\fgdt;"))
        {
            if let Some(close) = after.find('}') {
                flush(&mut buf, &mut tokens);
                for ch in after[..close].chars() {
                    if !ch.is_whitespace() {
                        tokens.push(FcfToken::Symbol(ch.to_ascii_lowercase()));
                    }
                }
                rest = &after[close + 1..];
                continue;
            }
        }
        let mut it = rest.chars();
        if let Some(c) = it.next() {
            buf.push(c);
            rest = it.as_str();
        }
    }
    flush(&mut buf, &mut tokens);
    tokens
}

/// Draws a standard GD&T symbol inside `[0, h] x [0, h]` (unit height `h`)
/// in local frame coordinates, returning stroke polylines.
fn gdt_symbol_strokes(sym: char, x0: f64, y0: f64, h: f64, tolerance: f64) -> Vec<Vec<Point2>> {
    let pt = |u: f64, v: f64| -> Point2 { [x0 + u * h, y0 + v * h] };
    let circle_at = |cu: f64, cv: f64, r: f64| -> Vec<Point2> {
        let mut c = curves::circle(pt(cu, cv), r * h, tolerance);
        if let Some(&first) = c.first() {
            if c.last() != Some(&first) {
                c.push(first);
            }
        }
        c
    };
    match sym {
        // Position (target crosshair)
        'j' => vec![
            circle_at(0.5, 0.5, 0.30),
            vec![pt(0.05, 0.5), pt(0.95, 0.5)],
            vec![pt(0.5, 0.05), pt(0.5, 0.95)],
        ],
        // Diameter (∅)
        'n' => vec![
            circle_at(0.5, 0.5, 0.32),
            vec![pt(0.22, 0.12), pt(0.78, 0.88)],
        ],
        // Perpendicularity (⊥)
        'b' => vec![
            vec![pt(0.1, 0.15), pt(0.9, 0.15)],
            vec![pt(0.5, 0.15), pt(0.5, 0.88)],
        ],
        // Parallelism (//)
        'f' => vec![
            vec![pt(0.2, 0.15), pt(0.45, 0.85)],
            vec![pt(0.55, 0.15), pt(0.8, 0.85)],
        ],
        // Angularity (∠)
        'a' => vec![vec![pt(0.85, 0.78), pt(0.12, 0.18), pt(0.88, 0.18)]],
        // Flatness (parallelogram)
        'c' => vec![vec![
            pt(0.10, 0.25),
            pt(0.68, 0.25),
            pt(0.90, 0.75),
            pt(0.32, 0.75),
            pt(0.10, 0.25),
        ]],
        // Circularity (○)
        'e' => vec![circle_at(0.5, 0.5, 0.36)],
        // Concentricity (◎)
        'r' => vec![circle_at(0.5, 0.5, 0.38), circle_at(0.5, 0.5, 0.18)],
        // Cylindricity
        'g' => vec![
            circle_at(0.5, 0.5, 0.26),
            vec![pt(0.12, 0.18), pt(0.32, 0.82)],
            vec![pt(0.68, 0.18), pt(0.88, 0.82)],
        ],
        // Straightness (—)
        'u' => vec![vec![pt(0.1, 0.5), pt(0.9, 0.5)]],
        // Symmetry (≡)
        'i' => vec![
            vec![pt(0.22, 0.75), pt(0.78, 0.75)],
            vec![pt(0.10, 0.50), pt(0.90, 0.50)],
            vec![pt(0.22, 0.25), pt(0.78, 0.25)],
        ],
        // Profile of a line (⌒)
        'k' => {
            let mut arc = curves::arc(pt(0.5, 0.25), 0.36 * h, 0.0, 180.0, tolerance);
            if arc.is_empty() {
                arc = vec![pt(0.14, 0.25), pt(0.86, 0.25)];
            }
            vec![arc]
        }
        // Profile of a surface (⌓)
        'd' => {
            let mut arc = curves::arc(pt(0.5, 0.25), 0.36 * h, 0.0, 180.0, tolerance);
            arc.push(pt(0.86, 0.25));
            arc.push(pt(0.14, 0.25));
            vec![arc]
        }
        // Runout (↗)
        'h' => vec![
            vec![pt(0.22, 0.15), pt(0.76, 0.82)],
            vec![pt(0.54, 0.76), pt(0.76, 0.82), pt(0.72, 0.58)],
        ],
        // Total runout (↗↗)
        't' => vec![
            vec![pt(0.12, 0.15), pt(0.88, 0.15)],
            vec![pt(0.20, 0.15), pt(0.48, 0.82)],
            vec![pt(0.32, 0.76), pt(0.48, 0.82), pt(0.48, 0.62)],
            vec![pt(0.55, 0.15), pt(0.83, 0.82)],
            vec![pt(0.67, 0.76), pt(0.83, 0.82), pt(0.83, 0.62)],
        ],
        // MMC (Ⓜ), LMC (Ⓛ), RFS (Ⓢ), Projected tolerance zone (Ⓟ)
        'm' => vec![
            circle_at(0.5, 0.5, 0.42),
            vec![
                pt(0.28, 0.28),
                pt(0.28, 0.72),
                pt(0.50, 0.48),
                pt(0.72, 0.72),
                pt(0.72, 0.28),
            ],
        ],
        'l' => vec![
            circle_at(0.5, 0.5, 0.42),
            vec![pt(0.36, 0.72), pt(0.36, 0.28), pt(0.68, 0.28)],
        ],
        's' => vec![
            circle_at(0.5, 0.5, 0.42),
            vec![
                pt(0.66, 0.68),
                pt(0.38, 0.68),
                pt(0.38, 0.50),
                pt(0.64, 0.50),
                pt(0.64, 0.30),
                pt(0.34, 0.30),
            ],
        ],
        'p' => vec![
            circle_at(0.5, 0.5, 0.42),
            vec![
                pt(0.36, 0.26),
                pt(0.36, 0.74),
                pt(0.62, 0.74),
                pt(0.66, 0.64),
                pt(0.62, 0.52),
                pt(0.36, 0.52),
            ],
        ],
        // Fallback small diamond
        _ => vec![vec![
            pt(0.5, 0.15),
            pt(0.85, 0.5),
            pt(0.5, 0.85),
            pt(0.15, 0.5),
            pt(0.5, 0.15),
        ]],
    }
}

/// Draws a geometric [`Tolerance`] feature control frame.
pub(super) fn draw_tolerance(
    drawer: &Drawer<'_>,
    tol: &Tolerance,
    pen: Pen<'_>,
    tolerance: f64,
    placed: Option<&Placed>,
) -> Drawn {
    let dim_style = drawer
        .sheet
        .dimension_style(&tol.dimension_style)
        .or_else(|| DimensionStyle::built_in(crate::model::ISO_DIMENSION_STYLE, drawer.sheet.units))
        .or_else(|| {
            DimensionStyle::built_in(crate::model::STANDARD_DIMENSION_STYLE, drawer.sheet.units)
        })
        .unwrap_or_else(|| {
            DimensionStyle::built_in(
                crate::model::STANDARD_DIMENSION_STYLE,
                crate::units::Unit::In,
            )
            .expect("Standard imperial dimension style exists")
        });
    let text_h = tol
        .text_height
        .unwrap_or_else(|| (dim_style.text_height * dim_style.scale).max(0.1));
    let style_name = if drawer
        .sheet
        .text_styles
        .iter()
        .any(|s| s.name == dim_style.text_style)
    {
        dim_style.text_style.as_str()
    } else {
        STANDARD
    };
    let rows = parse_fcf(&tol.text);
    if rows.is_empty() {
        return Drawn::default();
    }

    let pad_x = text_h * 0.45;
    let row_h = text_h * 2.0;
    let sym_advance = text_h * 1.15;

    let measure_text = |s: &str| -> f64 {
        let probe = Text {
            contents: s.to_owned(),
            position: crate::model::Point3::new(0.0, 0.0, 0.0),
            height: text_h,
            style: style_name.to_owned(),
            width_factor: 1.0,
            rotation: 0.0,
            justify: Justify::Left,
            underline: false,
            overline: false,
            field: None,
            annotative: false,
            annotation_scales: Vec::new(),
            layer: tol.layer.clone(),
            properties: tol.properties.clone(),
        };
        let l = text::draw_text(&probe, drawer.fonts, pen.lineweight, tolerance);
        l.runs
            .first()
            .map_or(s.chars().count() as f64 * text_h * 0.65, |r| {
                r.across[0].hypot(r.across[1])
            })
    };

    // Measure every row's cell widths.
    let mut row_widths: Vec<Vec<f64>> = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut widths = Vec::with_capacity(row.len());
        for cell in row {
            let mut w = 0.0;
            for tok in cell {
                match tok {
                    FcfToken::Symbol(_) => w += sym_advance,
                    FcfToken::Text(s) => w += measure_text(s),
                }
            }
            widths.push((w + 2.0 * pad_x).max(row_h));
        }
        row_widths.push(widths);
    }

    let (sin, cos) = sin_cos_degrees(tol.direction);
    let base = flat(tol.position);
    let to_world = |lx: f64, ly: f64| -> Point2 {
        let w = [base[0] + lx * cos - ly * sin, base[1] + lx * sin + ly * cos];
        match placed {
            Some(pl) => pl.point(w),
            None => w,
        }
    };

    let mut frame_runs: Vec<Vec<Point2>> = Vec::new();
    let mut drawn = Drawn::default();

    for (ri, (row, widths)) in rows.iter().zip(&row_widths).enumerate() {
        // Row 0 is vertically centred at y = 0 (AutoCAD FCF insertion point is middle-left of row 0).
        let y_top = row_h * 0.5 - (ri as f64) * row_h;
        let y_bot = y_top - row_h;
        let total_w: f64 = widths.iter().sum();

        // Outer box of this row.
        frame_runs.push(vec![
            to_world(0.0, y_bot),
            to_world(total_w, y_bot),
            to_world(total_w, y_top),
            to_world(0.0, y_top),
            to_world(0.0, y_bot),
        ]);

        let mut x = 0.0;
        for (ci, (cell, &cw)) in row.iter().zip(widths).enumerate() {
            if ci > 0 {
                frame_runs.push(vec![to_world(x, y_bot), to_world(x, y_top)]);
            }
            let mut cx = x + pad_x;
            let text_y = y_bot + (row_h - text_h) * 0.5;
            for tok in cell {
                match tok {
                    FcfToken::Symbol(ch) => {
                        for stroke in gdt_symbol_strokes(*ch, cx, text_y, text_h, tolerance) {
                            frame_runs
                                .push(stroke.into_iter().map(|[u, v]| to_world(u, v)).collect());
                        }
                        cx += sym_advance;
                    }
                    FcfToken::Text(s) => {
                        let w = measure_text(s);
                        if !s.trim().is_empty() {
                            let wp = [
                                base[0] + cx * cos - text_y * sin,
                                base[1] + cx * sin + text_y * cos,
                            ];
                            let txt = Text {
                                contents: s.clone(),
                                position: crate::model::Point3::new(wp[0], wp[1], tol.position.z),
                                height: text_h,
                                style: style_name.to_owned(),
                                width_factor: 1.0,
                                rotation: tol.direction,
                                justify: Justify::Left,
                                underline: false,
                                overline: false,
                                field: None,
                                annotative: false,
                                annotation_scales: Vec::new(),
                                layer: tol.layer.clone(),
                                properties: tol.properties.clone(),
                            };
                            let l = text::draw_text(&txt, drawer.fonts, pen.lineweight, tolerance);
                            match placed {
                                Some(pl) => {
                                    drawn
                                        .shapes
                                        .extend(l.shapes.into_iter().map(|sh| pl.shape(sh)));
                                    drawn.runs.extend(l.runs.into_iter().map(|r| pl.run(r)));
                                }
                                None => {
                                    drawn.shapes.extend(l.shapes);
                                    drawn.runs.extend(l.runs);
                                }
                            }
                        }
                        cx += w;
                    }
                }
            }
            x += cw;
        }
    }

    add_stroke(&mut drawn, pen.lineweight, frame_runs);
    drawn
}

/// Draws a [`CenterMark`] (crosshair + optional extension lines).
pub(super) fn draw_center_mark(cm: &CenterMark, pen: Pen<'_>, placed: Option<&Placed>) -> Drawn {
    let dashes = pen.dashes();
    let mut runs = Vec::new();
    for [a, b] in cm.segments() {
        let seg = match placed {
            Some(pl) => [pl.point(flat(a)), pl.point(flat(b))],
            None => [flat(a), flat(b)],
        };
        runs.extend(linetype::along(&seg, dashes.as_deref(), false));
    }
    let mut drawn = Drawn::default();
    add_stroke(&mut drawn, pen.lineweight, runs);
    drawn
}

/// Draws a [`CenterLine`] (line extended by `start_extension` and `end_extension`).
pub(super) fn draw_center_line(cl: &CenterLine, pen: Pen<'_>, placed: Option<&Placed>) -> Drawn {
    let (p0, p1) = cl.extended_endpoints();
    let seg = match placed {
        Some(pl) => [pl.point(flat(p0)), pl.point(flat(p1))],
        None => [flat(p0), flat(p1)],
    };
    let dashes = pen.dashes();
    let runs = linetype::along(&seg, dashes.as_deref(), false);
    let mut drawn = Drawn::default();
    add_stroke(&mut drawn, pen.lineweight, runs);
    drawn
}

/// Draws an SHX [`ShxShape`] from `sheet.resolved_shapes`.
pub(super) fn draw_shape(
    drawer: &Drawer<'_>,
    sh: &ShxShape,
    pen: Pen<'_>,
    placed: Option<&Placed>,
) -> Drawn {
    let origin = flat(sh.position);
    let Some(resolved) = drawer.sheet.resolved_shape_for(sh) else {
        let p = match placed {
            Some(pl) => pl.point(origin),
            None => origin,
        };
        return Drawn {
            reach: vec![p],
            ..Drawn::default()
        };
    };
    let (sin, cos) = sin_cos_degrees(sh.rotation);
    let shear = sh.oblique.to_radians().tan();
    let xform = |[sx, sy]: Point2| -> Point2 {
        let lx = (sx * sh.width_factor + sy * shear) * sh.size;
        let ly = sy * sh.size;
        let w = [
            origin[0] + lx * cos - ly * sin,
            origin[1] + lx * sin + ly * cos,
        ];
        match placed {
            Some(pl) => pl.point(w),
            None => w,
        }
    };
    let runs: Vec<Vec<Point2>> = resolved
        .strokes
        .iter()
        .map(|run| run.iter().copied().map(xform).collect())
        .collect();
    let mut drawn = Drawn::default();
    add_stroke(&mut drawn, pen.lineweight, runs);
    drawn
}
