//! DWG and DXF reading and writing for Rank 40 2D objects:
//! `POINT` (`PDMODE`, `PDSIZE`), `XLINE`, `RAY`, `MLINE` + `ACAD_MLINESTYLE`,
//! `TOLERANCE`, `CENTERMARK`, `CENTERLINE`, and `SHAPE`.

use std::collections::HashMap;

use acadrust::entities::{
    CenterLineAssociation, CenterLineSource, CenterLineSourceKind, CenterMarkAssociation,
    CenterMarkSource, CenterMarkSourceKind, EntityCommon, EntityType, Insert as DwgInsert,
    Line as DwgLine, MLine as DwgMLine, MLineFlags, MLineJustification as DwgMLineJustification,
    MLineSegment, Point as DwgPoint, Ray as DwgRay, Shape as DwgShape, Tolerance as DwgTolerance,
    XLine as DwgXLine, CENTERLINE_XDATA_APPLICATION, CENTERMARK_XDATA_APPLICATION,
};
use acadrust::objects::{
    MLineStyle as DwgMLineStyle, MLineStyleElement as DwgMLineStyleElement,
    MLineStyleFlags as DwgMLineStyleFlags, ObjectType,
};
use acadrust::tables::{AppId, BlockRecord, TextStyle as DwgTextStyle};
use acadrust::types::{Color as DwgColor, Handle, Vector3};
use acadrust::CadDocument;

use super::{point, vector};
use crate::description::{normalize_degrees, sin_cos_degrees};
use crate::model::{
    CenterLine, CenterMark, MLine, MLineElement, MLineJustification, MLineStyle, ObjectColor,
    ObjectLinetype, Point, Point3, Properties, Ray, Sheet, ShxShape, Tolerance, XLine, BY_BLOCK,
    BY_LAYER, LAYER_ZERO, STANDARD_MLINE_STYLE,
};

fn dwg_color(color: ObjectColor) -> DwgColor {
    match color {
        ObjectColor::ByLayer => DwgColor::ByLayer,
        ObjectColor::ByBlock => DwgColor::ByBlock,
        ObjectColor::Index(i) => DwgColor::Index(i),
        ObjectColor::Rgb([r, g, b]) => DwgColor::Rgb { r, g, b },
    }
}

fn read_dwg_color(color: DwgColor) -> Option<ObjectColor> {
    match color {
        DwgColor::ByLayer => Some(ObjectColor::ByLayer),
        DwgColor::ByBlock => Some(ObjectColor::ByBlock),
        DwgColor::Index(i) if i > 0 => Some(ObjectColor::Index(i)),
        DwgColor::Rgb { r, g, b } => Some(ObjectColor::Rgb([r, g, b])),
        _ => None,
    }
}

fn dwg_linetype_name(lt: &ObjectLinetype) -> String {
    match lt {
        ObjectLinetype::ByLayer => BY_LAYER.to_owned(),
        ObjectLinetype::ByBlock => BY_BLOCK.to_owned(),
        ObjectLinetype::Named(n) => n.clone(),
    }
}

fn read_dwg_linetype(name: &str) -> ObjectLinetype {
    if name.eq_ignore_ascii_case(BY_LAYER) || name.is_empty() {
        ObjectLinetype::ByLayer
    } else if name.eq_ignore_ascii_case(BY_BLOCK) {
        ObjectLinetype::ByBlock
    } else {
        ObjectLinetype::Named(name.to_owned())
    }
}

/// Writes sheet-level `PDMODE` and `PDSIZE` into the document header.
pub(super) fn write_point_style(drawing: &mut CadDocument, pdmode: i16, pdsize: f64) {
    drawing.header.point_display_mode = pdmode;
    drawing.header.point_display_size = pdsize;
}

/// Ensures `OCS_CENTERLINE` or `OCS_CENTERMARK` is registered in `app_ids`.
pub(super) fn ensure_app_id(drawing: &mut CadDocument, name: &str) {
    if !drawing
        .app_ids
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(name))
    {
        let _ = drawing.app_ids.add(AppId::new(name));
    }
}

/// Resolves a linetype name for an `MLineStyleElement` against `drawing.line_types`
/// so R2018+ hard-pointer lookup always finds an existing linetype handle.
fn dwg_mline_element_linetype(drawing: &CadDocument, lt: &ObjectLinetype) -> String {
    let raw = dwg_linetype_name(lt);
    drawing
        .line_types
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(&raw))
        .map(|entry| entry.name.clone())
        .unwrap_or(raw)
}

/// Writes `mline_styles` into `ACAD_MLINESTYLE`, updating `"Standard"` if listed
/// and adding custom styles. Returns a lowercase-name -> handle map.
pub(super) fn add_mline_styles(
    drawing: &mut CadDocument,
    styles: &[MLineStyle],
) -> HashMap<String, Handle> {
    let mut handles: HashMap<String, Handle> = HashMap::new();
    let dict_handle = drawing.header.acad_mlinestyle_dict_handle;

    // Seed with existing MLINESTYLE entries in the document (including Standard).
    for (h, obj) in &drawing.objects {
        if let ObjectType::MLineStyle(st) = obj {
            handles.insert(st.name.to_lowercase(), *h);
        }
    }

    for style in styles {
        let mut dwg_style = DwgMLineStyle::new(&style.name);
        dwg_style.description.clone_from(&style.description);
        dwg_style.start_angle = style.start_angle.to_radians();
        dwg_style.end_angle = style.end_angle.to_radians();
        dwg_style.fill_color = style.fill_color.map(dwg_color).unwrap_or(DwgColor::ByLayer);
        dwg_style.flags = DwgMLineStyleFlags {
            fill_on: style.fill_color.is_some(),
            display_joints: style.display_joints,
            start_square_cap: style.start_square_cap,
            start_inner_arcs_cap: style.start_inner_arcs_cap,
            start_round_cap: style.start_round_cap,
            end_square_cap: style.end_square_cap,
            end_inner_arcs_cap: style.end_inner_arcs_cap,
            end_round_cap: style.end_round_cap,
        };
        for elem in &style.elements {
            let lt_name = dwg_mline_element_linetype(drawing, &elem.linetype);
            dwg_style.add_element(DwgMLineStyleElement::full(
                elem.offset,
                dwg_color(elem.color),
                lt_name,
            ));
        }

        let key = style.name.to_lowercase();
        if let Some(&existing_handle) = handles.get(&key) {
            dwg_style.handle = existing_handle;
            dwg_style.owner = dict_handle;
            drawing
                .objects
                .insert(existing_handle, ObjectType::MLineStyle(dwg_style));
        } else {
            let h = drawing.allocate_handle();
            dwg_style.handle = h;
            dwg_style.owner = dict_handle;
            drawing.objects.insert(h, ObjectType::MLineStyle(dwg_style));
            if let Some(ObjectType::Dictionary(dict)) = drawing.objects.get_mut(&dict_handle) {
                dict.add_entry(&style.name, h);
            }
            handles.insert(key, h);
        }
    }

    handles
}

/// Reads all multiline styles from `drawing`'s `ACAD_MLINESTYLE` dictionary.
pub(super) fn read_mline_styles(drawing: &CadDocument) -> Vec<MLineStyle> {
    let mut out = Vec::new();
    let dict_handle = drawing.header.acad_mlinestyle_dict_handle;
    let entry_handles: Vec<Handle> = match drawing.objects.get(&dict_handle) {
        Some(ObjectType::Dictionary(d)) => d.entries.iter().map(|(_, h)| *h).collect(),
        _ => drawing
            .objects
            .iter()
            .filter_map(|(h, o)| matches!(o, ObjectType::MLineStyle(_)).then_some(*h))
            .collect(),
    };
    let std_default = MLineStyle::standard();
    for h in entry_handles {
        let Some(ObjectType::MLineStyle(st)) = drawing.objects.get(&h) else {
            continue;
        };
        let elements: Vec<MLineElement> = st
            .elements
            .iter()
            .map(|e| MLineElement {
                offset: e.offset,
                color: read_dwg_color(e.color).unwrap_or(ObjectColor::ByLayer),
                linetype: read_dwg_linetype(&e.linetype),
            })
            .collect();
        let fill_color = st
            .flags
            .fill_on
            .then(|| read_dwg_color(st.fill_color).unwrap_or(ObjectColor::ByLayer));
        let model = MLineStyle {
            name: st.name.clone(),
            description: st.description.clone(),
            elements,
            fill_color,
            display_joints: st.flags.display_joints,
            start_angle: st.start_angle.to_degrees(),
            end_angle: st.end_angle.to_degrees(),
            start_square_cap: st.flags.start_square_cap,
            start_round_cap: st.flags.start_round_cap,
            start_inner_arcs_cap: st.flags.start_inner_arcs_cap,
            end_square_cap: st.flags.end_square_cap,
            end_round_cap: st.flags.end_round_cap,
            end_inner_arcs_cap: st.flags.end_inner_arcs_cap,
        };
        // Omit unmodified built-in Standard style so plain sheets stay clean.
        if model.name.eq_ignore_ascii_case(STANDARD_MLINE_STYLE) && model == std_default {
            continue;
        }
        out.push(model);
    }
    out
}

pub(super) fn dwg_point(p: &Point) -> DwgPoint {
    let mut out = DwgPoint::at(vector(p.position));
    out.x_axis_angle = p.angle.to_radians();
    out
}

pub(super) fn read_point(p: &DwgPoint, layer: String, properties: Properties) -> Point {
    Point {
        position: point(p.location),
        angle: normalize_degrees(p.x_axis_angle.to_degrees()),
        layer,
        properties,
    }
}

pub(super) fn dwg_xline(xl: &XLine) -> DwgXLine {
    DwgXLine::new(vector(xl.through), vector(xl.unit_direction()))
}

pub(super) fn read_xline(xl: &DwgXLine, layer: String, properties: Properties) -> XLine {
    XLine {
        through: point(xl.base_point),
        direction: crate::model::normalize_point3(point(xl.direction)),
        layer,
        properties,
    }
}

pub(super) fn dwg_ray(r: &Ray) -> DwgRay {
    DwgRay::new(vector(r.from), vector(r.unit_direction()))
}

pub(super) fn read_ray(r: &DwgRay, layer: String, properties: Properties) -> Ray {
    Ray {
        from: point(r.base_point),
        direction: crate::model::normalize_point3(point(r.direction)),
        layer,
        properties,
    }
}

/// Converts a model [`MLine`] into an `acadrust::entities::MLine` with valid
/// direction vectors, miter vectors, and per-element segment parameters so
/// AutoCAD renders and audits it cleanly.
pub(super) fn dwg_mline(
    mline: &MLine,
    style: &MLineStyle,
    style_handle: Option<Handle>,
) -> DwgMLine {
    let mut out = DwgMLine::new();
    out.style_name.clone_from(&style.name);
    out.style_handle = style_handle;
    out.scale_factor = mline.scale;
    out.justification = match mline.justification {
        MLineJustification::Top => DwgMLineJustification::Top,
        MLineJustification::Zero => DwgMLineJustification::Zero,
        MLineJustification::Bottom => DwgMLineJustification::Bottom,
    };
    out.style_element_count = style.elements.len();
    for &pt in &mline.points {
        out.add_vertex(vector(pt));
    }
    if mline.closed {
        out.close();
    }
    if mline.suppress_start_caps {
        out.suppress_start_caps();
    }
    if mline.suppress_end_caps {
        out.suppress_end_caps();
    }

    // Recompute exact AutoCAD miter directions and segment parameters for every vertex.
    let n = out.vertices.len();
    if n >= 2 && !style.elements.is_empty() {
        let normal = Vector3::UNIT_Z;
        let closed = out.is_closed() && n >= 3;
        if !closed {
            // Apply style start_angle and end_angle to the open end miters.
            let d0 = out.vertices[0].direction;
            let (s0, c0) = style.start_angle.to_radians().sin_cos();
            out.vertices[0].miter =
                Vector3::new(d0.x * c0 - d0.y * s0, d0.x * s0 + d0.y * c0, 0.0).normalize();
            let dl = out.vertices[n - 1].direction;
            let (sl, cl) = style.end_angle.to_radians().sin_cos();
            out.vertices[n - 1].miter =
                Vector3::new(dl.x * cl - dl.y * sl, dl.x * sl + dl.y * cl, 0.0).normalize();
        }
        for v in &mut out.vertices {
            let perp = normal.cross(&v.direction).normalize();
            let dot = perp.dot(&v.miter);
            let denom = if dot.abs() > 0.1 { dot } else { 1.0 };
            v.segments.clear();
            for elem in &style.elements {
                let eff = mline.effective_offset(elem.offset, style);
                let mut seg = MLineSegment::new();
                seg.add_parameter(eff / denom);
                seg.add_parameter(0.0);
                v.segments.push(seg);
            }
        }
    }
    out
}

pub(super) fn read_mline(ml: &DwgMLine, layer: String, properties: Properties) -> MLine {
    let points: Vec<Point3> = ml.vertices.iter().map(|v| point(v.position)).collect();
    let justification = match ml.justification {
        DwgMLineJustification::Top => MLineJustification::Top,
        DwgMLineJustification::Zero => MLineJustification::Zero,
        DwgMLineJustification::Bottom => MLineJustification::Bottom,
    };
    MLine {
        points,
        style: if ml.style_name.is_empty() {
            STANDARD_MLINE_STYLE.to_owned()
        } else {
            ml.style_name.clone()
        },
        scale: ml.scale_factor,
        justification,
        closed: ml.flags.contains(MLineFlags::CLOSED),
        suppress_start_caps: ml.flags.contains(MLineFlags::NO_START_CAPS),
        suppress_end_caps: ml.flags.contains(MLineFlags::NO_END_CAPS),
        layer,
        properties,
    }
}

pub(super) fn dwg_tolerance(
    drawing: &CadDocument,
    tol: &Tolerance,
    sheet: Option<&Sheet>,
) -> DwgTolerance {
    let (sin, cos) = sin_cos_degrees(tol.direction);
    let mut out = DwgTolerance::new();
    out.insertion_point = vector(tol.position);
    out.direction = Vector3::new(cos, sin, 0.0);
    out.normal = Vector3::UNIT_Z;
    out.text.clone_from(&tol.text);
    let style_handle = drawing
        .dim_styles
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(&tol.dimension_style))
        .or_else(|| drawing.dim_styles.iter().next());
    if let Some(ds) = style_handle {
        out.dimension_style_name.clone_from(&ds.name);
        out.dimension_style_handle = Some(ds.handle);
        out.text_height = tol.text_height.unwrap_or(ds.dimtxt);
        out.dimension_gap = ds.dimgap;
    } else {
        out.dimension_style_name.clone_from(&tol.dimension_style);
        if let Some(h) = tol.text_height {
            out.text_height = h;
        } else if let Some(s) = sheet.and_then(|sh| sh.dimension_style(&tol.dimension_style)) {
            out.text_height = s.text_height * s.scale;
        }
    }
    out
}

pub(super) fn read_tolerance(
    drawing: &CadDocument,
    tol: &DwgTolerance,
    layer: String,
    properties: Properties,
) -> Tolerance {
    let style_name = tol
        .dimension_style_handle
        .and_then(|h| drawing.dim_styles.iter().find(|d| d.handle == h))
        .map(|d| d.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            if tol.dimension_style_name.is_empty() {
                crate::model::STANDARD_DIMENSION_STYLE.to_owned()
            } else {
                tol.dimension_style_name.clone()
            }
        });
    let dir_deg = normalize_degrees(tol.direction.y.atan2(tol.direction.x).to_degrees());
    Tolerance {
        position: point(tol.insertion_point),
        direction: dir_deg,
        dimension_style: style_name,
        text: tol.text.clone(),
        text_height: None,
        layer,
        properties,
    }
}

/// Writes a [`CenterMark`] as an anonymous `*U<n>` block `INSERT` holding the
/// crosshair and extension lines, tagged with `OCS_CENTERMARK` XDATA.
pub(super) fn dwg_center_mark(
    drawing: &mut CadDocument,
    cm: &CenterMark,
    counter: &mut usize,
) -> DwgInsert {
    ensure_app_id(drawing, CENTERMARK_XDATA_APPLICATION);
    let block_name = loop {
        *counter += 1;
        let candidate = format!("*U{}", *counter);
        if drawing.block_records.get(&candidate).is_none() {
            break candidate;
        }
    };
    let owner = drawing.allocate_handle();
    let mut record = BlockRecord::new(block_name.clone());
    record.handle = owner;
    record.block_entity_handle = drawing.allocate_handle();
    record.block_end_handle = drawing.allocate_handle();
    record.flags.anonymous = true;
    record.units = drawing.header.insertion_units;
    let _ = drawing.block_records.add(record);

    // Emit lines in block-local coordinates relative to `cm.center` so moving
    // the insert moves the whole centre mark cleanly in AutoCAD.
    let mut child_handles: Vec<Handle> = Vec::new();
    for [a, b] in cm.segments() {
        let mut line = DwgLine::new();
        line.start = Vector3::new(a.x - cm.center.x, a.y - cm.center.y, a.z - cm.center.z);
        line.end = Vector3::new(b.x - cm.center.x, b.y - cm.center.y, b.z - cm.center.z);
        line.common.owner_handle = owner;
        line.common.layer = LAYER_ZERO.to_owned();
        line.common.color = DwgColor::ByBlock;
        line.common.linetype = "ByBlock".to_owned();
        if let Ok(h) = drawing.add_entity(EntityType::Line(line)) {
            child_handles.push(h);
        }
    }
    if let Some(br) = drawing.block_records.get_mut(&block_name) {
        for h in child_handles {
            if !br.entity_handles.contains(&h) {
                br.entity_handles.push(h);
            }
        }
    }

    let mut ins = DwgInsert::new(block_name, vector(cm.center));
    let (sin, cos) = sin_cos_degrees(cm.rotation);
    let assoc = CenterMarkAssociation {
        source: CenterMarkSource {
            handle: Handle::NULL,
            kind: CenterMarkSourceKind::Circle,
            segment_index: 0,
            pick_point: vector(cm.center),
        },
        plane_origin: vector(cm.center),
        plane_x: Vector3::new(cos, sin, 0.0),
        plane_y: Vector3::new(-sin, cos, 0.0),
        center: vector(cm.center),
        radius: cm.radius,
        cross_size: cm.effective_cross_size(),
        cross_gap: cm.effective_cross_gap(),
        cross_size_relative: false,
        cross_gap_relative: false,
        extension_length: cm.effective_extension(),
        length_adjustments: [0.0; 4],
        overshoots: [0.0; 4],
        show_extensions: cm.show_extensions,
        associated: false,
    };
    assoc.write(&mut ins.common.extended_data);
    ins
}

/// Attempts to decode an `INSERT` with `OCS_CENTERMARK` XDATA as a [`CenterMark`].
pub(super) fn try_read_center_mark(
    ins: &DwgInsert,
    layer: String,
    properties: Properties,
) -> Option<CenterMark> {
    let assoc = CenterMarkAssociation::read(&ins.common.extended_data)?;
    let rot_deg = normalize_degrees(
        assoc.plane_x.y.atan2(assoc.plane_x.x).to_degrees() + ins.rotation.to_degrees(),
    );
    let scale = ins.x_scale().abs().max(1e-9);
    Some(CenterMark {
        center: point(ins.insert_point),
        radius: (assoc.radius * scale).max(1e-6),
        cross_size: Some((assoc.cross_size * scale).max(1e-6)),
        cross_gap: Some((assoc.cross_gap * scale).max(0.0)),
        extension: Some((assoc.extension_length * scale).max(0.0)),
        show_extensions: assoc.show_extensions,
        rotation: rot_deg,
        layer,
        properties,
    })
}

/// Writes a [`CenterLine`] as a `LINE` spanning its extended endpoints, tagged
/// with `OCS_CENTERLINE` XDATA.
pub(super) fn dwg_center_line(drawing: &mut CadDocument, cl: &CenterLine) -> DwgLine {
    ensure_app_id(drawing, CENTERLINE_XDATA_APPLICATION);
    let (p0, p1) = cl.extended_endpoints();
    let mut line = DwgLine::from_points(vector(p0), vector(p1));
    let assoc = CenterLineAssociation {
        first: CenterLineSource {
            handle: Handle::NULL,
            kind: CenterLineSourceKind::Line,
            segment_index: 0,
            pick_point: vector(cl.from),
        },
        second: CenterLineSource {
            handle: Handle::NULL,
            kind: CenterLineSourceKind::Line,
            segment_index: 0,
            pick_point: vector(cl.to),
        },
        plane_origin: vector(cl.from),
        plane_x: Vector3::UNIT_X,
        plane_y: Vector3::UNIT_Y,
        start_extension: cl.start_extension,
        end_extension: cl.end_extension,
        start_length_adjustment: 0.0,
        end_length_adjustment: 0.0,
        associated: false,
    };
    assoc.write(&mut line.common.extended_data);
    line
}

/// Attempts to decode a `LINE` with `OCS_CENTERLINE` XDATA as a [`CenterLine`].
pub(super) fn try_read_center_line(
    line: &DwgLine,
    layer: String,
    properties: Properties,
) -> Option<CenterLine> {
    let assoc = CenterLineAssociation::read(&line.common.extended_data)?;
    let dx = line.end.x - line.start.x;
    let dy = line.end.y - line.start.y;
    let dz = line.end.z - line.start.z;
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    let (from, to) = if len > 1e-12 && (assoc.start_extension != 0.0 || assoc.end_extension != 0.0)
    {
        let ux = dx / len;
        let uy = dy / len;
        let uz = dz / len;
        (
            Point3::new(
                line.start.x + ux * assoc.start_extension,
                line.start.y + uy * assoc.start_extension,
                line.start.z + uz * assoc.start_extension,
            ),
            Point3::new(
                line.end.x - ux * assoc.end_extension,
                line.end.y - uy * assoc.end_extension,
                line.end.z - uz * assoc.end_extension,
            ),
        )
    } else {
        (point(line.start), point(line.end))
    };
    Some(CenterLine {
        from,
        to,
        start_extension: assoc.start_extension,
        end_extension: assoc.end_extension,
        layer,
        properties,
    })
}

/// Ensures a `TextStyle` with `is_shape_file = true` exists in `drawing.text_styles`
/// for `file` and returns `(style_name, style_handle)`.
pub(super) fn ensure_shape_style(drawing: &mut CadDocument, file: &str) -> (String, Handle) {
    if let Some(existing) = drawing
        .text_styles
        .iter()
        .find(|ts| ts.is_shape_file && ts.font_file.eq_ignore_ascii_case(file))
    {
        return (existing.name.clone(), existing.handle);
    }
    // AutoCAD shape-file STYLE records carry `is_shape_file = true`, `font_file = "<file>.shx"`.
    let base: String = file
        .trim_end_matches([' ', '\t'])
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut candidate = format!("SHX_{base}");
    let mut n = 1usize;
    while drawing
        .text_styles
        .iter()
        .any(|ts| ts.name.eq_ignore_ascii_case(&candidate))
    {
        n += 1;
        candidate = format!("SHX_{base}_{n}");
    }
    let handle = drawing.allocate_handle();
    let mut ts = DwgTextStyle::new(&candidate);
    ts.handle = handle;
    ts.is_shape_file = true;
    ts.font_file = file.to_owned();
    ts.true_type_font.clear();
    let _ = drawing.text_styles.add(ts);
    (candidate, handle)
}

pub(super) fn dwg_shape(
    drawing: &mut CadDocument,
    sh: &ShxShape,
    sheet: Option<&Sheet>,
) -> DwgShape {
    let (style_name, style_handle) = ensure_shape_style(drawing, &sh.file);
    let resolved = sheet.and_then(|s| s.resolved_shape_for(sh));
    let number = sh
        .number
        .or_else(|| resolved.map(|r| r.number))
        .unwrap_or(1);
    let shape_name = if !sh.name.is_empty() {
        sh.name.clone()
    } else if let Some(r) = resolved {
        r.name.clone()
    } else {
        format!("SHAPE_{number}")
    };
    let mut out = DwgShape::new();
    out.insertion_point = vector(sh.position);
    out.size = sh.size;
    out.shape_name = shape_name;
    out.shape_number = number;
    out.rotation = sh.rotation.to_radians();
    out.relative_x_scale = sh.width_factor;
    out.oblique_angle = sh.oblique.to_radians();
    out.normal = Vector3::UNIT_Z;
    out.style_name = style_name;
    out.style_handle = Some(style_handle);
    out
}

pub(super) fn read_shape(
    drawing: &CadDocument,
    sh: &DwgShape,
    layer: String,
    properties: Properties,
) -> ShxShape {
    let file = sh
        .style_handle
        .and_then(|h| drawing.text_styles.iter().find(|ts| ts.handle == h))
        .or_else(|| {
            if !sh.style_name.is_empty() {
                drawing
                    .text_styles
                    .iter()
                    .find(|ts| ts.name.eq_ignore_ascii_case(&sh.style_name))
            } else {
                None
            }
        })
        .or_else(|| drawing.text_styles.iter().find(|ts| ts.is_shape_file))
        .map(|ts| ts.font_file.clone())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "ltypeshp.shx".to_owned());
    let number = (sh.shape_number > 0).then_some(sh.shape_number);
    ShxShape {
        file,
        name: sh.shape_name.clone(),
        number,
        position: point(sh.insertion_point),
        size: sh.size.max(1e-6),
        rotation: normalize_degrees(sh.rotation.to_degrees()),
        width_factor: if sh.relative_x_scale > 0.0 {
            sh.relative_x_scale
        } else {
            1.0
        },
        oblique: normalize_degrees(sh.oblique_angle.to_degrees()),
        layer,
        properties,
    }
}

/// Checks whether a `BlockRecord` is an anonymous block owned by a `CenterMark`
/// insert so `read_dwg` does not count it as an unread block definition.
pub(super) fn is_center_mark_block(drawing: &CadDocument, block_name: &str) -> bool {
    if !block_name.starts_with('*') {
        return false;
    }
    drawing.entities().any(|e| match e {
        EntityType::Insert(ins) => {
            ins.block_name.eq_ignore_ascii_case(block_name)
                && CenterMarkAssociation::read(&ins.common.extended_data).is_some()
        }
        _ => false,
    })
}

/// Removes `OCS_CENTERLINE` / `OCS_CENTERMARK` raw EED blobs if needed.
#[allow(dead_code)]
pub(super) fn clear_common_raw(common: &mut EntityCommon) {
    common.raw_record = None;
}
