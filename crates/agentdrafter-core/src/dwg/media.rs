//! Raster images (`IMAGE` / `IMAGEDEF` / `IMAGEDEF_REACTOR` / `RASTERVARIABLES`)
//! and PDF underlays (`PDFUNDERLAY` / `PDFDEFINITION`) behind the DWG/DXF seam (#91).

use std::path::Path;

use acadrust::entities::{
    ClipBoundary, ClipMode, ClipType, EntityType, ImageDisplayFlags, RasterImage as DwgRasterImage,
    Underlay as DwgUnderlay, UnderlayDefinition, UnderlayDisplayFlags, UnderlayType,
};
use acadrust::objects::{
    Dictionary, ImageDefinition, ImageDefinitionReactor, ObjectType, RasterVariables,
    ResolutionUnit,
};
use acadrust::types::{Handle, Vector2, Vector3};
use acadrust::CadDocument;

use super::wipeout::root_dictionary_handle;
use super::{normalize_degrees, point, vector};
use crate::model::{PdfUnderlay, Properties, RasterImage, WipeoutFrame};
use crate::units::Unit;

/// Named Object Dictionary key for AutoCAD's raster image definition dictionary.
pub(super) const ACAD_IMAGE_DICT: &str = "ACAD_IMAGE_DICT";

/// Named Object Dictionary key for AutoCAD's raster image variables (`IMAGEFRAME`).
pub(super) const ACAD_IMAGE_VARS: &str = "ACAD_IMAGE_VARS";

/// Named Object Dictionary key for AutoCAD's PDF underlay definition dictionary.
pub(super) const ACAD_PDFDEFINITIONS: &str = "ACAD_PDFDEFINITIONS";

/// Non-graphical object handles touched when attaching a raster image or PDF
/// underlay to a drawing, used by `dwg/revise.rs` to verify untouched records.
#[derive(Clone, Debug, Default)]
pub(super) struct MediaHandles {
    pub(super) added: Vec<u64>,
    pub(super) changed: Vec<u64>,
}

/// AutoCAD `RASTERVARIABLES.units` code for a sheet unit.
fn raster_units_code(unit: Unit) -> i16 {
    match unit {
        Unit::Mm => 1,
        Unit::Cm => 2,
        Unit::M => 3,
        Unit::In => 5,
        Unit::Ft => 6,
    }
}

/// Reads `ACAD_IMAGE_VARS` (`RASTERVARIABLES.display_image_frame`) from `drawing`.
/// Returns `WipeoutFrame::Off` when absent so images without explicit frame
/// variables don't get unwanted borders unless requested.
pub(super) fn read_image_frame(drawing: &CadDocument) -> WipeoutFrame {
    if let Some(root) = root_dictionary_handle(drawing) {
        if let Some(ObjectType::Dictionary(d)) = drawing.objects.get(&root) {
            if let Some(vars_handle) = d
                .entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(ACAD_IMAGE_VARS))
                .map(|(_, h)| *h)
            {
                if let Some(ObjectType::RasterVariables(rv)) = drawing.objects.get(&vars_handle) {
                    return WipeoutFrame::from_dwg_code(rv.display_image_frame);
                }
            }
        }
    }
    for obj in drawing.objects.values() {
        if let ObjectType::RasterVariables(rv) = obj {
            return WipeoutFrame::from_dwg_code(rv.display_image_frame);
        }
    }
    WipeoutFrame::Off
}

/// Writes or updates `ACAD_IMAGE_VARS` (`RASTERVARIABLES`) in `drawing`'s root
/// dictionary.
pub(super) fn write_image_frame(
    drawing: &mut CadDocument,
    frame: WipeoutFrame,
    units: Unit,
) -> Option<(Handle, Handle, bool)> {
    let root = root_dictionary_handle(drawing)?;
    let existing = match drawing.objects.get(&root) {
        Some(ObjectType::Dictionary(d)) => d
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(ACAD_IMAGE_VARS))
            .map(|(_, h)| *h),
        _ => None,
    };
    if let Some(handle) = existing {
        if let Some(ObjectType::RasterVariables(rv)) = drawing.objects.get_mut(&handle) {
            rv.display_image_frame = frame.dwg_code();
            rv.units = raster_units_code(units);
            return Some((root, handle, false));
        }
    }
    let handle = drawing.allocate_handle();
    let mut rv = RasterVariables::new();
    rv.handle = handle;
    rv.owner = root;
    rv.display_image_frame = frame.dwg_code();
    rv.image_quality = 1;
    rv.units = raster_units_code(units);
    drawing
        .objects
        .insert(handle, ObjectType::RasterVariables(rv));
    if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&root) {
        d.add_entry(ACAD_IMAGE_VARS, handle);
    }
    Some((root, handle, true))
}

/// Ensures a sub-dictionary `dict_name` exists under the root Named Objects
/// Dictionary, returning its handle and recording mutations in `handles`.
fn ensure_root_subdict(
    drawing: &mut CadDocument,
    dict_name: &str,
    handles: &mut MediaHandles,
) -> Option<Handle> {
    let root = root_dictionary_handle(drawing)?;
    let existing = match drawing.objects.get(&root) {
        Some(ObjectType::Dictionary(d)) => d
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(dict_name))
            .map(|(_, h)| *h),
        _ => None,
    };
    if let Some(h) = existing {
        if matches!(drawing.objects.get(&h), Some(ObjectType::Dictionary(_))) {
            return Some(h);
        }
    }
    let dict_handle = drawing.allocate_handle();
    let mut sub = Dictionary::new();
    sub.handle = dict_handle;
    sub.owner = root;
    drawing
        .objects
        .insert(dict_handle, ObjectType::Dictionary(sub));
    if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&root) {
        d.add_entry(dict_name, dict_handle);
    }
    handles.changed.push(root.value());
    handles.added.push(dict_handle.value());
    Some(dict_handle)
}

/// Generates a clean, unique dictionary entry key from a file path stem.
fn unique_dict_key(dict: &Dictionary, base: &str) -> String {
    let stem: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = if stem.is_empty() {
        "MEDIA".to_owned()
    } else {
        stem
    };
    if !dict
        .entries
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case(&stem))
    {
        return stem;
    }
    for n in 2..=10_000 {
        let candidate = format!("{stem}_{n}");
        if !dict
            .entries
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
    }
    format!("{stem}_{}", dict.entries.len() + 1)
}

/// Builds an `acadrust::entities::RasterImage` and wires its `ACAD_IMAGE_DICT`
/// `ImageDefinition` + `ImageDefinitionReactor` into `drawing`.
///
/// The caller must pass the pre-allocated `entity_handle` that the returned
/// entity will be stored under (so `IMAGEDEF_REACTOR.owner == entity_handle`).
pub(super) fn dwg_raster_image(
    drawing: &mut CadDocument,
    img: &RasterImage,
    natural_px: Option<(u32, u32)>,
    entity_handle: Handle,
) -> (EntityType, MediaHandles) {
    let mut handles = MediaHandles::default();
    let (w_px, h_px) = natural_px
        .filter(|&(w, h)| w > 0 && h > 0)
        .unwrap_or((1000, 1000));
    let (w_f, h_f) = (f64::from(w_px), f64::from(h_px));
    let (u_span, v_span) = img.axes(Some((w_px, h_px)));

    // 1. Ensure ACAD_IMAGE_DICT and find or create IMAGEDEF for `img.path`.
    let mut def_handle = None;
    let mut reactor_handle = None;
    if let Some(dict_handle) = ensure_root_subdict(drawing, ACAD_IMAGE_DICT, &mut handles) {
        let existing_def = drawing.objects.iter().find_map(|(h, obj)| match obj {
            ObjectType::ImageDefinition(d) if d.owner == dict_handle && d.file_name == img.path => {
                Some(*h)
            }
            _ => None,
        });
        let dh = match existing_def {
            Some(dh) => {
                handles.changed.push(dh.value());
                dh
            }
            None => {
                let dh = drawing.allocate_handle();
                let mut def = ImageDefinition::with_dimensions(&img.path, w_px, h_px);
                def.handle = dh;
                def.owner = dict_handle;
                def.is_loaded = true;
                def.pixel_size = (1.0 / w_f, 1.0 / w_f);
                def.resolution_unit = ResolutionUnit::None;
                def.reactors = vec![dict_handle];
                let stem = Path::new(&img.path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("IMAGE");
                let key = match drawing.objects.get(&dict_handle) {
                    Some(ObjectType::Dictionary(d)) => unique_dict_key(d, stem),
                    _ => stem.to_owned(),
                };
                drawing.objects.insert(dh, ObjectType::ImageDefinition(def));
                if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&dict_handle) {
                    d.add_entry(key, dh);
                }
                handles.changed.push(dict_handle.value());
                handles.added.push(dh.value());
                dh
            }
        };

        // 2. Create IMAGEDEF_REACTOR owned by the IMAGE entity.
        let rh = drawing.allocate_handle();
        let reactor = ImageDefinitionReactor {
            handle: rh,
            owner: entity_handle,
            image_handle: entity_handle,
        };
        drawing
            .objects
            .insert(rh, ObjectType::ImageDefinitionReactor(reactor));
        handles.added.push(rh.value());

        if let Some(ObjectType::ImageDefinition(def)) = drawing.objects.get_mut(&dh) {
            if !def.reactors.contains(&rh) {
                def.reactors.push(rh);
            }
        }
        def_handle = Some(dh);
        reactor_handle = Some(rh);
    }

    // 3. Build the RasterImage entity.
    let mut entity = DwgRasterImage::new(&img.path, vector(img.position), w_f, h_f);
    entity.common.handle = entity_handle;
    entity.u_vector = Vector3::new(u_span[0] / w_f, u_span[1] / w_f, 0.0);
    entity.v_vector = Vector3::new(v_span[0] / h_f, v_span[1] / h_f, 0.0);
    let mut flags = ImageDisplayFlags::SHOW_NOT_ALIGNED;
    if img.show {
        flags |= ImageDisplayFlags::SHOW_IMAGE;
    }
    if img.transparent {
        flags |= ImageDisplayFlags::TRANSPARENCY_ON;
    }
    if let Some(clip) = &img.clip {
        if clip.len() >= 3 {
            flags |= ImageDisplayFlags::USE_CLIPPING_BOUNDARY;
            entity.clipping_enabled = true;
            let det = entity.u_vector.x * entity.v_vector.y - entity.u_vector.y * entity.v_vector.x;
            if det.abs() > 1e-18 {
                let mut verts: Vec<Vector2> = clip
                    .iter()
                    .map(|p| {
                        let dx = p[0] - img.position.x;
                        let dy = p[1] - img.position.y;
                        let alpha = (dx * entity.v_vector.y - dy * entity.v_vector.x) / det;
                        let beta = (entity.u_vector.x * dy - entity.u_vector.y * dx) / det;
                        Vector2::new(alpha - 0.5, h_f - 0.5 - beta)
                    })
                    .collect();
                if let Some(&first) = verts.first() {
                    verts.push(first);
                }
                entity.clip_boundary = ClipBoundary {
                    clip_type: ClipType::Polygonal,
                    clip_mode: if img.clip_inverted {
                        ClipMode::Inside
                    } else {
                        ClipMode::Outside
                    },
                    vertices: verts,
                };
            }
        }
    } else {
        entity.clipping_enabled = false;
        entity.clip_boundary = ClipBoundary::full_image(w_f, h_f);
    }
    entity.flags = flags;
    entity.brightness = img.brightness;
    entity.contrast = img.contrast;
    entity.fade = img.fade;
    entity.definition_handle = def_handle;
    entity.definition_reactor_handle = reactor_handle;

    (EntityType::RasterImage(entity), handles)
}

/// Builds an `acadrust::entities::Underlay` (`PDFUNDERLAY`) and wires its
/// `ACAD_PDFDEFINITIONS` `UnderlayDefinition` (`PDFDEFINITION`) into `drawing`.
pub(super) fn dwg_pdf_underlay(
    drawing: &mut CadDocument,
    pdf: &PdfUnderlay,
    entity_handle: Handle,
) -> (EntityType, MediaHandles) {
    let mut handles = MediaHandles::default();
    let mut def_handle = Handle::NULL;
    if let Some(dict_handle) = ensure_root_subdict(drawing, ACAD_PDFDEFINITIONS, &mut handles) {
        let existing_def = drawing.objects.iter().find_map(|(h, obj)| match obj {
            ObjectType::UnderlayDefinition(d)
                if d.underlay_type == UnderlayType::Pdf
                    && d.owner_handle == dict_handle
                    && d.file_path == pdf.path
                    && d.page_name == pdf.page
                    && d.unloaded == pdf.unloaded =>
            {
                Some(*h)
            }
            _ => None,
        });
        let dh = match existing_def {
            Some(dh) => {
                handles.changed.push(dh.value());
                dh
            }
            None => {
                let dh = drawing.allocate_handle();
                let stem = Path::new(&pdf.path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("PDF");
                let base_key = format!("{stem}-{}", pdf.page);
                let key = match drawing.objects.get(&dict_handle) {
                    Some(ObjectType::Dictionary(d)) => unique_dict_key(d, &base_key),
                    _ => base_key,
                };
                let mut def = UnderlayDefinition::pdf(&pdf.path, &pdf.page);
                def.handle = dh;
                def.owner_handle = dict_handle;
                def.name = key.clone();
                def.unloaded = pdf.unloaded;
                def.reactors = vec![entity_handle];
                drawing
                    .objects
                    .insert(dh, ObjectType::UnderlayDefinition(def));
                if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&dict_handle) {
                    d.add_entry(key, dh);
                }
                handles.changed.push(dict_handle.value());
                handles.added.push(dh.value());
                dh
            }
        };
        if let Some(ObjectType::UnderlayDefinition(def)) = drawing.objects.get_mut(&dh) {
            if !def.reactors.contains(&entity_handle) {
                def.reactors.push(entity_handle);
            }
        }
        def_handle = dh;
    }

    let mut entity = DwgUnderlay::pdf();
    entity.common.handle = entity_handle;
    entity.definition_handle = def_handle;
    entity.insertion_point = vector(pdf.position);
    entity.set_scale(pdf.scale);
    entity.rotation = pdf.rotation.to_radians();
    entity.normal = Vector3::UNIT_Z;
    let mut flags = UnderlayDisplayFlags::NONE;
    if pdf.show {
        flags |= UnderlayDisplayFlags::ON;
    }
    if pdf.monochrome {
        flags |= UnderlayDisplayFlags::MONOCHROME;
    }
    if let Some(clip) = &pdf.clip {
        if clip.len() >= 3 {
            flags |= UnderlayDisplayFlags::CLIPPING;
            if pdf.clip_inverted {
                flags |= UnderlayDisplayFlags::CLIP_INSIDE;
            }
            let rad = -pdf.rotation.to_radians();
            let (sin, cos) = rad.sin_cos();
            let inv_scale = 1.0 / pdf.scale.max(1e-12);
            entity.clip_boundary_vertices = clip
                .iter()
                .map(|p| {
                    let dx = p[0] - pdf.position.x;
                    let dy = p[1] - pdf.position.y;
                    Vector2::new(
                        (dx * cos - dy * sin) * inv_scale,
                        (dx * sin + dy * cos) * inv_scale,
                    )
                })
                .collect();
            entity.clip_inverted = pdf.clip_inverted;
        }
    }
    entity.flags = flags;
    entity.contrast = pdf.contrast;
    entity.fade = pdf.fade;

    (EntityType::Underlay(entity), handles)
}

/// Reads an `acadrust::entities::RasterImage` into our [`RasterImage`] model.
pub(super) fn read_raster_image(
    drawing: &CadDocument,
    img: &DwgRasterImage,
    layer: String,
) -> Option<RasterImage> {
    let path = if !img.file_path.is_empty() {
        img.file_path.clone()
    } else if let Some(dh) = img.definition_handle {
        match drawing.objects.get(&dh) {
            Some(ObjectType::ImageDefinition(def)) => def.file_name.clone(),
            _ => String::new(),
        }
    } else {
        String::new()
    };
    if path.is_empty() {
        return None;
    }
    // Only 2D-plane images (u and v in XY plane, orthogonal or nearly so).
    if img.u_vector.z.abs() > 1e-6 || img.v_vector.z.abs() > 1e-6 {
        return None;
    }
    let w_px = img.size.x;
    let h_px = img.size.y;
    if !w_px.is_finite() || !h_px.is_finite() || w_px <= 0.0 || h_px <= 0.0 {
        return None;
    }
    let u_len = (img.u_vector.x.hypot(img.u_vector.y)) * w_px;
    let v_len = (img.v_vector.x.hypot(img.v_vector.y)) * h_px;
    if !u_len.is_finite() || !v_len.is_finite() || u_len <= 1e-12 || v_len <= 1e-12 {
        return None;
    }
    let rotation = normalize_degrees(img.u_vector.y.atan2(img.u_vector.x).to_degrees());

    let clip = if img.clipping_enabled
        && img.flags.contains(ImageDisplayFlags::USE_CLIPPING_BOUNDARY)
        && !img.clip_boundary.vertices.is_empty()
    {
        let pixel_to_world = |v: Vector2| -> [f64; 2] {
            let alpha = v.x + 0.5;
            let beta = h_px - 0.5 - v.y;
            [
                img.insertion_point.x + alpha * img.u_vector.x + beta * img.v_vector.x,
                img.insertion_point.y + alpha * img.u_vector.y + beta * img.v_vector.y,
            ]
        };
        let mut pts = match img.clip_boundary.clip_type {
            ClipType::Rectangular if img.clip_boundary.vertices.len() >= 2 => {
                let a = img.clip_boundary.vertices[0];
                let b = img.clip_boundary.vertices[1];
                vec![
                    pixel_to_world(Vector2::new(a.x, b.y)),
                    pixel_to_world(Vector2::new(b.x, b.y)),
                    pixel_to_world(Vector2::new(b.x, a.y)),
                    pixel_to_world(Vector2::new(a.x, a.y)),
                ]
            }
            ClipType::Polygonal if img.clip_boundary.vertices.len() >= 3 => img
                .clip_boundary
                .vertices
                .iter()
                .map(|&v| pixel_to_world(v))
                .collect(),
            _ => Vec::new(),
        };
        if pts.len() >= 4 {
            let first = pts[0];
            let last = *pts.last().unwrap();
            if (first[0] - last[0]).abs() <= 1e-9 && (first[1] - last[1]).abs() <= 1e-9 {
                pts.pop();
            }
        }
        (pts.len() >= 3).then_some(pts)
    } else {
        None
    };

    Some(RasterImage {
        path,
        position: point(img.insertion_point),
        width: u_len,
        height: Some(v_len),
        rotation,
        clip,
        clip_inverted: img.clip_boundary.clip_mode == ClipMode::Inside,
        brightness: img.brightness.min(100),
        contrast: img.contrast.min(100),
        fade: img.fade.min(100),
        transparent: img.flags.contains(ImageDisplayFlags::TRANSPARENCY_ON),
        show: img.flags.contains(ImageDisplayFlags::SHOW_IMAGE),
        layer,
        properties: Properties::default(),
    })
}

/// Reads an `acadrust::entities::Underlay` (`PDFUNDERLAY`) into our
/// [`PdfUnderlay`] model. Non-PDF underlays (DWF/DGN) return `None` so they
/// remain counted in `not_read` and preserved verbatim in revisions.
pub(super) fn read_pdf_underlay(
    drawing: &CadDocument,
    underlay: &DwgUnderlay,
    layer: String,
) -> Option<PdfUnderlay> {
    if underlay.underlay_type != UnderlayType::Pdf {
        return None;
    }
    if (underlay.normal.x.abs() > 1e-6)
        || (underlay.normal.y.abs() > 1e-6)
        || (underlay.normal.z - 1.0).abs() > 1e-6
    {
        return None;
    }
    let Some(ObjectType::UnderlayDefinition(def)) =
        drawing.objects.get(&underlay.definition_handle)
    else {
        return None;
    };
    if def.file_path.is_empty() {
        return None;
    }
    let scale = underlay.x_scale.abs();
    if !scale.is_finite() || scale <= 1e-12 {
        return None;
    }
    let rotation = normalize_degrees(underlay.rotation.to_degrees());
    let (sin, cos) = underlay.rotation.sin_cos();

    let clip = if underlay.flags.contains(UnderlayDisplayFlags::CLIPPING)
        && underlay.clip_boundary_vertices.len() >= 3
    {
        let mut pts: Vec<[f64; 2]> = underlay
            .clip_boundary_vertices
            .iter()
            .map(|v| {
                let sx = v.x * underlay.x_scale;
                let sy = v.y * underlay.y_scale;
                [
                    underlay.insertion_point.x + sx * cos - sy * sin,
                    underlay.insertion_point.y + sx * sin + sy * cos,
                ]
            })
            .collect();
        if pts.len() >= 4 {
            let first = pts[0];
            let last = *pts.last().unwrap();
            if (first[0] - last[0]).abs() <= 1e-9 && (first[1] - last[1]).abs() <= 1e-9 {
                pts.pop();
            }
        }
        (pts.len() >= 3).then_some(pts)
    } else {
        None
    };

    let page = if def.page_name.trim().is_empty() {
        "1".to_owned()
    } else {
        def.page_name.clone()
    };

    Some(PdfUnderlay {
        path: def.file_path.clone(),
        page,
        position: point(underlay.insertion_point),
        scale,
        rotation,
        clip,
        clip_inverted: underlay.clip_inverted
            || underlay.flags.contains(UnderlayDisplayFlags::CLIP_INSIDE),
        contrast: underlay.contrast.min(100),
        fade: underlay.fade.min(crate::model::MAX_PDF_FADE),
        monochrome: underlay.flags.contains(UnderlayDisplayFlags::MONOCHROME),
        show: underlay.flags.contains(UnderlayDisplayFlags::ON),
        unloaded: def.unloaded,
        layer,
        properties: Properties::default(),
    })
}
