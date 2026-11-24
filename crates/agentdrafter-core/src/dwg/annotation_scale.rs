//! AutoCAD annotation scale (`ACAD_SCALELIST`, `CANNOSCALE`, `ANNOALLVISIBLE`,
//! `AcadAnnotative` XDATA, and `AcDb*ObjectContextData` per-scale leaves)
//! behind the DWG seam (`crates/agentdrafter-core/src/dwg.rs`).

use std::collections::HashMap;

use acadrust::classes::{DxfClass, ProxyFlags};
use acadrust::entities::EntityType;
use acadrust::objects::{
    Dictionary, DictionaryVariable, DimContext, DimSubtype, HatchLoopContext, HatchScaleContext,
    MTextContext, ObjectContextData, ObjectContextKind, ObjectType, Scale,
};
use acadrust::types::{Handle, Vector2, Vector3};
use acadrust::xdata::{ExtendedData, ExtendedDataRecord, XDataValue};
use acadrust::CadDocument;

use crate::model::{AnnotationScale, Object, Sheet, DEFAULT_ANNOTATION_SCALE};

/// Registered application name AutoCAD uses to mark annotative entities,
/// styles, and block records.
pub(super) const ACAD_ANNOTATIVE: &str = "AcadAnnotative";
pub(super) const ANNOTATIVE_APP: &str = ACAD_ANNOTATIVE;
const ANNOTATIVE_DATA: &str = "AnnotativeData";

/// Root named-object dictionary key holding `SCALE` objects.
pub(super) const ACAD_SCALELIST: &str = "ACAD_SCALELIST";

/// Root named-object dictionary key holding `DICTIONARYVAR` entries such as
/// `CANNOSCALE` and `ANNOALLVISIBLE`.
pub(super) const VARIABLE_DICT: &str = "AcDbVariableDictionary";

const CONTEXT_DATA_MANAGER: &str = "AcDbContextDataManager";
const ANNOTATION_SCALES_DICT: &str = "ACDB_ANNOTATIONSCALES";
const CANNOSCALE_VAR: &str = "CANNOSCALE";
const ANNOALLVISIBLE_VAR: &str = "ANNOALLVISIBLE";

/// Whether `sheet` declares or uses any annotation-scale data.
/// When `false`, `write_new_sheet_as` omits `ACAD_SCALELIST` and
/// `AcDbVariableDictionary` so non-annotative DWGs stay byte-identical.
pub(super) fn sheet_uses_annotation_scales(sheet: &Sheet) -> bool {
    if !sheet.annotation_scales.is_empty()
        || sheet.annotation_scale.is_some()
        || sheet.anno_all_visible.is_some()
    {
        return true;
    }
    if sheet.text_styles.iter().any(|s| s.annotative)
        || sheet.dimension_styles.iter().any(|s| s.annotative)
        || sheet.mleader_styles.iter().any(|s| s.annotative)
        || sheet.blocks.iter().any(|b| b.annotative)
    {
        return true;
    }
    sheet.objects.iter().any(|o| {
        o.annotative_flag()
            || !o.annotation_scales().is_empty()
            || matches!(o, Object::Viewport(v) if v.annotation_scale.is_some())
    })
}

/// Ensures `dxf_name` is registered in `drawing.classes`.
fn ensure_dxf_class(drawing: &mut CadDocument, dxf_name: &str, cpp_name: &str, proxy_bits: u16) {
    if drawing.classes.get_by_name(dxf_name).is_some() {
        return;
    }
    drawing.classes.add_or_update(DxfClass {
        dxf_name: dxf_name.to_owned(),
        cpp_class_name: cpp_name.to_owned(),
        application_name: "ObjectDBX Classes".to_owned(),
        proxy_flags: ProxyFlags(proxy_bits),
        instance_count: 0,
        was_zombie: false,
        is_an_entity: false,
        class_number: 0,
        item_class_id: 0x1F3,
        dwg_version: 0,
        maintenance_version: 0,
        unknown1: 0,
        unknown2: 0,
    });
}

/// Registers an `ACDB_*OBJECTCONTEXTDATA_CLASS` entry in `drawing.classes`.
fn ensure_context_class(drawing: &mut CadDocument, dxf_name: &str) {
    drawing.register_object_context_class(dxf_name);
    if drawing.classes.get_by_name(dxf_name).is_some() {
        return;
    }
    let cpp = match dxf_name {
        "ACDB_ANNOTSCALEOBJECTCONTEXTDATA_CLASS" => "AcDbAnnotScaleObjectContextData",
        "ACDB_MLEADEROBJECTCONTEXTDATA_CLASS" => "AcDbMLeaderObjectContextData",
        "ACDB_MTEXTATTRIBUTEOBJECTCONTEXTDATA_CLASS" => "AcDbMTextAttributeObjectContextData",
        "ACDB_LEADEROBJECTCONTEXTDATA_CLASS" => "AcDbLeaderObjectContextData",
        _ => return,
    };
    let proxy_bits = ProxyFlags::ERASE_ALLOWED.0
        | ProxyFlags::CLONING_ALLOWED.0
        | ProxyFlags::DISABLES_PROXY_WARNING_DIALOG.0;
    ensure_dxf_class(drawing, dxf_name, cpp, proxy_bits);
}

/// Finds the root named-objects dictionary handle.
pub(super) fn root_dict_handle(drawing: &CadDocument) -> Option<Handle> {
    let h = drawing.header.named_objects_dict_handle;
    if !h.is_null() && matches!(drawing.objects.get(&h), Some(ObjectType::Dictionary(_))) {
        return Some(h);
    }
    drawing.objects.values().find_map(|o| match o {
        ObjectType::Dictionary(d) if d.owner.is_null() => Some(d.handle),
        _ => None,
    })
}

/// Gets or creates a named child dictionary under `parent_handle`.
fn ensure_sub_dictionary(drawing: &mut CadDocument, parent_handle: Handle, key: &str) -> Handle {
    if let Some(ObjectType::Dictionary(parent)) = drawing.objects.get(&parent_handle) {
        if let Some((_, existing)) = parent
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            if matches!(
                drawing.objects.get(existing),
                Some(ObjectType::Dictionary(_))
            ) {
                return *existing;
            }
        }
    }
    let child_handle = drawing.allocate_handle();
    let mut dict = Dictionary::new();
    dict.handle = child_handle;
    dict.owner = parent_handle;
    drawing
        .objects
        .insert(child_handle, ObjectType::Dictionary(dict));
    if let Some(ObjectType::Dictionary(parent)) = drawing.objects.get_mut(&parent_handle) {
        parent.add_entry(key, child_handle);
    }
    child_handle
}

/// Writes `ACAD_SCALELIST` (`SCALE` objects), header `CANNOSCALE` /
/// `CANNOSCALEVALUE`, and `AcDbVariableDictionary` (`CANNOSCALE`, optional
/// `ANNOALLVISIBLE`), returning a map of lowercase scale name -> `Scale` handle.
pub(super) fn write_scales_and_variables(
    drawing: &mut CadDocument,
    sheet: &Sheet,
) -> HashMap<String, Handle> {
    ensure_dxf_class(drawing, "SCALE", "AcDbScale", 1153);
    ensure_dxf_class(drawing, "DICTIONARYVAR", "AcDbDictionaryVar", 0);

    // Collect every scale referenced on the sheet, always ensuring "1:1" exists.
    let mut scales: Vec<AnnotationScale> = Vec::new();
    let mut push_unique = |s: AnnotationScale| {
        if !scales
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&s.name))
        {
            scales.push(s);
        }
    };
    push_unique(AnnotationScale::one_to_one());
    for s in &sheet.annotation_scales {
        push_unique(s.clone());
    }
    push_unique(sheet.active_annotation_scale());
    for obj in &sheet.objects {
        for s in sheet.object_annotation_scales(obj) {
            push_unique(s);
        }
        if let Object::Viewport(vp) = obj {
            push_unique(sheet.viewport_annotation_scale(vp));
        }
    }

    let mut scale_handles: HashMap<String, Handle> = HashMap::new();
    let Some(root) = root_dict_handle(drawing) else {
        return scale_handles;
    };

    let scalelist_handle = ensure_sub_dictionary(drawing, root, ACAD_SCALELIST);
    // Keep any pre-existing scales mapped; add missing ones with A0, A1, ...
    let mut next_idx: usize = 0;
    if let Some(ObjectType::Dictionary(dict)) = drawing.objects.get(&scalelist_handle) {
        let entries = dict.entries.clone();
        next_idx = entries.len();
        for (_, h) in entries {
            if let Some(ObjectType::Scale(sc)) = drawing.objects.get(&h) {
                scale_handles.insert(sc.name.to_lowercase(), h);
            }
        }
    }
    for s in &scales {
        let key_lower = s.name.to_lowercase();
        if scale_handles.contains_key(&key_lower) {
            continue;
        }
        let h = drawing.allocate_handle();
        let mut sc = Scale::new(&s.name, s.paper_units, s.drawing_units);
        sc.handle = h;
        sc.owner_handle = scalelist_handle;
        sc.is_unit_scale = (s.paper_units - s.drawing_units).abs() < 1e-10;
        drawing.objects.insert(h, ObjectType::Scale(sc));
        if let Some(ObjectType::Dictionary(dict)) = drawing.objects.get_mut(&scalelist_handle) {
            dict.add_entry(format!("A{next_idx}"), h);
        }
        next_idx += 1;
        scale_handles.insert(key_lower, h);
    }

    // Set CANNOSCALE & CANNOSCALEVALUE in header and AcDbVariableDictionary.
    let active = sheet.active_annotation_scale();
    drawing.header.current_annotation_scale = active.name.clone();
    drawing.header.annotation_scale_value = active.factor();

    let var_dict = ensure_sub_dictionary(drawing, root, VARIABLE_DICT);
    upsert_dict_var(drawing, var_dict, CANNOSCALE_VAR, &active.name);
    if let Some(all_visible) = sheet.anno_all_visible {
        upsert_dict_var(
            drawing,
            var_dict,
            ANNOALLVISIBLE_VAR,
            if all_visible { "1" } else { "0" },
        );
    }

    scale_handles
}

/// Updates or inserts a `DictionaryVariable` under `var_dict`.
pub(super) fn upsert_dict_var(
    drawing: &mut CadDocument,
    var_dict: Handle,
    name: &str,
    value: &str,
) -> Handle {
    ensure_dxf_class(drawing, "DICTIONARYVAR", "AcDbDictionaryVar", 0);
    let existing_handle = match drawing.objects.get(&var_dict) {
        Some(ObjectType::Dictionary(dict)) => dict
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, h)| *h),
        _ => None,
    };
    if let Some(h) = existing_handle {
        if let Some(ObjectType::DictionaryVariable(dv)) = drawing.objects.get_mut(&h) {
            dv.value = value.to_owned();
            return h;
        }
    }
    let h = drawing.allocate_handle();
    let mut dv = DictionaryVariable::new(name, value);
    dv.handle = h;
    dv.owner_handle = var_dict;
    drawing
        .objects
        .insert(h, ObjectType::DictionaryVariable(dv));
    if let Some(ObjectType::Dictionary(dict)) = drawing.objects.get_mut(&var_dict) {
        dict.add_entry(name, h);
    }
    h
}

/// Ensures `AcDbVariableDictionary` exists under root and sets a variable.
pub(super) fn set_drawing_var(
    drawing: &mut CadDocument,
    name: &str,
    value: &str,
) -> Option<Handle> {
    let root = root_dict_handle(drawing)?;
    let var_dict = ensure_sub_dictionary(drawing, root, VARIABLE_DICT);
    Some(upsert_dict_var(drawing, var_dict, name, value))
}

/// Ensures a `Scale` exists in `ACAD_SCALELIST` and returns its handle.
pub(super) fn ensure_scale_in_drawing(
    drawing: &mut CadDocument,
    scale: &AnnotationScale,
) -> Option<Handle> {
    ensure_dxf_class(drawing, "SCALE", "AcDbScale", 1153);
    let root = root_dict_handle(drawing)?;
    let scalelist_handle = ensure_sub_dictionary(drawing, root, ACAD_SCALELIST);
    if let Some(ObjectType::Dictionary(dict)) = drawing.objects.get(&scalelist_handle) {
        for (_, h) in &dict.entries {
            if let Some(ObjectType::Scale(sc)) = drawing.objects.get(h) {
                if sc.name.eq_ignore_ascii_case(&scale.name) {
                    return Some(*h);
                }
            }
        }
    }
    let next_idx = match drawing.objects.get(&scalelist_handle) {
        Some(ObjectType::Dictionary(dict)) => dict.entries.len(),
        _ => 0,
    };
    let h = drawing.allocate_handle();
    let mut sc = Scale::new(&scale.name, scale.paper_units, scale.drawing_units);
    sc.handle = h;
    sc.owner_handle = scalelist_handle;
    sc.is_unit_scale = (scale.paper_units - scale.drawing_units).abs() < 1e-10;
    drawing.objects.insert(h, ObjectType::Scale(sc));
    if let Some(ObjectType::Dictionary(dict)) = drawing.objects.get_mut(&scalelist_handle) {
        dict.add_entry(format!("A{next_idx}"), h);
    }
    Some(h)
}

/// Attaches AutoCAD's `AcadAnnotative` XDATA record (`AnnotativeData { 1 1 }`).
pub(super) fn attach_annotative_xdata(extended_data: &mut ExtendedData) {
    let mut rec = ExtendedDataRecord::new(ACAD_ANNOTATIVE);
    rec.add_value(XDataValue::String(ANNOTATIVE_DATA.to_owned()));
    rec.add_value(XDataValue::ControlString("{".to_owned()));
    rec.add_value(XDataValue::Integer16(1));
    rec.add_value(XDataValue::Integer16(1));
    rec.add_value(XDataValue::ControlString("}".to_owned()));
    extended_data.upsert_record(rec);
}

/// Returns the `APPID` handle value for `AcadAnnotative`, if registered.
fn annotative_app_handle(drawing: &CadDocument) -> Option<u64> {
    drawing
        .app_ids
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(ACAD_ANNOTATIVE))
        .map(|a| a.handle.value())
}

/// Removes `AcadAnnotative` XDATA if present (both structured records and any
/// raw DWG EED blob for `app_handle` so `DwgObjectWriter` does not re-emit
/// stale bytes verbatim).
pub(super) fn remove_annotative_xdata(extended_data: &mut ExtendedData, app_handle: Option<u64>) {
    let names: Vec<String> = extended_data
        .records()
        .iter()
        .filter(|r| r.application_name.eq_ignore_ascii_case(ACAD_ANNOTATIVE))
        .map(|r| r.application_name.clone())
        .collect();
    for name in names {
        extended_data.remove_record(&name);
    }
    if let Some(h) = app_handle {
        extended_data.raw_dwg_eed.retain(|(app, _)| *app != h);
    }
}

/// Whether `extended_data` carries `AcadAnnotative` with flag `!= 0`.
pub(super) fn has_annotative_xdata(extended_data: &ExtendedData) -> bool {
    let Some(rec) = extended_data
        .records()
        .iter()
        .find(|r| r.application_name.eq_ignore_ascii_case(ACAD_ANNOTATIVE))
    else {
        return false;
    };
    match rec.values.as_slice() {
        [XDataValue::String(tag), XDataValue::ControlString(open), XDataValue::Integer16(_ver), XDataValue::Integer16(flag), ..]
            if tag == ANNOTATIVE_DATA && open == "{" =>
        {
            *flag != 0
        }
        _ => false,
    }
}

/// Marks a `BlockRecord` annotative via `drawing.object_xdata`.
pub(super) fn mark_block_annotative(drawing: &mut CadDocument, handle: Handle) {
    let xdata = drawing.object_xdata.entry(handle).or_default();
    attach_annotative_xdata(xdata);
}

/// Returns whether the `BlockRecord` with `handle` carries `AcadAnnotative` XDATA.
pub(super) fn block_is_annotative(drawing: &CadDocument, handle: Handle) -> bool {
    drawing
        .object_xdata
        .get(&handle)
        .is_some_and(has_annotative_xdata)
}

/// Picks the default annotation scale for writing an annotative entity:
/// prefers `sheet.active_annotation_scale()` if present in `scales`, else the
/// first scale in `scales`, else `1:1`.
pub(super) fn default_scale_for(sheet: &Sheet, scales: &[AnnotationScale]) -> AnnotationScale {
    let active = sheet.active_annotation_scale();
    if let Some(found) = scales
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(&active.name))
    {
        return found.clone();
    }
    scales
        .first()
        .cloned()
        .unwrap_or_else(AnnotationScale::one_to_one)
}

/// Attaches `AcadAnnotative` XDATA, native entity annotative flags, and
/// `xdict -> AcDbContextDataManager -> ACDB_ANNOTATIONSCALES -> *A1..*An`
/// `ObjectContextData` leaves onto the entity at `entity_handle`.
///
/// When `for_dxf` is true, `MultiLeader` leaves use `ObjectContextKind::AnnotScale`
/// (`ACDB_ANNOTSCALEOBJECTCONTEXTDATA_CLASS`) rather than `ObjectContextKind::MLeader`,
/// working around an upstream `acadrust` DXF-writer bug that wraps
/// `ACDB_MLEADEROBJECTCONTEXTDATA_CLASS` in a `100/AcDbMLeaderObjectContextData`
/// marker and `300/CONTEXT_DATA{` block that AutoCAD rejects on DXF import.
pub(super) fn attach_entity_annotation_scales(
    drawing: &mut CadDocument,
    entity_handle: Handle,
    scales: &[AnnotationScale],
    default_scale: &AnnotationScale,
    scale_handles: &HashMap<String, Handle>,
    for_dxf: bool,
) {
    if scales.is_empty() {
        return;
    }
    let Some(entity_snapshot) = drawing.get_entity(entity_handle).cloned() else {
        return;
    };

    // Set XDATA + native flags and ensure extension dictionary exists.
    let existing_xdict = entity_snapshot.common().xdictionary_handle;
    let xdict_handle = match existing_xdict {
        Some(h)
            if !h.is_null()
                && matches!(drawing.objects.get(&h), Some(ObjectType::Dictionary(_))) =>
        {
            h
        }
        _ => {
            let h = drawing.allocate_handle();
            let mut dict = Dictionary::new();
            dict.handle = h;
            dict.owner = entity_handle;
            drawing.objects.insert(h, ObjectType::Dictionary(dict));
            h
        }
    };
    let app_handle = annotative_app_handle(drawing);
    if let Some(ent) = drawing.get_entity_mut(entity_handle) {
        ent.common_mut().xdictionary_handle = Some(xdict_handle);
        remove_annotative_xdata(&mut ent.common_mut().extended_data, app_handle);
        attach_annotative_xdata(&mut ent.common_mut().extended_data);
        match ent {
            EntityType::MText(m) => m.is_annotative = true,
            EntityType::MultiLeader(ml) => ml.enable_annotation_scale = true,
            _ => {}
        }
    }

    let mgr_handle = ensure_sub_dictionary(drawing, xdict_handle, CONTEXT_DATA_MANAGER);
    let scales_dict_handle = ensure_sub_dictionary(drawing, mgr_handle, ANNOTATION_SCALES_DICT);

    // Clear any prior leaves when replacing (e.g. during OBJECTSCALE revision).
    if let Some(ObjectType::Dictionary(dict)) = drawing.objects.get_mut(&scales_dict_handle) {
        let old: Vec<Handle> = dict.entries.iter().map(|(_, h)| *h).collect();
        dict.entries.clear();
        for h in old {
            drawing.objects.remove(&h);
            drawing.context_scales.remove(&h);
        }
    }

    let default_inv = default_scale.inverse_factor();
    for (idx, sc) in scales.iter().enumerate() {
        let Some(&scale_handle) = scale_handles.get(&sc.name.to_lowercase()) else {
            continue;
        };
        let is_default = sc.name.eq_ignore_ascii_case(&default_scale.name);
        let rel = sc.inverse_factor() / default_inv;
        let kind = build_context_kind(drawing, &entity_snapshot, rel, for_dxf);
        let leaf_handle = drawing.allocate_handle();
        let leaf = ObjectContextData {
            handle: leaf_handle,
            owner_handle: scales_dict_handle,
            reactors: vec![scales_dict_handle],
            xdictionary_handle: None,
            class_version: 4,
            has_file_to_extension_dictionary: true,
            uses_legacy_mleader_context_base: true,
            is_default,
            scale: scale_handle,
            kind,
        };
        let class_name = leaf.class_name().to_owned();
        ensure_context_class(drawing, &class_name);
        drawing
            .objects
            .insert(leaf_handle, ObjectType::ObjectContextData(leaf));
        drawing.context_scales.insert(leaf_handle, scale_handle);
        if let Some(ObjectType::Dictionary(dict)) = drawing.objects.get_mut(&scales_dict_handle) {
            dict.add_entry(format!("*A{}", idx + 1), leaf_handle);
        }
    }
}

/// Removes annotative XDATA, native flags, and `ACDB_ANNOTATIONSCALES` leaves
/// from an entity (used by `set_object_scales` when turning `annotative: false`).
pub(super) fn clear_entity_annotation_scales(drawing: &mut CadDocument, entity_handle: Handle) {
    let app_handle = annotative_app_handle(drawing);
    let xdict = match drawing.get_entity_mut(entity_handle) {
        Some(ent) => {
            remove_annotative_xdata(&mut ent.common_mut().extended_data, app_handle);
            match ent {
                EntityType::MText(m) => m.is_annotative = false,
                EntityType::MultiLeader(ml) => ml.enable_annotation_scale = false,
                _ => {}
            }
            ent.common().xdictionary_handle
        }
        None => return,
    };
    let Some(xdict_handle) = xdict else {
        return;
    };
    let mgr_handle = match drawing.objects.get(&xdict_handle) {
        Some(ObjectType::Dictionary(d)) => d
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(CONTEXT_DATA_MANAGER))
            .map(|(_, h)| *h),
        _ => None,
    };
    let Some(mgr_handle) = mgr_handle else {
        return;
    };
    let scales_dict_handle = match drawing.objects.get(&mgr_handle) {
        Some(ObjectType::Dictionary(d)) => d
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(ANNOTATION_SCALES_DICT))
            .map(|(_, h)| *h),
        _ => None,
    };
    let Some(scales_dict_handle) = scales_dict_handle else {
        return;
    };
    if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&scales_dict_handle) {
        let old: Vec<Handle> = d.entries.iter().map(|(_, h)| *h).collect();
        d.entries.clear();
        for h in old {
            drawing.objects.remove(&h);
            drawing.context_scales.remove(&h);
        }
    }
}

/// Builds the typed [`ObjectContextKind`] payload for `entity` at relative
/// inverse scale `rel = target_inv / default_inv`.
fn build_context_kind(
    drawing: &CadDocument,
    entity: &EntityType,
    rel: f64,
    for_dxf: bool,
) -> ObjectContextKind {
    match entity {
        EntityType::Text(t) => {
            let align = t.alignment_point.unwrap_or(t.insertion_point);
            ObjectContextKind::Text {
                horizontal_mode: t.horizontal_alignment as i16,
                rotation: t.rotation,
                insertion: Vector2::new(t.insertion_point.x, t.insertion_point.y),
                alignment: Vector2::new(align.x, align.y),
            }
        }
        EntityType::MText(m) => {
            let (sin, cos) = m.rotation.sin_cos();
            ObjectContextKind::MText(MTextContext {
                attachment: m.attachment_point as i32,
                x_axis_dir: Vector3::new(cos, sin, 0.0),
                insertion: m.insertion_point,
                rect_width: m.rectangle_width * rel,
                rect_height: m.rectangle_height.unwrap_or(0.0) * rel,
                extents_width: m.rectangle_width * rel,
                extents_height: m.height * rel,
                column_type: 0,
                columns: None,
            })
        }
        EntityType::Insert(ins) => ObjectContextKind::BlkRef {
            rotation: ins.rotation,
            insertion: ins.insert_point,
            scale_factor: Vector3::new(
                ins.x_scale() * rel,
                ins.y_scale() * rel,
                ins.z_scale() * rel,
            ),
        },
        EntityType::Hatch(h) => {
            let pattern_lines = h
                .pattern
                .lines
                .iter()
                .map(|l| {
                    let mut scaled = l.clone();
                    scaled.base_point = Vector2::new(l.base_point.x * rel, l.base_point.y * rel);
                    scaled.offset = Vector2::new(l.offset.x * rel, l.offset.y * rel);
                    scaled.dash_lengths = l.dash_lengths.iter().map(|d| d * rel).collect();
                    scaled
                })
                .collect();
            let loops = h
                .paths
                .iter()
                .map(|p| HatchLoopContext {
                    loop_type: p.flags.bits() as i32,
                    supports_context: false,
                    boundary: None,
                })
                .collect();
            ObjectContextKind::HatchScale(HatchScaleContext {
                pattern_lines,
                pattern_scale: h.pattern_scale * rel,
                pattern_base: Vector3::new(
                    h.seed_points.first().map_or(0.0, |p| p.x),
                    h.seed_points.first().map_or(0.0, |p| p.y),
                    h.elevation,
                ),
                loops,
            })
        }
        EntityType::Dimension(d) => {
            let base = d.base();
            let block_handle = drawing
                .block_records
                .iter()
                .find(|r| r.name.eq_ignore_ascii_case(&base.block_name))
                .map(|r| r.handle)
                .unwrap_or(Handle::NULL);
            let subtype = match &**d {
                acadrust::entities::Dimension::Linear(l) => DimSubtype::Aligned {
                    dimline_pt: l.base.definition_point,
                },
                acadrust::entities::Dimension::Aligned(a) => DimSubtype::Aligned {
                    dimline_pt: a.base.definition_point,
                },
                acadrust::entities::Dimension::Radius(r) => DimSubtype::Radial {
                    first_arc_pt: r.angle_vertex,
                },
                acadrust::entities::Dimension::Diameter(dm) => DimSubtype::Diametric {
                    first_arc_pt: dm.angle_vertex,
                    def_pt: dm.base.definition_point,
                },
                acadrust::entities::Dimension::Angular2Ln(a) => DimSubtype::Angular {
                    arc_pt: a.dimension_arc,
                },
                acadrust::entities::Dimension::Angular3Pt(a) => DimSubtype::Angular {
                    arc_pt: a.base.definition_point,
                },
                acadrust::entities::Dimension::Ordinate(o) => DimSubtype::Ordinate {
                    feature_location_pt: o.feature_location,
                    leader_endpt: o.leader_endpoint,
                },
                acadrust::entities::Dimension::Arc(a) => DimSubtype::Angular {
                    arc_pt: a.definition_point,
                },
                acadrust::entities::Dimension::LargeRadial(r) => DimSubtype::Radial {
                    first_arc_pt: r.jog_point,
                },
            };
            ObjectContextKind::Dim(DimContext {
                def_pt: Vector2::new(base.text_middle_point.x, base.text_middle_point.y),
                is_def_textloc: true,
                text_rotation: base.text_rotation,
                block: block_handle,
                b293: false,
                dimtofl: false,
                dimosxd: false,
                dimatfit: false,
                dimtix: false,
                dimtmove: false,
                override_code: 0,
                has_arrow2: true,
                flip_arrow2: false,
                flip_arrow1: false,
                subtype,
            })
        }
        EntityType::MultiLeader(ml) => {
            if for_dxf {
                // Work around acadrust's DXF writer wrapping
                // ACDB_MLEADEROBJECTCONTEXTDATA_CLASS in 100/AcDbMLeaderObjectContextData
                // and 300/CONTEXT_DATA{, which AutoCAD rejects on DXF import.
                return ObjectContextKind::AnnotScale;
            }
            let mut ctx = ml.context.clone();
            ctx.scale_factor *= rel;
            ctx.text_height *= rel;
            ctx.arrowhead_size *= rel;
            ctx.landing_gap *= rel;
            ctx.block_content_scale = Vector3::new(
                ctx.block_content_scale.x * rel,
                ctx.block_content_scale.y * rel,
                ctx.block_content_scale.z * rel,
            );
            for root in &mut ctx.leader_roots {
                root.landing_distance *= rel;
            }
            ObjectContextKind::MLeader(ctx)
        }
        _ => ObjectContextKind::AnnotScale,
    }
}

/// Reads `ACAD_SCALELIST`, `CANNOSCALE`, and `ANNOALLVISIBLE` from `drawing`.
pub(super) fn read_scales_and_variables(
    drawing: &CadDocument,
) -> (
    Vec<AnnotationScale>,
    Option<String>,
    Option<bool>,
    HashMap<Handle, AnnotationScale>,
) {
    let mut scales = Vec::new();
    let mut handle_map = HashMap::new();
    let mut active_scale: Option<String> = None;
    let mut anno_all_visible: Option<bool> = None;

    if let Some(root) = root_dict_handle(drawing) {
        if let Some(ObjectType::Dictionary(root_dict)) = drawing.objects.get(&root) {
            if let Some((_, sl_handle)) = root_dict
                .entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(ACAD_SCALELIST))
            {
                if let Some(ObjectType::Dictionary(sl_dict)) = drawing.objects.get(sl_handle) {
                    for (_, sc_handle) in &sl_dict.entries {
                        if let Some(ObjectType::Scale(sc)) = drawing.objects.get(sc_handle) {
                            if sc.paper_units.is_finite()
                                && sc.paper_units > 0.0
                                && sc.drawing_units.is_finite()
                                && sc.drawing_units > 0.0
                                && !sc.name.is_empty()
                            {
                                let model_scale = AnnotationScale::new(
                                    sc.name.clone(),
                                    sc.paper_units,
                                    sc.drawing_units,
                                );
                                handle_map.insert(*sc_handle, model_scale.clone());
                                if !scales.iter().any(|s: &AnnotationScale| {
                                    s.name.eq_ignore_ascii_case(&model_scale.name)
                                }) {
                                    scales.push(model_scale);
                                }
                            }
                        }
                    }
                }
            }
            if let Some((_, vd_handle)) = root_dict
                .entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(VARIABLE_DICT))
            {
                if let Some(ObjectType::Dictionary(vd_dict)) = drawing.objects.get(vd_handle) {
                    for (k, vh) in &vd_dict.entries {
                        let Some(ObjectType::DictionaryVariable(dv)) = drawing.objects.get(vh)
                        else {
                            continue;
                        };
                        if k.eq_ignore_ascii_case(CANNOSCALE_VAR) && !dv.value.trim().is_empty() {
                            active_scale = Some(dv.value.trim().to_owned());
                        } else if k.eq_ignore_ascii_case(ANNOALLVISIBLE_VAR) {
                            anno_all_visible = dv.as_bool();
                        }
                    }
                }
            }
        }
    }

    if active_scale.is_none() {
        let hdr = drawing.header.current_annotation_scale.trim();
        if !hdr.is_empty() && !hdr.eq_ignore_ascii_case(DEFAULT_ANNOTATION_SCALE) {
            active_scale = Some(hdr.to_owned());
        }
    }

    (scales, active_scale, anno_all_visible, handle_map)
}

/// Reads an entity's annotative state, supported scale names, and default
/// context scale factor (`paper / drawing`) from its XDATA, native flags, and
/// `ACDB_ANNOTATIONSCALES` extension dictionary.
pub(super) fn read_entity_annotation(
    drawing: &CadDocument,
    entity: &EntityType,
    scale_map: &HashMap<Handle, AnnotationScale>,
) -> (bool, Vec<String>, Option<f64>) {
    let mut annotative = has_annotative_xdata(&entity.common().extended_data);
    match entity {
        EntityType::MText(m) if m.is_annotative => annotative = true,
        EntityType::MultiLeader(ml) if ml.enable_annotation_scale => annotative = true,
        _ => {}
    }

    let mut scale_names: Vec<String> = Vec::new();
    let mut default_factor: Option<f64> = None;

    if let Some(xdict_handle) = entity.common().xdictionary_handle {
        if let Some(ObjectType::Dictionary(xdict)) = drawing.objects.get(&xdict_handle) {
            if let Some((_, mgr_handle)) = xdict
                .entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(CONTEXT_DATA_MANAGER))
            {
                if let Some(ObjectType::Dictionary(mgr)) = drawing.objects.get(mgr_handle) {
                    if let Some((_, sc_dict_handle)) = mgr
                        .entries
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(ANNOTATION_SCALES_DICT))
                    {
                        if let Some(ObjectType::Dictionary(sc_dict)) =
                            drawing.objects.get(sc_dict_handle)
                        {
                            for (_, leaf_handle) in &sc_dict.entries {
                                let (sc_handle, is_def) = match drawing.objects.get(leaf_handle) {
                                    Some(ObjectType::ObjectContextData(ctx)) => {
                                        (ctx.scale, ctx.is_default)
                                    }
                                    _ => match drawing.context_scales.get(leaf_handle) {
                                        Some(&sh) => (sh, false),
                                        None => continue,
                                    },
                                };
                                let resolved = scale_map.get(&sc_handle).cloned().or_else(|| {
                                    match drawing.objects.get(&sc_handle) {
                                        Some(ObjectType::Scale(sc)) => Some(AnnotationScale::new(
                                            sc.name.clone(),
                                            sc.paper_units,
                                            sc.drawing_units,
                                        )),
                                        _ => None,
                                    }
                                });
                                if let Some(sc) = resolved {
                                    annotative = true;
                                    if is_def && default_factor.is_none() {
                                        default_factor = Some(sc.factor());
                                    }
                                    if !scale_names.iter().any(|n| n.eq_ignore_ascii_case(&sc.name))
                                    {
                                        scale_names.push(sc.name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if annotative && default_factor.is_none() {
        if let Some(first_name) = scale_names.first() {
            if let Some(sc) = scale_map
                .values()
                .find(|s| s.name.eq_ignore_ascii_case(first_name))
                .cloned()
                .or_else(|| AnnotationScale::from_name(first_name))
            {
                default_factor = Some(sc.factor());
            }
        }
    }

    (annotative, scale_names, default_factor)
}
