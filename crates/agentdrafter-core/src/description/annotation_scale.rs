//! Reads annotation scales (`annotation_scales`, `annotation_scale`,
//! `anno_all_visible`), per-object `annotative` / `annotation_scales`, and
//! per-viewport `annotation_scale`, and states them in the JSON Schema.

use serde_json::{json, Map, Value};

use super::schema::{optional_text, POSITIVE_NUMBER, TEXT_TYPES};
use super::strict_yaml::{index_path, key_path, Kind, Node};
use super::{at, bad_value, boolean, plain_number, scalar, sequence, wrong_type, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{AnnotationScale, DEFAULT_ANNOTATION_SCALE};

/// Top-level sheet key declaring named annotation scales (`ACAD_SCALELIST`).
pub(crate) const ANNOTATION_SCALES_KEY: &str = "annotation_scales";

/// Top-level sheet key and viewport key naming the active annotation scale
/// (`CANNOSCALE`).
pub(crate) const ANNOTATION_SCALE_KEY: &str = "annotation_scale";

/// Top-level sheet key controlling `ANNOALLVISIBLE`.
pub(crate) const ANNO_ALL_VISIBLE_KEY: &str = "anno_all_visible";

/// Key on styles, blocks, and annotative objects marking them annotative.
pub(crate) const ANNOTATIVE_KEY: &str = "annotative";

/// Reads top-level `annotation_scales:` mapping from scale name to
/// `[paper_units, drawing_units]`.
pub(super) fn annotation_scales(node: &Node) -> Result<Vec<AnnotationScale>, Error> {
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(
            node,
            ANNOTATION_SCALES_KEY,
            "a mapping from annotation scale name to [paper_units, drawing_units], such as `'1:50': [1, 50]`",
        ));
    };
    let mut out: Vec<AnnotationScale> = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.key.trim();
        let path = key_path(ANNOTATION_SCALES_KEY, &entry.key);
        if name.is_empty() || name != entry.key || name.chars().any(char::is_control) {
            return Err(Error::new(
                ErrorCode::BadValue,
                "an annotation scale name cannot be empty, start or end with a space, or hold control characters",
            )
            .at_field(path)
            .at_position(entry.key_at.line, entry.key_at.column));
        }
        if let Some(prev) = out.iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            return Err(Error::new(
                ErrorCode::DuplicateName,
                format!(
                    "annotation scale `{name}` is already defined as `{}`; AutoCAD ignores case in scale names",
                    prev.name
                ),
            )
            .at_field(path)
            .at_position(entry.key_at.line, entry.key_at.column));
        }
        let pair = sequence(
            &entry.value,
            &path,
            "two positive numbers [paper_units, drawing_units], such as [1, 50]",
        )?;
        let [p_node, d_node] = pair else {
            return Err(at(
                Error::new(
                    ErrorCode::WrongType,
                    format!(
                        "an annotation scale takes [paper_units, drawing_units] (2 numbers); found {}",
                        pair.len()
                    ),
                ),
                &path,
                entry.value.at,
            ));
        };
        let p_path = index_path(&path, 0);
        let d_path = index_path(&path, 1);
        let paper = plain_number(p_node, &p_path, "paper units, a number more than 0")?;
        if !paper.is_finite() || paper <= 0.0 {
            return Err(bad_value(
                p_node,
                &p_path,
                "paper units must be a finite number more than 0",
            ));
        }
        let drawing = plain_number(d_node, &d_path, "drawing units, a number more than 0")?;
        if !drawing.is_finite() || drawing <= 0.0 {
            return Err(bad_value(
                d_node,
                &d_path,
                "drawing units must be a finite number more than 0",
            ));
        }
        out.push(AnnotationScale::new(name, paper, drawing));
    }
    Ok(out)
}

/// Resolves a scale name referenced on a sheet (by `annotation_scale:`, an
/// object's `annotation_scales:`, or a viewport's `annotation_scale:`).
/// Accepts any scale defined in `declared` (case-insensitively, returning its
/// canonical name) or any valid ratio name such as `"1:50"` or
/// `"1/4\" = 1'-0\""`.
pub(super) fn resolve_scale_name(
    node: &Node,
    path: &str,
    declared: &[AnnotationScale],
) -> Result<String, Error> {
    let raw = scalar(
        node,
        path,
        "an annotation scale name such as `'1:50'` or `'1:1'`",
    )?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(bad_value(
            node,
            path,
            "an annotation scale name cannot be empty",
        ));
    }
    if let Some(found) = declared
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(trimmed))
    {
        return Ok(found.name.clone());
    }
    if let Some(parsed) = AnnotationScale::from_name(trimmed) {
        return Ok(parsed.name);
    }
    let known: Vec<&str> = declared.iter().map(|s| s.name.as_str()).collect();
    let hint = if known.is_empty() {
        "write a ratio such as `'1:50'` or `'1:1'`, or define it under top-level `annotation_scales:`".to_owned()
    } else {
        format!(
            "write a ratio such as `'1:50'` or one of the declared `annotation_scales`: {}",
            known.join(", ")
        )
    };
    Err(bad_value(
        node,
        path,
        format!("unknown annotation scale `{raw}`; {hint}"),
    ))
}

/// Reads optional `annotative: bool` on a style or block.
pub(super) fn optional_annotative(fields: &Fields) -> Result<bool, Error> {
    match fields.get(ANNOTATIVE_KEY) {
        Some(node) => boolean(node, &fields.path_of(ANNOTATIVE_KEY)),
        None => Ok(false),
    }
}

/// Reads optional `annotative` and `annotation_scales` on an annotative object
/// (`text`, `mtext`, `dimension`, `mleader`, `hatch`, `insert`).
///
/// If `annotation_scales` is provided and `annotative` is omitted, `annotative`
/// defaults to `true`. Specifying `annotative: false` with non-empty
/// `annotation_scales` is an error.
pub(super) fn object_annotation(
    fields: &Fields,
    declared: &[AnnotationScale],
) -> Result<(bool, Vec<String>), Error> {
    let explicit_flag = match fields.get(ANNOTATIVE_KEY) {
        Some(node) => Some(boolean(node, &fields.path_of(ANNOTATIVE_KEY))?),
        None => None,
    };
    let mut scales = Vec::new();
    if let Some(node) = fields.get(ANNOTATION_SCALES_KEY) {
        let path = fields.path_of(ANNOTATION_SCALES_KEY);
        let items = sequence(
            node,
            &path,
            "a list of annotation scale names, such as `['1:50', '1:100']`",
        )?;
        for (i, item) in items.iter().enumerate() {
            let item_path = index_path(&path, i);
            let name = resolve_scale_name(item, &item_path, declared)?;
            if scales
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&name))
            {
                return Err(bad_value(
                    item,
                    &item_path,
                    format!("annotation scale `{name}` is listed more than once"),
                ));
            }
            scales.push(name);
        }
        if explicit_flag == Some(false) && !scales.is_empty() {
            return Err(bad_value(
                node,
                &path,
                "`annotation_scales` cannot be set when `annotative` is `false`",
            ));
        }
    }
    let annotative = match explicit_flag {
        Some(flag) => flag,
        None => !scales.is_empty(),
    };
    Ok((annotative, scales))
}

/// Ensures every scale referenced by `sheet.annotation_scale`, viewports, or
/// objects is present in `scales` (in order, without case-insensitive
/// duplicates).
pub(super) fn collect_referenced_scales(
    scales: &mut Vec<AnnotationScale>,
    sheet_scale: Option<&str>,
    objects: &[crate::model::Object],
) {
    let mut ensure = |name: &str| {
        if !scales.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
            if let Some(parsed) = AnnotationScale::from_name(name) {
                scales.push(parsed);
            }
        }
    };
    if let Some(name) = sheet_scale {
        ensure(name);
    }
    for obj in objects {
        for name in obj.annotation_scales() {
            ensure(name);
        }
        if let crate::model::Object::Viewport(vp) = obj {
            if let Some(name) = &vp.annotation_scale {
                ensure(name);
            }
        }
    }
}

/// JSON Schema for top-level `annotation_scales`.
pub(super) fn annotation_scales_property() -> Value {
    json!({
        "description": "Named annotation scales the sheet defines, matching AutoCAD's \
                        ACAD_SCALELIST: a mapping from scale name (such as `'1:50'` or \
                        `'1/4\" = 1'-0\"'`) to `[paper_units, drawing_units]` (two numbers \
                        more than 0). Standard ratio names such as `'1:50'` or `'1:100'` may \
                        also be used directly without listing them here.",
        "type": "object",
        "additionalProperties": {
            "type": "array",
            "minItems": 2,
            "maxItems": 2,
            "items": {
                "anyOf": [
                    { "type": "number", "exclusiveMinimum": 0 },
                    { "type": "string", "pattern": format!("^{POSITIVE_NUMBER}$") },
                ],
            },
        },
    })
}

/// JSON Schema for top-level `annotation_scale` (`CANNOSCALE`).
pub(super) fn annotation_scale_property() -> Value {
    optional_text(json!({
        "description": format!(
            "The sheet's current annotation scale (AutoCAD's CANNOSCALE), such as `'1:50'`. \
             Annotative objects drawn in model space scale by its inverse, and annotative \
             objects that omit `annotation_scales` support it by default. Without it, \
             `{DEFAULT_ANNOTATION_SCALE}`."
        ),
        "type": TEXT_TYPES,
        "minLength": 1,
    }))
}

/// JSON Schema for top-level `anno_all_visible` (`ANNOALLVISIBLE`).
pub(super) fn anno_all_visible_property() -> Value {
    json!({
        "description": "Whether annotative objects draw even when they do not support the \
                        active viewport or model-space annotation scale (AutoCAD's \
                        ANNOALLVISIBLE): true or false. Without it, model space shows all \
                        annotative objects and paper-space viewports show only those that \
                        support the viewport's scale.",
        "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
    })
}

/// JSON Schema for the `annotative` boolean key on styles, blocks, and objects.
pub(super) fn annotative_property(description: &str) -> Value {
    json!({
        "description": description,
        "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
    })
}

/// JSON Schema for the `annotation_scales` list key on annotative objects.
pub(super) fn object_annotation_scales_property() -> Value {
    json!({
        "description": "The annotation scales this annotative object supports (AutoCAD's \
                        OBJECTSCALE), such as `['1:50', '1:100']`. Through a viewport at one \
                        of these scales the object draws at its declared paper size; through \
                        a viewport at another scale it is hidden unless `anno_all_visible` is \
                        true. Giving this list marks the object annotative even if \
                        `annotative` is omitted.",
        "type": "array",
        "items": { "type": TEXT_TYPES, "minLength": 1 },
    })
}

/// JSON Schema for a viewport's `annotation_scale` key.
pub(super) fn viewport_annotation_scale_property() -> Value {
    optional_text(json!({
        "description": "The viewport's annotation scale (such as `'1:50'`), which determines \
                        which annotative model-space objects show in this viewport and at \
                        what size. Without it, the viewport uses its `scale`.",
        "type": TEXT_TYPES,
        "minLength": 1,
    }))
}

/// Adds shared annotation-scale definitions to `defs` if needed.
pub(super) fn add_defs(defs: &mut Map<String, Value>) {
    defs.insert(
        "annotative".into(),
        annotative_property(
            "Whether this object is annotative (AutoCAD's ANNOTATIVE): true or false. \
             An annotative object declares its paper size and draws scaled by the inverse \
             of the active annotation scale so it lands on paper at that exact size. \
             Without it, false (unless `annotation_scales` is given or its style/block is \
             annotative).",
        ),
    );
    defs.insert(
        "object_annotation_scales".into(),
        object_annotation_scales_property(),
    );
}
