//! Wipeout (`WIPEOUT` / `WIPEOUTVARIABLES`) and draw order (`SORTENTSTABLE` /
//! `ACAD_SORTENTS`) behind the DWG seam (#78).

use std::collections::HashMap;

use acadrust::entities::{
    EntityType, Wipeout as DwgWipeout, WipeoutClipMode, WipeoutClipType, WipeoutDisplayFlags,
};
use acadrust::objects::{ObjectType, SortEntitiesTable, WipeoutVariables};
use acadrust::types::{Handle, Vector2, Vector3};
use acadrust::CadDocument;

use super::point;
use crate::model::{Point3, Properties, Wipeout, WipeoutFrame};

/// Named Object Dictionary key for AutoCAD's wipeout frame setting.
pub(super) const ACAD_WIPEOUT_VARS: &str = "ACAD_WIPEOUT_VARS";

/// Extension dictionary key on a block record for AutoCAD's draw order table.
pub(super) const ACAD_SORTENTS: &str = "ACAD_SORTENTS";

/// Builds an `acadrust::entities::Wipeout` matching AutoCAD 2026's polygonal
/// `WIPEOUT` representation (including the repeated closing boundary vertex).
pub(super) fn dwg_wipeout(wipeout: &Wipeout) -> EntityType {
    let mut min_x = wipeout.points[0].x;
    let mut min_y = wipeout.points[0].y;
    let mut max_x = wipeout.points[0].x;
    let mut max_y = wipeout.points[0].y;
    for p in wipeout.points.iter().skip(1) {
        min_x = min_x.min(p.x);
        min_y = min_y.min(p.y);
        max_x = max_x.max(p.x);
        max_y = max_y.max(p.y);
    }
    let width = (max_x - min_x).max(1e-12);
    let height = (max_y - min_y).max(1e-12);
    let z = wipeout.points[0].z;

    let mut normalized: Vec<Vector2> = wipeout
        .points
        .iter()
        .map(|p| Vector2::new((p.x - min_x) / width - 0.5, 0.5 - (p.y - min_y) / height))
        .collect();
    if let Some(&first) = normalized.first() {
        normalized.push(first);
    }

    let mut entity = DwgWipeout::new();
    entity.insertion_point = Vector3::new(min_x, min_y, z);
    entity.u_vector = Vector3::new(width, 0.0, 0.0);
    entity.v_vector = Vector3::new(0.0, height, 0.0);
    entity.size = Vector2::new(1.0, 1.0);
    entity.flags = WipeoutDisplayFlags::DEFAULT;
    entity.clipping_enabled = true;
    entity.brightness = 50;
    entity.contrast = 50;
    entity.fade = 0;
    entity.clip_mode = WipeoutClipMode::Outside;
    entity.clip_type = WipeoutClipType::Polygonal;
    entity.clip_boundary_vertices = normalized;
    EntityType::Wipeout(entity)
}

/// Reads an `acadrust::entities::Wipeout` into our `Wipeout` model.
pub(super) fn read_wipeout(w: &DwgWipeout, layer: String) -> Option<Wipeout> {
    let raw: Vec<Vector3> = match w.clip_type {
        WipeoutClipType::Rectangular => w.corners().to_vec(),
        WipeoutClipType::Polygonal => w.world_boundary_vertices(),
    };
    let mut pts: Vec<Point3> = raw.into_iter().map(point).collect();
    if pts.len() >= 4 {
        let first = pts[0];
        let last = *pts.last().unwrap();
        if (first.x - last.x).abs() <= 1e-9
            && (first.y - last.y).abs() <= 1e-9
            && (first.z - last.z).abs() <= 1e-9
        {
            pts.pop();
        }
    }
    if pts.len() < 3 {
        return None;
    }
    let z = pts[0].z;
    if !pts
        .iter()
        .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite() && (p.z - z).abs() <= 1e-6)
    {
        return None;
    }
    if crate::model::revision_cloud::signed_area(&pts).abs() <= 1e-12 {
        return None;
    }
    Some(Wipeout {
        points: pts,
        layer,
        properties: Properties::default(),
    })
}

/// Finds the root Named Object Dictionary handle in `drawing`.
pub(super) fn root_dictionary_handle(drawing: &CadDocument) -> Option<Handle> {
    drawing.objects.iter().find_map(|(h, obj)| match obj {
        ObjectType::Dictionary(d) if d.owner.is_null() && !h.is_null() => Some(*h),
        _ => None,
    })
}

/// Reads `ACAD_WIPEOUT_VARS` (`WIPEOUTVARIABLES.display_frame`) from `drawing`.
/// Returns `WipeoutFrame::On` when the object is absent (matching AutoCAD's default).
pub(super) fn read_wipeout_frame(drawing: &CadDocument) -> WipeoutFrame {
    if let Some(root) = root_dictionary_handle(drawing) {
        if let Some(ObjectType::Dictionary(d)) = drawing.objects.get(&root) {
            if let Some(vars_handle) = d
                .entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(ACAD_WIPEOUT_VARS))
                .map(|(_, h)| *h)
            {
                if let Some(ObjectType::WipeoutVariables(wv)) = drawing.objects.get(&vars_handle) {
                    return WipeoutFrame::from_dwg_code(wv.display_frame);
                }
            }
        }
    }
    for obj in drawing.objects.values() {
        if let ObjectType::WipeoutVariables(wv) = obj {
            return WipeoutFrame::from_dwg_code(wv.display_frame);
        }
    }
    WipeoutFrame::On
}

/// Writes or updates `ACAD_WIPEOUT_VARS` (`WIPEOUTVARIABLES`) in `drawing`'s
/// root dictionary. Returns `(root_dict_handle, vars_handle, created_new)` so
/// revision verification can whitelist the exact mutated non-graphical handles.
pub(super) fn write_wipeout_frame(
    drawing: &mut CadDocument,
    frame: WipeoutFrame,
) -> Option<(Handle, Handle, bool)> {
    let root = root_dictionary_handle(drawing)?;
    let existing = match drawing.objects.get(&root) {
        Some(ObjectType::Dictionary(d)) => d
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(ACAD_WIPEOUT_VARS))
            .map(|(_, h)| *h),
        _ => None,
    };
    if let Some(handle) = existing {
        if let Some(ObjectType::WipeoutVariables(wv)) = drawing.objects.get_mut(&handle) {
            wv.display_frame = frame.dwg_code();
            return Some((root, handle, false));
        }
    }
    let handle = drawing.allocate_handle();
    let wv = WipeoutVariables {
        handle,
        owner: root,
        display_frame: frame.dwg_code(),
    };
    drawing
        .objects
        .insert(handle, ObjectType::WipeoutVariables(wv));
    if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&root) {
        d.add_entry(ACAD_WIPEOUT_VARS, handle);
    }
    Some((root, handle, true))
}

/// Looks up the `ACAD_SORTENTS` (`SortEntitiesTable`) for `block_handle`, if any.
pub(super) fn sort_table_for_block(
    drawing: &CadDocument,
    block_handle: Handle,
) -> Option<&SortEntitiesTable> {
    if let Some(xdic_handle) = drawing.extension_dictionary_handle(block_handle) {
        if let Some(ObjectType::Dictionary(d)) = drawing.objects.get(&xdic_handle) {
            if let Some(sort_handle) = d
                .entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(ACAD_SORTENTS))
                .map(|(_, h)| *h)
            {
                if let Some(ObjectType::SortEntitiesTable(t)) = drawing.objects.get(&sort_handle) {
                    return Some(t);
                }
            }
        }
    }
    drawing.objects.values().find_map(|obj| match obj {
        ObjectType::SortEntitiesTable(t) if t.block_owner_handle == block_handle => Some(t),
        _ => None,
    })
}

/// The draw-order sort key for `entity_handle` inside `block_handle`: its
/// `ACAD_SORTENTS` sort handle when listed, or its own handle otherwise.
pub(super) fn sort_key(table: Option<&SortEntitiesTable>, entity_handle: Handle) -> u64 {
    table
        .and_then(|t| t.get_sort_handle(entity_handle))
        .unwrap_or(entity_handle)
        .value()
}

/// Returns a block's objects sorted in AutoCAD's `ACAD_SORTENTS` draw order.
pub(super) fn sorted_block_objects<'a>(
    drawing: &'a CadDocument,
    block_handle: Handle,
    objects: Vec<&'a EntityType>,
) -> Vec<&'a EntityType> {
    let Some(table) = sort_table_for_block(drawing, block_handle) else {
        return objects;
    };
    if table.is_empty() {
        return objects;
    }
    let mut keyed: Vec<(u64, usize, &'a EntityType)> = objects
        .into_iter()
        .enumerate()
        .map(|(idx, e)| (sort_key(Some(table), e.common().handle), idx, e))
        .collect();
    keyed.sort_by_key(|&(k, idx, _)| (k, idx));
    keyed.into_iter().map(|(_, _, e)| e).collect()
}

/// Creates or updates `ACAD_SORTENTS` on `block_handle` so the listed space
/// handles draw in the exact order given by `ordered_handles`.
///
/// Returns `(mutated_existing_handles, added_new_handles)` for revision
/// verification.
pub(super) fn apply_sort_order(
    drawing: &mut CadDocument,
    block_handle: Handle,
    ordered_handles: &[Handle],
) -> (Vec<u64>, Vec<u64>) {
    let mut changed = Vec::new();
    let mut added = Vec::new();

    let xdic_existed = drawing.extension_dictionary_handle(block_handle).is_some();
    let xdic_handle = drawing.ensure_extension_dictionary(block_handle);
    if xdic_existed {
        changed.push(xdic_handle.value());
    } else {
        added.push(xdic_handle.value());
    }

    let existing_sort_handle = match drawing.objects.get(&xdic_handle) {
        Some(ObjectType::Dictionary(d)) => d
            .entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(ACAD_SORTENTS))
            .map(|(_, h)| *h),
        _ => None,
    };

    let sort_handle = match existing_sort_handle {
        Some(h)
            if matches!(
                drawing.objects.get(&h),
                Some(ObjectType::SortEntitiesTable(_))
            ) =>
        {
            changed.push(h.value());
            h
        }
        _ => {
            let h = drawing.allocate_handle();
            let mut table = SortEntitiesTable::for_block(block_handle);
            table.handle = h;
            table.owner_handle = xdic_handle;
            drawing
                .objects
                .insert(h, ObjectType::SortEntitiesTable(table));
            if let Some(ObjectType::Dictionary(d)) = drawing.objects.get_mut(&xdic_handle) {
                d.remove_entry(ACAD_SORTENTS);
                d.add_entry(ACAD_SORTENTS, h);
            }
            added.push(h.value());
            h
        }
    };

    // Assign strictly increasing sort keys by permuting the space's own sorted
    // handles (matching AutoCAD 2026's own DRAWORDER convention).
    let mut sorted_keys: Vec<Handle> = ordered_handles.to_vec();
    sorted_keys.sort_by_key(|h| h.value());
    let mut dedup_seen: HashMap<u64, ()> = HashMap::new();
    let unique_ordered: Vec<Handle> = ordered_handles
        .iter()
        .copied()
        .filter(|h| dedup_seen.insert(h.value(), ()).is_none())
        .collect();
    sorted_keys.truncate(unique_ordered.len());

    if let Some(ObjectType::SortEntitiesTable(table)) = drawing.objects.get_mut(&sort_handle) {
        table.clear();
        for (entity_h, sort_h) in unique_ordered.into_iter().zip(sorted_keys) {
            table.add_entry(entity_h, sort_h);
        }
    }

    (changed, added)
}
