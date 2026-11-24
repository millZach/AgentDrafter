//! AutoCAD `FIELD` / `FIELDLIST` (`ACAD_FIELD` / `ACAD_FIELDLIST`) reading and
//! writing behind the DWG seam (#87).

use std::collections::{BTreeMap, HashMap};

use acadrust::fields::NewField;
use acadrust::objects::{Field as DwgField, ObjectType};
use acadrust::types::Handle;
use acadrust::CadDocument;

use super::text::{mtext_code, mtext_contents, text_code, text_contents};
use super::BUILD_MARK;
use crate::error::{Error, ErrorCode};
use crate::model::field::{
    parse_autocad_code, FieldItem, TemplatePart, TextField, UNEVALUATED_FIELD,
};

/// Reads custom drawing properties (`DWGPROPS`) from `drawing.summary_info`,
/// skipping AgentDrafter's own build mark.
pub(super) fn read_custom_properties(drawing: &CadDocument) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in &drawing.summary_info.custom_properties {
        if !k.eq_ignore_ascii_case(BUILD_MARK) && !k.trim().is_empty() {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// Writes custom drawing properties (`DWGPROPS`) into `drawing.summary_info`.
pub(super) fn write_custom_properties(drawing: &mut CadDocument, props: &BTreeMap<String, String>) {
    for (k, v) in props {
        if !k.eq_ignore_ascii_case(BUILD_MARK) {
            drawing
                .summary_info
                .custom_properties
                .push((k.clone(), v.clone()));
        }
    }
}

/// Attaches `tf` to `host` (`Text`, `MText`, or `AttributeEntity` inside an
/// `Insert`) using AutoCAD's `ACAD_FIELD` / `FIELD` / `FIELDLIST` structure,
/// updating the host's displayed text and checksum to match `display_slots`.
pub(super) fn attach_text_field(
    drawing: &mut CadDocument,
    host: Handle,
    tf: &TextField,
    display_slots: &[String],
    id_to_handle: &HashMap<String, Handle>,
    width_factor: Option<f64>,
    is_mtext: bool,
) -> Result<Handle, Error> {
    let mut template = String::new();
    if is_mtext {
        if let Some(wf) = width_factor {
            template.push_str(&format!("\\W{wf};"));
        }
    }
    for part in tf.template_parts() {
        match part {
            TemplatePart::Literal(lit) => {
                if is_mtext {
                    template.push_str(&mtext_code(lit, None));
                } else {
                    template.push_str(&text_code(lit));
                }
            }
            TemplatePart::Slot(idx) => {
                template.push_str(&format!("%<\\_FldIdx {idx}>%"));
            }
        }
    }

    let mut children = Vec::with_capacity(tf.items.len());
    for (i, item) in tf.items.iter().enumerate() {
        let code = item.autocad_code();
        let raw_display = display_slots
            .get(i)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| UNEVALUATED_FIELD.to_owned());
        // `acadrust::CadDocument::set_text_field` escapes child display strings
        // for MText itself, while copying them directly for Text / Attrib.
        let display = if is_mtext {
            raw_display
        } else {
            text_code(&raw_display)
        };
        let mut nf = NewField::new(code, display);
        for obj_ref in item.referenced_objects() {
            let handle = resolve_ref_handle(obj_ref, id_to_handle).ok_or_else(|| {
                Error::new(
                    ErrorCode::UnknownName,
                    format!("field references unknown object `{obj_ref}`"),
                )
            })?;
            nf.objects.push(handle);
        }
        children.push(nf);
    }

    drawing
        .set_text_field(host, &template, children)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::DwgWrite,
                format!(
                    "could not attach field to object handle `{:X}`",
                    host.value()
                ),
            )
        })
}

/// Removes any `ACAD_FIELD` structure attached to `host`.
pub(super) fn clear_host_field(drawing: &mut CadDocument, host: Handle) -> bool {
    drawing.remove_text_field(host)
}

fn resolve_ref_handle(obj_ref: &str, id_to_handle: &HashMap<String, Handle>) -> Option<Handle> {
    if let Some(hex) = obj_ref.strip_prefix('#') {
        if let Ok(val) = u64::from_str_radix(hex.trim(), 16) {
            if val != 0 {
                return Some(Handle::new(val));
            }
        }
    }
    if let Some(&h) = id_to_handle.get(&obj_ref.to_lowercase()) {
        return Some(h);
    }
    let local = obj_ref
        .rsplit_once('/')
        .map_or(obj_ref, |(_, tail)| tail)
        .to_lowercase();
    id_to_handle.get(&local).copied()
}

/// Reads an attached `ACAD_FIELD` (`TEXT` container + child `FIELD`s) from an
/// object's extension dictionary, if present.
pub(super) fn read_host_field(
    drawing: &CadDocument,
    xdictionary_handle: Option<Handle>,
    handle_to_id: &HashMap<u64, String>,
    is_mtext: bool,
) -> Option<TextField> {
    let xdict_handle = xdictionary_handle.filter(|h| !h.is_null())?;
    let ObjectType::Dictionary(xdict) = drawing.objects.get(&xdict_handle)? else {
        return None;
    };
    let field_dict_handle = xdict
        .entries
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("ACAD_FIELD"))
        .map(|(_, h)| *h)?;
    let ObjectType::Dictionary(field_dict) = drawing.objects.get(&field_dict_handle)? else {
        return None;
    };
    let container_handle = field_dict
        .entries
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("TEXT"))
        .or_else(|| field_dict.entries.first())
        .map(|(_, h)| *h)?;

    let container = match drawing.objects.get(&container_handle) {
        Some(ObjectType::Field(f)) => f,
        _ => return None,
    };

    if container.evaluator_id == "_text" {
        read_container_field(drawing, container, handle_to_id, is_mtext)
    } else {
        let item = read_leaf_field(container, handle_to_id);
        Some(TextField::single(item))
    }
}

fn read_container_field(
    drawing: &CadDocument,
    container: &DwgField,
    handle_to_id: &HashMap<u64, String>,
    is_mtext: bool,
) -> Option<TextField> {
    // Collect child fields in order.
    let mut children: Vec<&DwgField> = container
        .child_fields
        .iter()
        .filter_map(|h| match drawing.objects.get(h) {
            Some(ObjectType::Field(f)) => Some(f),
            _ => None,
        })
        .collect();
    if children.is_empty() {
        // Fallback: scan objects owned by container, sorted by handle.
        children = drawing
            .objects
            .values()
            .filter_map(|o| match o {
                ObjectType::Field(f) if f.owner == container.handle => Some(f),
                _ => None,
            })
            .collect();
        children.sort_by_key(|f| f.handle.value());
    }
    if children.is_empty() {
        return None;
    }

    let items: Vec<FieldItem> = children
        .iter()
        .map(|child| read_leaf_field(child, handle_to_id))
        .collect();

    // Decode container.code replacing `%<\_FldIdx N>%` with `{N}` and
    // stripping any leading MText `\W...;` width-factor prefix.
    let raw_code = container.code.as_str();
    let stripped = if is_mtext {
        strip_mtext_width_prefix(raw_code)
    } else {
        raw_code
    };
    let mut template = String::new();
    let mut rest = stripped;
    const MARKER: &str = "%<\\_FldIdx ";
    while let Some(pos) = rest.find(MARKER) {
        let lit = &rest[..pos];
        template.push_str(&decode_literal(lit, is_mtext));
        let after = &rest[pos + MARKER.len()..];
        let Some(end) = after.find(">%") else {
            template.push_str(&decode_literal(&rest[pos..], is_mtext));
            rest = "";
            break;
        };
        let idx_str = after[..end].trim();
        if let Ok(idx) = idx_str.parse::<usize>() {
            if idx < items.len() {
                template.push_str(&format!("{{{idx}}}"));
            } else {
                template.push_str(&rest[pos..pos + MARKER.len() + end + 2]);
            }
        } else {
            template.push_str(&rest[pos..pos + MARKER.len() + end + 2]);
        }
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        template.push_str(&decode_literal(rest, is_mtext));
    }
    if template.is_empty() {
        template = "{0}".to_owned();
    }
    Some(TextField { template, items })
}

fn read_leaf_field(field: &DwgField, handle_to_id: &HashMap<u64, String>) -> FieldItem {
    let ref_ids: Vec<String> = field
        .referenced_objects
        .iter()
        .map(|h| {
            handle_to_id
                .get(&h.value())
                .cloned()
                .unwrap_or_else(|| format!("#{:X}", h.value()))
        })
        .collect();
    let raw_display = field.value.display();
    let cached = (!raw_display.is_empty()).then(|| raw_display.to_owned());
    parse_autocad_code(&field.code, &ref_ids, cached)
}

fn strip_mtext_width_prefix(s: &str) -> &str {
    if let Some(after) = s.strip_prefix("\\W") {
        if let Some((num, tail)) = after.split_once(';') {
            if num.parse::<f64>().is_ok() {
                return tail;
            }
        }
    }
    s
}

fn decode_literal(lit: &str, is_mtext: bool) -> String {
    if lit.is_empty() {
        return String::new();
    }
    if is_mtext {
        mtext_contents(lit)
            .map(|(decoded, _)| decoded)
            .unwrap_or_else(|| lit.to_owned())
    } else {
        text_contents(lit).unwrap_or_else(|| lit.to_owned())
    }
}
