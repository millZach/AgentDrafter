//! AutoCAD `GROUP` / `ACAD_GROUP` dictionary read, write, and revision
//! helpers behind the DWG seam (`crates/agentdrafter-core/src/dwg.rs`).

use std::collections::HashMap;

use acadrust::objects::ObjectType;
use acadrust::types::Handle;
use acadrust::CadDocument;

use crate::error::{Error, ErrorCode};
use crate::model::Group;

/// Writes every [`Group`] in `groups` into `drawing`'s `ACAD_GROUP` dictionary
/// and attaches the group reactor handle to each member entity.
pub(super) fn write_groups(
    drawing: &mut CadDocument,
    groups: &[Group],
    index_to_handle: &[Option<Handle>],
) -> Result<(), Error> {
    for group in groups {
        let handles: Vec<Handle> = group
            .objects
            .iter()
            .filter_map(|&idx| index_to_handle.get(idx).copied().flatten())
            .collect();
        if handles.is_empty() {
            continue;
        }
        let group_handle = if group.unnamed {
            drawing.create_unnamed_group(&handles)
        } else {
            drawing.create_group(&group.name, &handles)
        }
        .map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not write group `{}`: {e}", group.name),
            )
        })?;
        if let Some(ObjectType::Group(dwg_group)) = drawing.objects.get_mut(&group_handle) {
            dwg_group.description = group.description.clone();
            dwg_group.selectable = group.selectable;
        }
    }
    Ok(())
}

/// Reads all groups from `drawing`'s `ACAD_GROUP` dictionary, mapping member
/// entity handles onto indices in `handles` (the uppercase hex handle list of
/// `DrawingContents::objects`).
pub(super) fn read_groups(drawing: &CadDocument, handles: &[String]) -> Vec<Group> {
    let mut handle_to_index: HashMap<u64, usize> = HashMap::with_capacity(handles.len());
    for (idx, hex) in handles.iter().enumerate() {
        if let Ok(val) = u64::from_str_radix(hex, 16) {
            handle_to_index.insert(val, idx);
        }
    }
    let dict_handle = acad_group_dict_handle(drawing);
    let mut out = Vec::new();
    let Some(dict_handle) = dict_handle else {
        return out;
    };
    let Some(ObjectType::Dictionary(dict)) = drawing.objects.get(&dict_handle) else {
        return out;
    };
    for (key, group_handle) in &dict.entries {
        let Some(ObjectType::Group(g)) = drawing.objects.get(group_handle) else {
            continue;
        };
        let mut objects: Vec<usize> = Vec::with_capacity(g.entities.len());
        for h in &g.entities {
            if let Some(&idx) = handle_to_index.get(&h.value()) {
                if !objects.contains(&idx) {
                    objects.push(idx);
                }
            }
        }
        let name = if !key.is_empty() {
            key.clone()
        } else {
            g.name.clone()
        };
        out.push(Group {
            name,
            description: g.description.clone(),
            selectable: g.selectable,
            unnamed: g.unnamed,
            objects,
        });
    }
    out
}

/// Locates the `ACAD_GROUP` dictionary handle from `drawing.header` or the
/// root named-object dictionary.
pub(super) fn acad_group_dict_handle(drawing: &CadDocument) -> Option<Handle> {
    let h = drawing.header.acad_group_dict_handle;
    if !h.is_null() && matches!(drawing.objects.get(&h), Some(ObjectType::Dictionary(_))) {
        return Some(h);
    }
    let root = drawing.header.named_objects_dict_handle;
    let dict = match drawing.objects.get(&root) {
        Some(ObjectType::Dictionary(d)) => Some(d),
        _ => drawing.objects.values().find_map(|o| match o {
            ObjectType::Dictionary(d) if d.owner.is_null() => Some(d),
            _ => None,
        }),
    }?;
    dict.entries
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("ACAD_GROUP"))
        .map(|(_, h)| *h)
}

/// Looks up a group's `(canonical_name, group_handle)` in `ACAD_GROUP`
/// case-insensitively.
pub(super) fn find_group(drawing: &CadDocument, name: &str) -> Option<(String, Handle)> {
    let dict_handle = acad_group_dict_handle(drawing)?;
    let ObjectType::Dictionary(dict) = drawing.objects.get(&dict_handle)? else {
        return None;
    };
    dict.entries
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .and_then(|(k, h)| {
            matches!(drawing.objects.get(h), Some(ObjectType::Group(_))).then(|| (k.clone(), *h))
        })
}

/// Returns all canonical group names in `drawing`'s `ACAD_GROUP` dictionary.
pub(super) fn group_names(drawing: &CadDocument) -> Vec<String> {
    let Some(dict_handle) = acad_group_dict_handle(drawing) else {
        return Vec::new();
    };
    let Some(ObjectType::Dictionary(dict)) = drawing.objects.get(&dict_handle) else {
        return Vec::new();
    };
    dict.entries
        .iter()
        .filter(|(_, h)| matches!(drawing.objects.get(h), Some(ObjectType::Group(_))))
        .map(|(k, _)| k.clone())
        .collect()
}
