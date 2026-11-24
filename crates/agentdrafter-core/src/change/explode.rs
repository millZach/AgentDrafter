//! Exploding compound objects (`explode` / AutoCAD `EXPLODE` / `X`) into the
//! simpler objects that draw them:
//!
//! - `Insert` -> its evaluated block's objects placed, scaled, rotated and
//!   mirrored on the sheet (resolving layer `"0"` and `ByBlock` properties from
//!   the insert), plus each visible non-empty attribute turned into `Text`.
//!   Nested inserts explode one level, as in AutoCAD.
//! - `Polyline` -> `Line` and `Arc` segments (dropping width).
//! - `Donut` -> two semicircular `Arc`s.
//! - `RevisionCloud` -> the `Arc` (or `Line`) segments of its cloud polyline.
//! - `Hatch` -> clipped/dashed pattern `Line`s for a patterned hatch, or
//!   triangulated 2D `Solid` triangles for a `SOLID` hatch.
//! - `Dimension` -> extension/dimension `Line`s, `Arc`s, arrowhead `Solid` /
//!   `Donut` / `Line`s, and `MText` (or stacked-fraction `Text` + bar `Line`s).
//! - `MLeader` -> trimmed leader `Line`s, arrowheads, landing `Line`, and
//!   `MText` or `Insert` content.
//! - `Leader` -> trimmed leader `Line`s, arrowhead, and optional `MText`.
//! - `Table` -> border `Line`s and non-empty cell `MText`s.

use std::collections::BTreeMap;

use crate::description::normalize_degrees;
use crate::draw::polyline::arc_of;
use crate::draw::{hatch as draw_hatch, inside_outline, mtext_box, triangles, Point2, Shape};
use crate::error::{Error, ErrorCode};
use crate::model::block::{attribute_text, readable, Placement};
use crate::model::leader::{arrowhead_shapes, trimmed};
use crate::model::{
    Arc, Arrowhead, Circle, Dimension, Donut, Ellipse, Hatch, HatchEdge, HatchLoop, Insert,
    LaidContent, Leader, Lettered, Line, MLeader, MLeaderStyle, Object, ObjectColor,
    ObjectLinetype, ObjectLineweight, Point3, Polyline, Properties, RevisionCloud,
    RevisionCloudBoundary, Sheet, Solid, Spline, Table, TableStyle, Wipeout, LAYER_ZERO,
};
use crate::text::placement::SheetFonts;
use crate::text::{Justify, MText, Text};

/// Maximum number of objects one `explode` operation may produce from a single
/// compound object (guards against microscopically fine hatch patterns).
pub(crate) const MOST_EXPLODED_OBJECTS: usize = 10_000;

const EPS: f64 = 1e-9;

fn norm_deg(a: f64) -> f64 {
    let d = normalize_degrees(a);
    if (d - 360.0).abs() <= 1e-12 || d.abs() <= 1e-12 {
        0.0
    } else {
        d
    }
}

fn non_degenerate_line(a: Point3, b: Point3) -> bool {
    (b.x - a.x).hypot(b.y - a.y) > EPS || (b.z - a.z).abs() > EPS
}

fn effective_layer(child_layer: &str, parent_layer: &str) -> String {
    if child_layer == LAYER_ZERO {
        parent_layer.to_owned()
    } else {
        child_layer.to_owned()
    }
}

fn resolve_by_block(child: &Properties, parent: &Properties) -> Properties {
    Properties {
        color: match child.color {
            ObjectColor::ByBlock => parent.color,
            other => other,
        },
        linetype: match &child.linetype {
            ObjectLinetype::ByBlock => parent.linetype.clone(),
            other => other.clone(),
        },
        lineweight: match child.lineweight {
            ObjectLineweight::ByBlock => parent.lineweight,
            other => other,
        },
        linetype_scale: child.linetype_scale,
        plot_style: match &child.plot_style {
            crate::model::ObjectPlotStyle::ByBlock => parent.plot_style.clone(),
            other => other.clone(),
        },
        transparency: match child.transparency {
            crate::model::ObjectTransparency::ByBlock => parent.transparency,
            other => other,
        },
    }
}

fn resolve_style_pen(
    style_color: ObjectColor,
    style_lineweight: ObjectLineweight,
    parent: &Properties,
) -> Properties {
    Properties {
        color: match style_color {
            ObjectColor::ByBlock => parent.color,
            other => other,
        },
        linetype: parent.linetype.clone(),
        lineweight: match style_lineweight {
            ObjectLineweight::ByBlock => parent.lineweight,
            other => other,
        },
        linetype_scale: parent.linetype_scale,
        plot_style: parent.plot_style.clone(),
        transparency: parent.transparency,
    }
}

/// Explodes `object` on `sheet` into the simpler [`Object`]s that draw it.
pub(crate) fn compute_explode(
    object: &Object,
    sheet: &Sheet,
    fonts: &SheetFonts,
) -> Result<Vec<Object>, Error> {
    let out = match object {
        Object::Insert(insert) => explode_insert(insert, sheet)?,
        Object::Polyline(poly) => explode_polyline(poly)?,
        Object::Donut(donut) => explode_polyline(&donut.polyline())?,
        Object::RevisionCloud(cloud) => explode_polyline(&cloud.polyline())?,
        Object::Hatch(hatch) => explode_hatch(hatch)?,
        Object::Dimension(dim) => explode_dimension(dim, sheet, fonts)?,
        Object::MLeader(mleader) => explode_mleader(mleader, sheet, fonts)?,
        Object::Leader(leader) => explode_leader(leader)?,
        Object::Table(table) => explode_table(table, sheet)?,
        other => {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "cannot explode a {}; `explode` works on inserts, polylines, donuts, \
                     revision clouds, hatches, dimensions, multileaders, leaders and tables",
                    other.type_name()
                ),
            ));
        }
    };
    if out.is_empty() {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "exploding this {} would produce no objects",
                object.type_name()
            ),
        ));
    }
    if out.len() > MOST_EXPLODED_OBJECTS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "exploding this {} would create {} objects, over the {MOST_EXPLODED_OBJECTS} \
                 limit per object",
                object.type_name(),
                out.len()
            ),
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Insert (single-level explode, evaluating dynamic blocks & turning attributes into Text)
// ---------------------------------------------------------------------------

fn explode_insert(insert: &Insert, sheet: &Sheet) -> Result<Vec<Object>, Error> {
    let Some(block_cow) = sheet.block_for_insert(insert) else {
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!(
                "cannot explode insert of block `{}`: its block definition is not in the drawing",
                insert.block
            ),
        ));
    };
    let block = block_cow.as_ref();
    let placement = Placement::of(insert, block.base_point);
    let mut out = Vec::new();

    // Geometry and nested inserts in drawing order.
    for child in &block.objects {
        if matches!(child, Object::AttributeDefinition(_)) {
            continue;
        }
        if let Some(placed) = place_object(
            child,
            &placement,
            insert.rotation,
            &insert.layer,
            &insert.properties,
        ) {
            out.push(placed);
        }
    }

    // Visible attributes turned into single-line Text with their carried/constant values.
    for (p, child) in block.objects.iter().enumerate() {
        let Object::AttributeDefinition(def) = child else {
            continue;
        };
        if def.invisible {
            continue;
        }
        let value = block
            .attribute_value(insert, p)
            .unwrap_or(def.default.as_str());
        if value.trim().is_empty() {
            continue;
        }
        let mut text = attribute_text(insert, block, def, value);
        text.properties = resolve_by_block(&text.properties, &insert.properties);
        out.push(Object::Text(text));
    }

    Ok(out)
}

fn place_arc_angles(start_angle: f64, end_angle: f64, mirrored: bool, rotation: f64) -> (f64, f64) {
    if mirrored {
        (
            norm_deg(rotation + 180.0 - end_angle),
            norm_deg(rotation + 180.0 - start_angle),
        )
    } else {
        (
            norm_deg(start_angle + rotation),
            norm_deg(end_angle + rotation),
        )
    }
}

fn place_object(
    object: &Object,
    placement: &Placement,
    insert_rotation: f64,
    insert_layer: &str,
    insert_props: &Properties,
) -> Option<Object> {
    let scale = placement.scale();
    let mirrored = placement.mirrored();
    let layer = effective_layer(object.layer(), insert_layer);
    let properties = resolve_by_block(object.properties(), insert_props);

    match object {
        Object::Line(l) => Some(Object::Line(Line {
            from: placement.point(l.from),
            to: placement.point(l.to),
            layer,
            properties,
        })),
        Object::Polyline(p) => {
            let points = p.points.iter().map(|&pt| placement.point(pt)).collect();
            let bulges = if mirrored {
                p.bulges.iter().map(|&b| -b).collect()
            } else {
                p.bulges.clone()
            };
            let widths = p
                .widths
                .iter()
                .map(|&[sw, ew]| [sw * scale, ew * scale])
                .collect();
            Some(Object::Polyline(Polyline {
                points,
                bulges,
                widths,
                width: p.width * scale,
                closed: p.closed,
                layer,
                properties,
            }))
        }
        Object::Arc(a) => {
            let (start_angle, end_angle) =
                place_arc_angles(a.start_angle, a.end_angle, mirrored, insert_rotation);
            Some(Object::Arc(Arc {
                center: placement.point(a.center),
                radius: a.radius * scale,
                start_angle,
                end_angle,
                layer,
                properties,
            }))
        }
        Object::Circle(c) => Some(Object::Circle(Circle {
            center: placement.point(c.center),
            radius: c.radius * scale,
            layer,
            properties,
        })),
        Object::Ellipse(e) => {
            let origin = placement.point(Point3::new(0.0, 0.0, 0.0));
            let tip = placement.point(e.major_axis);
            let major_axis = Point3::new(tip.x - origin.x, tip.y - origin.y, 0.0);
            let (start_angle, end_angle) = if e.is_whole() {
                (0.0, 360.0)
            } else if mirrored {
                (
                    norm_deg(360.0 - e.end_angle),
                    norm_deg(360.0 - e.start_angle),
                )
            } else {
                (e.start_angle, e.end_angle)
            };
            Some(Object::Ellipse(Ellipse {
                center: placement.point(e.center),
                major_axis,
                ratio: e.ratio,
                start_angle,
                end_angle,
                layer,
                properties,
            }))
        }
        Object::Spline(s) => Some(Object::Spline(Spline {
            fit_points: s.fit_points.iter().map(|&p| placement.point(p)).collect(),
            control_points: s
                .control_points
                .iter()
                .map(|&p| placement.point(p))
                .collect(),
            degree: s.degree,
            layer,
            properties,
        })),
        Object::Donut(d) => Some(Object::Donut(Donut {
            center: placement.point(d.center),
            inside_diameter: d.inside_diameter * scale,
            outside_diameter: d.outside_diameter * scale,
            layer,
            properties,
        })),
        Object::Solid(s) => Some(Object::Solid(Solid {
            corners: s.corners.iter().map(|&c| placement.point(c)).collect(),
            layer,
            properties,
        })),
        Object::Wipeout(w) => Some(Object::Wipeout(Wipeout {
            points: w.points.iter().map(|&p| placement.point(p)).collect(),
            layer,
            properties,
        })),
        Object::RasterImage(img) => {
            let mut out = img.clone();
            out.position = placement.point(img.position);
            out.width *= scale;
            out.height = img.height.map(|h| h * scale);
            out.rotation = (img.rotation + insert_rotation).rem_euclid(360.0);
            out.clip = img.clip.as_ref().map(|pts| {
                pts.iter()
                    .map(|&p| {
                        let q = placement.point(Point3::new(p[0], p[1], img.position.z));
                        [q.x, q.y]
                    })
                    .collect()
            });
            out.layer = layer;
            out.properties = properties;
            Some(Object::RasterImage(out))
        }
        Object::PdfUnderlay(pdf) => {
            let mut out = pdf.clone();
            out.position = placement.point(pdf.position);
            out.scale *= scale;
            out.rotation = (pdf.rotation + insert_rotation).rem_euclid(360.0);
            out.clip = pdf.clip.as_ref().map(|pts| {
                pts.iter()
                    .map(|&p| {
                        let q = placement.point(Point3::new(p[0], p[1], pdf.position.z));
                        [q.x, q.y]
                    })
                    .collect()
            });
            out.layer = layer;
            out.properties = properties;
            Some(Object::PdfUnderlay(out))
        }
        Object::RevisionCloud(rc) => {
            let boundary = match &rc.boundary {
                RevisionCloudBoundary::Rectangle { from, to }
                    if !mirrored && insert_rotation.abs() <= 1e-9 =>
                {
                    RevisionCloudBoundary::Rectangle {
                        from: placement.point(*from),
                        to: placement.point(*to),
                    }
                }
                RevisionCloudBoundary::Rectangle { from, to } => {
                    let z = from.z;
                    let corners = [
                        *from,
                        Point3::new(to.x, from.y, z),
                        *to,
                        Point3::new(from.x, to.y, z),
                    ];
                    let mut pts: Vec<Point3> =
                        corners.into_iter().map(|c| placement.point(c)).collect();
                    if mirrored {
                        pts.reverse();
                    }
                    RevisionCloudBoundary::Polygon(pts)
                }
                RevisionCloudBoundary::Polygon(pts) => {
                    let mut out_pts: Vec<Point3> =
                        pts.iter().map(|&p| placement.point(p)).collect();
                    if mirrored {
                        out_pts.reverse();
                    }
                    RevisionCloudBoundary::Polygon(out_pts)
                }
            };
            Some(Object::RevisionCloud(RevisionCloud {
                boundary,
                arc_length: rc.arc_length * scale,
                style: rc.style,
                reverse: rc.reverse,
                layer,
                properties,
            }))
        }
        Object::Hatch(h) => {
            let boundary = h
                .boundary
                .iter()
                .map(|lp| match lp {
                    HatchLoop::Polyline { points } => HatchLoop::Polyline {
                        points: points.iter().map(|&p| placement.point(p)).collect(),
                    },
                    HatchLoop::Edges(edges) => HatchLoop::Edges(
                        edges
                            .iter()
                            .map(|edge| match *edge {
                                HatchEdge::Line { from, to } => HatchEdge::Line {
                                    from: placement.point(from),
                                    to: placement.point(to),
                                },
                                HatchEdge::Arc {
                                    center,
                                    radius,
                                    start_angle,
                                    end_angle,
                                } => {
                                    let (s, e) = place_arc_angles(
                                        start_angle,
                                        end_angle,
                                        mirrored,
                                        insert_rotation,
                                    );
                                    HatchEdge::Arc {
                                        center: placement.point(center),
                                        radius: radius * scale,
                                        start_angle: s,
                                        end_angle: e,
                                    }
                                }
                            })
                            .collect(),
                    ),
                })
                .collect();
            let (new_scale, new_angle, new_origin) = if h.is_solid() {
                (1.0, 0.0, [0.0, 0.0])
            } else {
                let o = placement.point(Point3::new(h.origin[0], h.origin[1], 0.0));
                let ang = if mirrored {
                    norm_deg(insert_rotation + 180.0 - h.angle)
                } else {
                    norm_deg(h.angle + insert_rotation)
                };
                (h.scale * scale, ang, [o.x, o.y])
            };
            Some(Object::Hatch(Hatch {
                pattern: h.pattern.clone(),
                scale: new_scale,
                angle: new_angle,
                origin: new_origin,
                boundary,
                layer,
                properties,
                annotative: false,
                annotation_scales: Vec::new(),
            }))
        }
        Object::Text(t) => {
            if t.contents.trim().is_empty() {
                return None;
            }
            let rotation = if mirrored {
                readable(insert_rotation + 180.0 - t.rotation)
            } else {
                norm_deg(t.rotation + insert_rotation)
            };
            Some(Object::Text(Text {
                contents: t.contents.clone(),
                position: placement.point(t.position),
                height: t.height * scale,
                style: t.style.clone(),
                width_factor: t.width_factor,
                rotation,
                justify: t.justify,
                layer,
                field: t.field.clone(),
                properties,
                annotative: false,
                annotation_scales: Vec::new(),
            }))
        }
        Object::MText(t) => {
            if t.contents.trim().is_empty() {
                return None;
            }
            let rotation = if mirrored {
                readable(insert_rotation + 180.0 - t.rotation)
            } else {
                norm_deg(t.rotation + insert_rotation)
            };
            Some(Object::MText(MText {
                contents: t.contents.clone(),
                position: placement.point(t.position),
                height: t.height * scale,
                width: t.width * scale,
                style: t.style.clone(),
                width_factor: t.width_factor,
                rotation,
                justify: t.justify,
                layer,
                field: t.field.clone(),
                properties,
                annotative: false,
                annotation_scales: Vec::new(),
            }))
        }
        Object::Insert(inner) => {
            let new_rotation = if mirrored {
                norm_deg(insert_rotation - inner.rotation)
            } else {
                norm_deg(insert_rotation + inner.rotation)
            };
            Some(Object::Insert(Insert {
                block: inner.block.clone(),
                position: placement.point(inner.position),
                scale: inner.scale * scale,
                rotation: new_rotation,
                mirrored: inner.mirrored ^ mirrored,
                readable_attributes: inner.readable_attributes || mirrored,
                attributes: inner.attributes.clone(),
                state: inner.state.clone(),
                evaluated_block: inner.evaluated_block.clone(),
                name: None,
                layer,
                properties,
                annotative: false,
                annotation_scales: Vec::new(),
            }))
        }
        Object::AttributeDefinition(_)
        | Object::Table(_)
        | Object::MLeader(_)
        | Object::Leader(_)
        | Object::Dimension(_)
        | Object::Viewport(_) => None,
    }
}

// ---------------------------------------------------------------------------
// Polyline / Donut / RevisionCloud -> Lines and Arcs
// ---------------------------------------------------------------------------

fn explode_polyline(poly: &Polyline) -> Result<Vec<Object>, Error> {
    let n = poly.points.len();
    if n < 2 {
        return Ok(Vec::new());
    }
    let flat = poly.is_flat();
    let seg_count = if poly.closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(seg_count);
    for i in 0..seg_count {
        let p0 = poly.points[i];
        let p1 = poly.points[(i + 1) % n];
        if !non_degenerate_line(p0, p1) {
            continue;
        }
        let bulge = if flat { poly.bulge(i) } else { 0.0 };
        if bulge.abs() > 1e-12 {
            if let Some(sa) = arc_of([p0.x, p0.y], [p1.x, p1.y], bulge) {
                let (start_angle, end_angle) = if sa.sweep >= 0.0 {
                    (
                        norm_deg(sa.start.to_degrees()),
                        norm_deg((sa.start + sa.sweep).to_degrees()),
                    )
                } else {
                    (
                        norm_deg((sa.start + sa.sweep).to_degrees()),
                        norm_deg(sa.start.to_degrees()),
                    )
                };
                if (start_angle - end_angle).abs() > 1e-9 {
                    out.push(Object::Arc(Arc {
                        center: Point3::new(sa.center[0], sa.center[1], p0.z),
                        radius: sa.radius,
                        start_angle,
                        end_angle,
                        layer: poly.layer.clone(),
                        properties: poly.properties.clone(),
                    }));
                    continue;
                }
            }
        }
        out.push(Object::Line(Line {
            from: p0,
            to: p1,
            layer: poly.layer.clone(),
            properties: poly.properties.clone(),
        }));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Hatch -> Pattern Lines (patterned) or Triangulated 2D Solids (solid)
// ---------------------------------------------------------------------------

fn hatch_elevation(hatch: &Hatch) -> f64 {
    for lp in &hatch.boundary {
        match lp {
            HatchLoop::Polyline { points } => {
                if let Some(p) = points.first() {
                    return p.z;
                }
            }
            HatchLoop::Edges(edges) => {
                if let Some(edge) = edges.first() {
                    return match *edge {
                        HatchEdge::Line { from, .. } => from.z,
                        HatchEdge::Arc { center, .. } => center.z,
                    };
                }
            }
        }
    }
    0.0
}

fn explode_hatch(hatch: &Hatch) -> Result<Vec<Object>, Error> {
    let z = hatch_elevation(hatch);
    // Estimate scale for curve chord tolerance.
    let mut reach = 1.0_f64;
    for lp in &hatch.boundary {
        match lp {
            HatchLoop::Polyline { points } => {
                for p in points {
                    reach = reach.max(p.x.abs()).max(p.y.abs());
                }
            }
            HatchLoop::Edges(edges) => {
                for e in edges {
                    let (a, b) = e.ends();
                    reach = reach
                        .max(a.x.abs())
                        .max(a.y.abs())
                        .max(b.x.abs())
                        .max(b.y.abs());
                }
            }
        }
    }
    let tolerance = (reach * 1e-4).clamp(1e-4, 0.25);
    let drawn = draw_hatch::draw(hatch, 0.25, tolerance, MOST_EXPLODED_OBJECTS + 1);
    if drawn.cut_short {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "hatch `{}` pattern exceeds the {MOST_EXPLODED_OBJECTS}-segment explode limit",
                hatch.pattern.name
            ),
        ));
    }

    let mut out = Vec::new();
    if hatch.is_solid() {
        let polygons = merge_hatch_loops_with_holes(&drawn.loops);
        for poly in polygons {
            for [a, b, c] in triangles(&poly) {
                let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                if cross.abs() <= 1e-12 {
                    continue;
                }
                out.push(Object::Solid(Solid {
                    corners: vec![
                        Point3::new(a[0], a[1], z),
                        Point3::new(b[0], b[1], z),
                        Point3::new(c[0], c[1], z),
                    ],
                    layer: hatch.layer.clone(),
                    properties: hatch.properties.clone(),
                }));
            }
        }
    } else {
        for shape in drawn.shapes {
            if let Shape::Stroke { runs, .. } = shape {
                for run in runs {
                    for pair in run.windows(2) {
                        let a = Point3::new(pair[0][0], pair[0][1], z);
                        let b = Point3::new(pair[1][0], pair[1][1], z);
                        if non_degenerate_line(a, b) {
                            out.push(Object::Line(Line {
                                from: a,
                                to: b,
                                layer: hatch.layer.clone(),
                                properties: hatch.properties.clone(),
                            }));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Groups solid-hatch loops into simple outer polygons with any direct hole
/// loops bridged into them so ear-clipping triangulation covers the exact
/// even-odd filled region.
fn merge_hatch_loops_with_holes(loops: &[Vec<Point2>]) -> Vec<Vec<Point2>> {
    if loops.len() <= 1 {
        return loops.to_vec();
    }
    let areas: Vec<f64> = loops.iter().map(|l| signed_area(l).abs()).collect();
    // Depth = how many other loops strictly contain loops[i][0].
    let mut parents: Vec<Option<usize>> = vec![None; loops.len()];
    let mut depths: Vec<usize> = vec![0; loops.len()];
    for i in 0..loops.len() {
        let sample = loops[i][0];
        let mut containers: Vec<usize> = (0..loops.len())
            .filter(|&j| j != i && areas[j] > areas[i] && inside_outline(sample, &loops[j]))
            .collect();
        depths[i] = containers.len();
        containers.sort_by(|&a, &b| areas[a].total_cmp(&areas[b]));
        parents[i] = containers.first().copied();
    }

    let mut holes_of: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..loops.len() {
        if depths[i] % 2 == 1 {
            if let Some(parent) = parents[i] {
                holes_of.entry(parent).or_default().push(i);
            }
        }
    }

    let mut result = Vec::new();
    for (i, lp) in loops.iter().enumerate() {
        if depths[i] % 2 == 1 {
            continue;
        }
        let mut outer = lp.clone();
        if signed_area(&outer) < 0.0 {
            outer.reverse();
        }
        if let Some(mut holes) = holes_of.remove(&i) {
            // Sort holes by rightmost X descending so earlier bridges never block later holes.
            holes.sort_by(|&a, &b| {
                let max_x = |idx: usize| {
                    loops[idx]
                        .iter()
                        .map(|p| p[0])
                        .fold(f64::NEG_INFINITY, f64::max)
                };
                max_x(b).total_cmp(&max_x(a))
            });
            for h_idx in holes {
                let mut hole = loops[h_idx].clone();
                if signed_area(&hole) > 0.0 {
                    hole.reverse();
                }
                outer = bridge_hole(outer, &hole);
            }
        }
        result.push(outer);
    }
    result
}

fn signed_area(poly: &[Point2]) -> f64 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        * 0.5
}

fn bridge_hole(outer: Vec<Point2>, hole: &[Point2]) -> Vec<Point2> {
    if hole.is_empty() || outer.is_empty() {
        return outer;
    }
    let (h_idx, &h_pt) = hole
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a[0].total_cmp(&b[0]))
        .unwrap();
    // Pick the nearest outer vertex (preferring x >= h_pt[0]) as the bridge target.
    let o_idx = outer
        .iter()
        .enumerate()
        .filter(|(_, p)| p[0] >= h_pt[0] - 1e-9)
        .min_by(|(_, a), (_, b)| {
            let da = (a[0] - h_pt[0]).hypot(a[1] - h_pt[1]);
            let db = (b[0] - h_pt[0]).hypot(b[1] - h_pt[1]);
            da.total_cmp(&db)
        })
        .or_else(|| {
            outer.iter().enumerate().min_by(|(_, a), (_, b)| {
                let da = (a[0] - h_pt[0]).hypot(a[1] - h_pt[1]);
                let db = (b[0] - h_pt[0]).hypot(b[1] - h_pt[1]);
                da.total_cmp(&db)
            })
        })
        .map_or(0, |(idx, _)| idx);

    let mut merged = Vec::with_capacity(outer.len() + hole.len() + 2);
    merged.extend_from_slice(&outer[..=o_idx]);
    for k in 0..hole.len() {
        merged.push(hole[(h_idx + k) % hole.len()]);
    }
    merged.push(hole[h_idx]);
    merged.extend_from_slice(&outer[o_idx..]);
    merged
}

// ---------------------------------------------------------------------------
// Arrowhead helper shared by Dimension, MLeader and Leader
// ---------------------------------------------------------------------------

fn arrowhead_objects(
    arrowhead: Arrowhead,
    tip: Point3,
    base: Point3,
    size: f64,
    layer: &str,
    props: &Properties,
    out: &mut Vec<Object>,
) {
    let Some(shapes) = arrowhead_shapes(arrowhead, [tip.x, tip.y], [base.x, base.y], size) else {
        return;
    };
    let z = tip.z;
    for tri in shapes.fills {
        if tri.len() >= 3 {
            out.push(Object::Solid(Solid {
                corners: tri
                    .into_iter()
                    .map(|p| Point3::new(p[0], p[1], z))
                    .collect(),
                layer: layer.to_owned(),
                properties: props.clone(),
            }));
        }
    }
    if let Some((center, radius)) = shapes.dot {
        if radius > EPS {
            out.push(Object::Donut(Donut {
                center: Point3::new(center[0], center[1], z),
                inside_diameter: 0.0,
                outside_diameter: radius * 2.0,
                layer: layer.to_owned(),
                properties: props.clone(),
            }));
        }
    }
    for line in shapes.lines {
        for pair in line.windows(2) {
            let a = Point3::new(pair[0][0], pair[0][1], z);
            let b = Point3::new(pair[1][0], pair[1][1], z);
            if non_degenerate_line(a, b) {
                out.push(Object::Line(Line {
                    from: a,
                    to: b,
                    layer: layer.to_owned(),
                    properties: props.clone(),
                }));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dimension -> Lines, Arcs, Arrowheads, and MText (or stacked Text + bars)
// ---------------------------------------------------------------------------

fn explode_dimension(
    dim: &Dimension,
    sheet: &Sheet,
    fonts: &SheetFonts,
) -> Result<Vec<Object>, Error> {
    let Some(style) = sheet.dimension_style(&dim.dimension_style) else {
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!(
                "dimension uses style `{}`, which is not defined in the drawing",
                dim.dimension_style
            ),
        ));
    };
    let laid = dim.laid_out(&style, sheet.units, &|t| mtext_box(t, fonts));
    // Match Drawer::dimension pens: lines inherit ByBlock color & lineweight, ByLayer linetype;
    // text inherits ByBlock color and ByLayer lineweight & linetype.
    let line_props = Properties {
        color: dim.properties.color,
        linetype: ObjectLinetype::ByLayer,
        lineweight: dim.properties.lineweight,
        linetype_scale: 1.0,
        plot_style: dim.properties.plot_style.clone(),
        transparency: dim.properties.transparency,
    };
    let text_props = Properties {
        color: dim.properties.color,
        linetype: ObjectLinetype::ByLayer,
        lineweight: ObjectLineweight::ByLayer,
        linetype_scale: 1.0,
        plot_style: dim.properties.plot_style.clone(),
        transparency: dim.properties.transparency,
    };
    let mut out = Vec::new();
    for [a, b] in laid.lines() {
        if non_degenerate_line(a, b) {
            out.push(Object::Line(Line {
                from: a,
                to: b,
                layer: dim.layer.clone(),
                properties: line_props.clone(),
            }));
        }
    }
    for (center, radius, start_angle, end_angle) in laid.arcs() {
        if radius > EPS {
            out.push(Object::Arc(Arc {
                center,
                radius,
                start_angle: norm_deg(start_angle),
                end_angle: norm_deg(end_angle),
                layer: dim.layer.clone(),
                properties: line_props.clone(),
            }));
        }
    }
    for (tip, base) in laid.arrows() {
        arrowhead_objects(
            laid.arrowhead,
            tip,
            base,
            laid.arrow_size,
            &dim.layer,
            &line_props,
            &mut out,
        );
    }
    if laid.lettering.is_empty() {
        if !laid.text.contents.trim().is_empty() {
            let mut mtext = laid.text;
            mtext.layer = dim.layer.clone();
            mtext.properties = text_props;
            out.push(Object::MText(mtext));
        }
    } else {
        let width_factor = fonts.style(&laid.text.style).1;
        for piece in &laid.lettering {
            match piece {
                Lettered::Text { contents, start } => {
                    if !contents.trim().is_empty() {
                        out.push(Object::Text(Text {
                            contents: contents.clone(),
                            position: *start,
                            height: laid.text.height,
                            style: laid.text.style.clone(),
                            width_factor,
                            rotation: laid.text.rotation,
                            justify: Justify::Left,
                            layer: dim.layer.clone(),
                            field: None,
                            properties: text_props.clone(),
                            annotative: false,
                            annotation_scales: Vec::new(),
                        }));
                    }
                }
                Lettered::Line([a, b]) => {
                    if non_degenerate_line(*a, *b) {
                        out.push(Object::Line(Line {
                            from: *a,
                            to: *b,
                            layer: dim.layer.clone(),
                            properties: text_props.clone(),
                        }));
                    }
                }
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// MLeader & Leader -> Arrowheads, Leader Lines, Landing, and MText / Insert
// ---------------------------------------------------------------------------

fn explode_mleader(
    mleader: &MLeader,
    sheet: &Sheet,
    fonts: &SheetFonts,
) -> Result<Vec<Object>, Error> {
    let style = sheet
        .mleader_style(&mleader.mleader_style)
        .unwrap_or_else(|| MLeaderStyle::standard(sheet.units));
    let laid = mleader.laid_out(&style, &|t| mtext_box(t, fonts), &sheet.blocks);
    let line_props = resolve_style_pen(
        style.leader_color,
        style.leader_lineweight,
        &mleader.properties,
    );
    let mut out = Vec::new();
    for pts in &laid.lines {
        if pts.len() < 2 {
            continue;
        }
        let z = pts[0].z;
        arrowhead_objects(
            laid.arrowhead,
            pts[0],
            pts[1],
            laid.arrow_size,
            &mleader.layer,
            &line_props,
            &mut out,
        );
        let pts_2d: Vec<[f64; 2]> = pts.iter().map(|p| [p.x, p.y]).collect();
        let shaft = trimmed(&pts_2d, laid.arrowhead, laid.arrow_size);
        for pair in shaft.windows(2) {
            let a = Point3::new(pair[0][0], pair[0][1], z);
            let b = Point3::new(pair[1][0], pair[1][1], z);
            if non_degenerate_line(a, b) {
                out.push(Object::Line(Line {
                    from: a,
                    to: b,
                    layer: mleader.layer.clone(),
                    properties: line_props.clone(),
                }));
            }
        }
    }
    if laid.landing > 0.0 {
        let a = laid.connection;
        let b = laid.landing_end();
        if non_degenerate_line(a, b) {
            out.push(Object::Line(Line {
                from: a,
                to: b,
                layer: mleader.layer.clone(),
                properties: line_props,
            }));
        }
    }
    match laid.content {
        LaidContent::Text { mut text, .. } => {
            if !text.contents.trim().is_empty() {
                text.layer = mleader.layer.clone();
                text.properties = mleader.properties.clone();
                out.push(Object::MText(text));
            }
        }
        LaidContent::Block { mut insert, .. } => {
            insert.name = None;
            insert.layer = mleader.layer.clone();
            insert.properties = mleader.properties.clone();
            out.push(Object::Insert(insert));
        }
    }
    Ok(out)
}

fn explode_leader(leader: &Leader) -> Result<Vec<Object>, Error> {
    let path = leader.path();
    let mut out = Vec::new();
    if path.len() >= 2 {
        let z = path[0].z;
        arrowhead_objects(
            leader.arrowhead,
            path[0],
            path[1],
            leader.arrow_size,
            &leader.layer,
            &leader.properties,
            &mut out,
        );
        let pts_2d: Vec<[f64; 2]> = path.iter().map(|p| [p.x, p.y]).collect();
        let shaft = trimmed(&pts_2d, leader.arrowhead, leader.arrow_size);
        for pair in shaft.windows(2) {
            let a = Point3::new(pair[0][0], pair[0][1], z);
            let b = Point3::new(pair[1][0], pair[1][1], z);
            if non_degenerate_line(a, b) {
                out.push(Object::Line(Line {
                    from: a,
                    to: b,
                    layer: leader.layer.clone(),
                    properties: leader.properties.clone(),
                }));
            }
        }
    }
    if let Some(mut annotation) = leader.annotation_text() {
        if !annotation.contents.trim().is_empty() {
            annotation.layer = leader.layer.clone();
            annotation.properties = leader.properties.clone();
            out.push(Object::MText(annotation));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Table -> Border Lines and Cell MTexts
// ---------------------------------------------------------------------------

fn explode_table(table: &Table, sheet: &Sheet) -> Result<Vec<Object>, Error> {
    let style = sheet
        .table_style(&table.table_style)
        .unwrap_or_else(|| TableStyle::standard(sheet.units));
    let border_props = resolve_style_pen(
        style.border_color,
        style.border_lineweight,
        &table.properties,
    );
    let text_props = Properties {
        color: table.properties.color,
        linetype: ObjectLinetype::ByLayer,
        lineweight: ObjectLineweight::ByLayer,
        linetype_scale: 1.0,
        plot_style: table.properties.plot_style.clone(),
        transparency: table.properties.transparency,
    };
    let z = table.position.z;
    let mut out = Vec::new();
    for [from, to] in table.borders() {
        let a = Point3::new(from[0], from[1], z);
        let b = Point3::new(to[0], to[1], z);
        if non_degenerate_line(a, b) {
            out.push(Object::Line(Line {
                from: a,
                to: b,
                layer: table.layer.clone(),
                properties: border_props.clone(),
            }));
        }
    }
    for cell in table.laid_out(&style) {
        if let Some(mut text) = cell.text {
            if !text.contents.trim().is_empty() {
                text.layer = table.layer.clone();
                text.properties = text_props.clone();
                out.push(Object::MText(text));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Attribute, AttributeDefinition, Block, DimensionStyle, HatchPattern, HatchPatternLine,
        LeaderAnnotation, MLeaderContent, MLeaderText, Measured, TableCell, ISO_DIMENSION_STYLE,
        SOLID, STANDARD_MLEADER_STYLE, STANDARD_TABLE_STYLE,
    };
    use crate::settings::Settings;
    use crate::text::STANDARD;
    use crate::units::Unit;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-5, "expected {b}, got {a}");
    }

    fn base_sheet() -> Sheet {
        Sheet {
            units: Unit::Mm,
            block_library: None,
            linetypes: Vec::new(),
            layers: Vec::new(),
            text_styles: Vec::new(),
            table_styles: Vec::new(),
            mleader_styles: Vec::new(),
            dimension_styles: Vec::new(),
            blocks: Vec::new(),
            xrefs: Vec::new(),
            resolved_xrefs: Vec::new(),
            resolved_media: Vec::new(),
            objects: Vec::new(),
            paper: None,
            page_setup: None,
            ltscale: 1.0,
            layout: None,
            other_layouts: Vec::new(),
            wipeout_frame: crate::model::WipeoutFrame::Off,
            image_frame: crate::model::WipeoutFrame::Off,
            custom_properties: BTreeMap::new(),
            groups: Vec::new(),
            annotation_scales: Vec::new(),
            annotation_scale: None,
            anno_all_visible: None,
        }
    }

    #[test]
    fn explode_insert_transforms_geometry_resolves_layer0_byblock_and_turns_attributes_into_text() {
        let mut sheet = base_sheet();
        sheet.blocks.push(Block {
            name: "VALVE".to_owned(),
            base_point: Point3::new(0.0, 0.0, 0.0),
            objects: vec![
                Object::Line(Line {
                    from: Point3::new(-5.0, 0.0, 0.0),
                    to: Point3::new(5.0, 0.0, 0.0),
                    layer: LAYER_ZERO.to_owned(),
                    properties: Properties {
                        color: ObjectColor::ByBlock,
                        ..Properties::default()
                    },
                }),
                Object::Arc(Arc {
                    center: Point3::new(0.0, 0.0, 0.0),
                    radius: 3.0,
                    start_angle: 0.0,
                    end_angle: 90.0,
                    layer: "SYMBOLS".to_owned(),
                    properties: Properties::default(),
                }),
                Object::AttributeDefinition(AttributeDefinition {
                    tag: "TAG".to_owned(),
                    prompt: String::new(),
                    default: "XV-000".to_owned(),
                    position: Point3::new(0.0, 6.0, 0.0),
                    height: 2.5,
                    style: STANDARD.to_owned(),
                    width_factor: 1.0,
                    rotation: 0.0,
                    justify: Justify::MiddleCenter,
                    invisible: false,
                    constant: false,
                    verify: false,
                    preset: false,
                    lock_position: false,
                    layer: LAYER_ZERO.to_owned(),
                    properties: Properties {
                        color: ObjectColor::ByBlock,
                        ..Properties::default()
                    },
                }),
                Object::AttributeDefinition(AttributeDefinition {
                    tag: "SECRET".to_owned(),
                    prompt: String::new(),
                    default: "HIDDEN".to_owned(),
                    position: Point3::new(0.0, -6.0, 0.0),
                    height: 2.0,
                    style: STANDARD.to_owned(),
                    width_factor: 1.0,
                    rotation: 0.0,
                    justify: Justify::Left,
                    invisible: true,
                    constant: false,
                    verify: false,
                    preset: false,
                    lock_position: false,
                    layer: LAYER_ZERO.to_owned(),
                    properties: Properties::default(),
                }),
            ],
            cells: Vec::new(),
            parameters: Vec::new(),
            annotative: false,
        });
        let ins = Object::Insert(Insert {
            block: "VALVE".to_owned(),
            position: Point3::new(100.0, 50.0, 0.0),
            scale: 2.0,
            rotation: 90.0,
            mirrored: false,
            readable_attributes: false,
            attributes: vec![
                Attribute {
                    tag: "TAG".to_owned(),
                    value: "XV-101".to_owned(),
                    field: None,
                },
                Attribute {
                    tag: "SECRET".to_owned(),
                    value: "X".to_owned(),
                    field: None,
                },
            ],
            state: BTreeMap::new(),
            evaluated_block: None,
            name: Some("v1".to_owned()),
            layer: "INSTRUMENTS".to_owned(),
            properties: Properties {
                color: ObjectColor::Index(3),
                ..Properties::default()
            },
            annotative: false,
            annotation_scales: Vec::new(),
        });
        let fonts = SheetFonts::find(&sheet, &Settings::default());
        let pieces = compute_explode(&ins, &sheet, &fonts).unwrap();
        // Line + Arc + visible TAG Text (invisible SECRET skipped) = 3.
        assert_eq!(pieces.len(), 3);
        let Object::Line(ref l) = pieces[0] else {
            panic!("expected line");
        };
        assert_eq!(l.layer, "INSTRUMENTS");
        assert_eq!(l.properties.color, ObjectColor::Index(3));
        close(l.from.x, 100.0);
        close(l.from.y, 40.0);
        close(l.to.x, 100.0);
        close(l.to.y, 60.0);

        let Object::Arc(ref a) = pieces[1] else {
            panic!("expected arc");
        };
        assert_eq!(a.layer, "SYMBOLS");
        close(a.radius, 6.0);
        close(a.start_angle, 90.0);
        close(a.end_angle, 180.0);

        let Object::Text(ref t) = pieces[2] else {
            panic!("expected text");
        };
        assert_eq!(t.contents, "XV-101");
        assert_eq!(t.layer, "INSTRUMENTS");
        assert_eq!(t.properties.color, ObjectColor::Index(3));
        close(t.height, 5.0);
        close(t.position.x, 88.0);
        close(t.position.y, 50.0);
        close(t.rotation, 90.0);
    }

    #[test]
    fn explode_polyline_with_bulge_donut_and_hatches() {
        let sheet = base_sheet();
        let fonts = SheetFonts::find(&sheet, &Settings::default());
        let poly = Object::Polyline(Polyline {
            points: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(20.0, 0.0, 0.0),
                Point3::new(20.0, 10.0, 0.0),
            ],
            bulges: vec![0.0, 1.0, 0.0],
            widths: Vec::new(),
            width: 1.0,
            closed: false,
            layer: "WIRES".to_owned(),
            properties: Properties::default(),
        });
        let pieces = compute_explode(&poly, &sheet, &fonts).unwrap();
        assert_eq!(pieces.len(), 2);
        assert!(matches!(pieces[0], Object::Line(_)));
        let Object::Arc(ref a) = pieces[1] else {
            panic!("expected arc");
        };
        close(a.center.x, 20.0);
        close(a.center.y, 5.0);
        close(a.radius, 5.0);
        close(a.start_angle, 270.0);
        close(a.end_angle, 90.0);

        // Donut -> 2 semicircular arcs.
        let donut = Object::Donut(Donut {
            center: Point3::new(5.0, 5.0, 0.0),
            inside_diameter: 4.0,
            outside_diameter: 8.0,
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let dp = compute_explode(&donut, &sheet, &fonts).unwrap();
        assert_eq!(dp.len(), 2);
        assert!(dp.iter().all(|o| matches!(o, Object::Arc(_))));

        // Patterned hatch -> Lines.
        let pat_hatch = Object::Hatch(Hatch {
            pattern: HatchPattern {
                name: "STRIPES".to_owned(),
                lines: vec![HatchPatternLine {
                    angle: 0.0,
                    origin: [0.0, 0.0],
                    delta: [0.0, 2.0],
                    dashes: Vec::new(),
                }],
            },
            scale: 1.0,
            angle: 0.0,
            origin: [0.0, 0.0],
            boundary: vec![HatchLoop::Polyline {
                points: vec![
                    Point3::new(0.0, 0.0, 0.0),
                    Point3::new(10.0, 0.0, 0.0),
                    Point3::new(10.0, 10.0, 0.0),
                    Point3::new(0.0, 10.0, 0.0),
                ],
            }],
            layer: "HATCH".to_owned(),
            properties: Properties::default(),
            annotative: false,
            annotation_scales: Vec::new(),
        });
        let hp = compute_explode(&pat_hatch, &sheet, &fonts).unwrap();
        assert_eq!(hp.len(), 4); // y = 2, 4, 6, 8
        assert!(hp.iter().all(|o| matches!(o, Object::Line(_))));

        // Solid hatch -> triangulated Solids.
        let solid_hatch = Object::Hatch(Hatch {
            pattern: HatchPattern {
                name: SOLID.to_owned(),
                lines: Vec::new(),
            },
            scale: 1.0,
            angle: 0.0,
            origin: [0.0, 0.0],
            boundary: vec![HatchLoop::Polyline {
                points: vec![
                    Point3::new(0.0, 0.0, 0.0),
                    Point3::new(10.0, 0.0, 0.0),
                    Point3::new(10.0, 10.0, 0.0),
                    Point3::new(0.0, 10.0, 0.0),
                ],
            }],
            layer: "FILL".to_owned(),
            properties: Properties::default(),
            annotative: false,
            annotation_scales: Vec::new(),
        });
        let sp = compute_explode(&solid_hatch, &sheet, &fonts).unwrap();
        assert_eq!(sp.len(), 2);
        assert!(sp.iter().all(|o| matches!(o, Object::Solid(_))));
    }

    #[test]
    fn explode_dimension_mleader_leader_table_and_refuse_primitive() {
        let sheet = base_sheet();
        let fonts = SheetFonts::find(&sheet, &Settings::default());
        let dim = Object::Dimension(Dimension {
            measured: Measured::Linear {
                points: [Point3::new(0.0, 0.0, 0.0), Point3::new(50.0, 0.0, 0.0)],
                angle: 0.0,
            },
            location: Point3::new(25.0, 15.0, 0.0),
            text: None,
            dimension_style: ISO_DIMENSION_STYLE.to_owned(),
            continued: false,
            layer: "DIM".to_owned(),
            properties: Properties::default(),
            annotative: false,
            annotation_scales: Vec::new(),
        });
        let dp = compute_explode(&dim, &sheet, &fonts).unwrap();
        assert!(dp.iter().any(|o| matches!(o, Object::Line(_))));
        assert!(dp.iter().any(|o| matches!(o, Object::Solid(_))));
        assert!(dp.iter().any(|o| matches!(o, Object::MText(_))));

        let mleader = Object::MLeader(MLeader {
            leaders: vec![vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(15.0, 10.0, 0.0),
            ]],
            content: MLeaderContent::Text(MLeaderText {
                text: "NOTE".to_owned(),
                style: None,
                height: None,
                attachment: None,
            }),
            landing: None,
            gap: None,
            arrowhead: None,
            arrow_size: None,
            mleader_style: STANDARD_MLEADER_STYLE.to_owned(),
            layer: "NOTES".to_owned(),
            properties: Properties::default(),
            annotative: false,
            annotation_scales: Vec::new(),
        });
        let mp = compute_explode(&mleader, &sheet, &fonts).unwrap();
        assert!(mp.iter().any(|o| matches!(o, Object::Solid(_))));
        assert!(mp.iter().any(|o| matches!(o, Object::Line(_))));
        assert!(mp
            .iter()
            .any(|o| matches!(o, Object::MText(t) if t.contents == "NOTE")));

        let leader = Object::Leader(Leader {
            points: vec![Point3::new(0.0, 0.0, 0.0), Point3::new(20.0, 10.0, 0.0)],
            arrowhead: Arrowhead::ClosedFilled,
            arrow_size: 2.5,
            gap: 1.0,
            annotation: Some(LeaderAnnotation {
                text: "OLD".to_owned(),
                style: STANDARD.to_owned(),
                height: 2.5,
            }),
            layer: "NOTES".to_owned(),
            properties: Properties::default(),
        });
        let lp = compute_explode(&leader, &sheet, &fonts).unwrap();
        assert!(lp
            .iter()
            .any(|o| matches!(o, Object::MText(t) if t.contents == "OLD")));

        let table = Object::Table(Table {
            position: Point3::new(0.0, 40.0, 0.0),
            columns: vec![30.0, 30.0],
            rows: vec![10.0, 10.0],
            cells: vec![
                vec![
                    TableCell {
                        text: "TAG".to_owned(),
                        ..Default::default()
                    },
                    TableCell {
                        text: "DESC".to_owned(),
                        ..Default::default()
                    },
                ],
                vec![
                    TableCell {
                        text: "FT-101".to_owned(),
                        ..Default::default()
                    },
                    TableCell::default(),
                ],
            ],
            title: false,
            header: true,
            table_style: STANDARD_TABLE_STYLE.to_owned(),
            layer: "TABLES".to_owned(),
            properties: Properties::default(),
        });
        let tp = compute_explode(&table, &sheet, &fonts).unwrap();
        assert!(tp.iter().any(|o| matches!(o, Object::Line(_))));
        let mtexts: Vec<&str> = tp
            .iter()
            .filter_map(|o| match o {
                Object::MText(t) => Some(t.contents.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(mtexts, vec!["TAG", "DESC", "FT-101"]);

        // Primitive line is refused cleanly.
        let line = Object::Line(Line {
            from: Point3::new(0.0, 0.0, 0.0),
            to: Point3::new(10.0, 0.0, 0.0),
            layer: "0".to_owned(),
            properties: Properties::default(),
        });
        let err = compute_explode(&line, &sheet, &fonts).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadValue);

        // Suppress unused import warning on DimensionStyle.
        let _ = DimensionStyle::built_in(ISO_DIMENSION_STYLE, Unit::Mm);
    }
}
