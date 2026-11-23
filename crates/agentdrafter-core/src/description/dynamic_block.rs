//! Reads dynamic block `parameters:` in a block library and `state:` on an
//! insert in a sheet description, template, or row.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::strict_yaml::{index_path, key_path, Kind, Node};
use super::{
    angle, at, bad_value, boolean, length, plain_number, point, scalar, sequence, wrong_type,
    Fields,
};
use crate::error::{Error, ErrorCode};
use crate::model::{
    resolve_state, Block, DynamicParameter, DynamicParameterKind, DynamicValue, LookupRow, Point3,
    VisibilityState,
};
use crate::units::Unit;

/// Parameter kinds accepted under `blocks.<NAME>.parameters.<PARAM>.kind`.
pub(crate) const PARAMETER_KINDS: [&str; 5] =
    ["visibility", "flip", "linear", "rotation", "lookup"];

const VISIBILITY_KEYS: [&str; 3] = ["kind", "default", "states"];
const FLIP_KEYS: [&str; 5] = ["kind", "axis", "default", "labels", "objects"];
const LINEAR_KEYS: [&str; 11] = [
    "kind",
    "from",
    "to",
    "default",
    "min",
    "max",
    "increment",
    "allowed",
    "stretch_box",
    "stretch_objects",
    "move_objects",
];
const ROTATION_KEYS: [&str; 7] = [
    "kind",
    "base_point",
    "default",
    "min",
    "max",
    "allowed",
    "objects",
];
const LOOKUP_KEYS: [&str; 3] = ["kind", "default", "rows"];

/// Parses `blocks.<NAME>.parameters`, validating object indices against
/// `object_count` and cross-parameter references in `lookup` rows.
pub(super) fn parameters(
    node: &Node,
    path: &str,
    units: Unit,
    object_count: usize,
) -> Result<Vec<DynamicParameter>, Error> {
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(
            node,
            path,
            "a mapping from dynamic parameter names to their definitions",
        ));
    };
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let param_path = key_path(path, &entry.key);
        let name = entry.key.trim();
        if name.is_empty() {
            return Err(at(
                Error::new(
                    ErrorCode::BadValue,
                    "a dynamic parameter name cannot be empty",
                ),
                &param_path,
                entry.key_at,
            ));
        }
        let Kind::Mapping(kentries) = &entry.value.kind else {
            return Err(wrong_type(
                &entry.value,
                &param_path,
                "a mapping defining a dynamic parameter with `kind`",
            ));
        };
        let Some(kind_entry) = kentries.iter().find(|e| e.key == "kind") else {
            return Err(at(
                Error::new(
                    ErrorCode::MissingKey,
                    format!(
                        "missing key `kind`; expected one of {}",
                        PARAMETER_KINDS.join(", ")
                    ),
                ),
                &key_path(&param_path, "kind"),
                entry.value.at,
            ));
        };
        let kind_path = key_path(&param_path, "kind");
        let kind_str = scalar(
            &kind_entry.value,
            &kind_path,
            "a dynamic parameter kind (visibility, flip, linear, rotation, or lookup)",
        )?;
        let kind = match kind_str {
            "visibility" => parse_visibility(&entry.value, &param_path, object_count)?,
            "flip" => parse_flip(&entry.value, &param_path, units, object_count)?,
            "linear" => parse_linear(&entry.value, &param_path, units, object_count)?,
            "rotation" => parse_rotation(&entry.value, &param_path, units, object_count)?,
            "lookup" => parse_lookup(&entry.value, &param_path)?,
            other => {
                return Err(bad_value(
                    &kind_entry.value,
                    &kind_path,
                    format!(
                        "unknown dynamic parameter kind `{other}`; expected one of {}",
                        PARAMETER_KINDS.join(", ")
                    ),
                ))
            }
        };
        parsed.push(DynamicParameter {
            name: entry.key.clone(),
            kind,
        });
    }

    // Validate lookup rows against the other parameters of the same block
    // (converting length/angle units for referenced linear/rotation parameters).
    let non_lookup: Vec<DynamicParameter> = parsed
        .iter()
        .filter(|p| !matches!(p.kind, DynamicParameterKind::Lookup { .. }))
        .cloned()
        .collect();
    for (pi, entry) in entries.iter().enumerate() {
        let DynamicParameterKind::Lookup { rows, .. } = &mut parsed[pi].kind else {
            continue;
        };
        let rows_path = key_path(&key_path(path, &entry.key), "rows");
        for row in rows.iter_mut() {
            let row_path = key_path(&rows_path, &row.name);
            let mut normalized_values = BTreeMap::new();
            for (target_name, val) in &row.values {
                let target_path = key_path(&row_path, target_name);
                let Some(target) = non_lookup.iter().find(|p| p.name == *target_name) else {
                    let listed = non_lookup
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(at(
                        Error::new(
                            ErrorCode::UnknownName,
                            format!(
                                "lookup row `{}` sets unknown parameter `{target_name}`; \
                                 available non-lookup parameters: {listed}",
                                row.name
                            ),
                        ),
                        &target_path,
                        entry.value.at,
                    ));
                };
                let coerced = coerce_dynamic_value(target, val, units).map_err(|msg| {
                    at(
                        Error::new(ErrorCode::BadValue, msg),
                        &target_path,
                        entry.value.at,
                    )
                })?;
                let validated = target.validate(&coerced).map_err(|msg| {
                    at(
                        Error::new(ErrorCode::BadValue, msg),
                        &target_path,
                        entry.value.at,
                    )
                })?;
                normalized_values.insert(target_name.clone(), validated);
            }
            row.values = normalized_values;
        }
    }

    Ok(parsed)
}

fn parse_object_indices(node: &Node, path: &str, object_count: usize) -> Result<Vec<usize>, Error> {
    let items = sequence(node, path, "a list of object indices, counted from 0")?;
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let item_path = index_path(path, i);
        let raw = scalar(item, &item_path, "an object index, a non-negative integer")?;
        let idx: usize = raw.parse().map_err(|_| {
            bad_value(
                item,
                &item_path,
                format!("`{raw}` is not a non-negative integer object index"),
            )
        })?;
        if idx >= object_count {
            return Err(bad_value(
                item,
                &item_path,
                format!(
                    "object index {idx} is out of range; the block has {object_count} object(s) \
                     (0..{object_count})"
                ),
            ));
        }
        if !out.contains(&idx) {
            out.push(idx);
        }
    }
    Ok(out)
}

fn parse_visibility(
    node: &Node,
    path: &str,
    object_count: usize,
) -> Result<DynamicParameterKind, Error> {
    let fields = Fields::of(node, path, &VISIBILITY_KEYS, "a visibility parameter")?;
    let states_node = fields.require("states")?;
    let states_path = fields.path_of("states");
    let Kind::Mapping(entries) = &states_node.kind else {
        return Err(wrong_type(
            states_node,
            &states_path,
            "a mapping from visibility state names to lists of visible object indices",
        ));
    };
    if entries.is_empty() {
        return Err(bad_value(
            states_node,
            &states_path,
            "a visibility parameter needs at least one state",
        ));
    }
    let mut states = Vec::with_capacity(entries.len());
    for entry in entries {
        let st_path = key_path(&states_path, &entry.key);
        let objects = parse_object_indices(&entry.value, &st_path, object_count)?;
        states.push(VisibilityState {
            name: entry.key.clone(),
            objects,
        });
    }
    let default = match fields.text("default") {
        Some(def_node) => {
            let def_path = fields.path_of("default");
            let name = scalar(def_node, &def_path, "a visibility state name")?;
            if !states.iter().any(|s| s.name == name) {
                return Err(bad_value(
                    def_node,
                    &def_path,
                    format!(
                        "default visibility state `{name}` is not among `states`: {}",
                        states
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
            name.to_owned()
        }
        None => states[0].name.clone(),
    };
    Ok(DynamicParameterKind::Visibility { default, states })
}

fn parse_flip(
    node: &Node,
    path: &str,
    units: Unit,
    object_count: usize,
) -> Result<DynamicParameterKind, Error> {
    let fields = Fields::of(node, path, &FLIP_KEYS, "a flip parameter")?;
    let axis_node = fields.require("axis")?;
    let axis_path = fields.path_of("axis");
    let axis_items = sequence(
        axis_node,
        &axis_path,
        "two points [[x1, y1, z1], [x2, y2, z2]]",
    )?;
    let [a_node, b_node] = axis_items else {
        return Err(bad_value(
            axis_node,
            &axis_path,
            "a flip axis is two points: [[x1, y1, z1], [x2, y2, z2]]",
        ));
    };
    let a = point(a_node, &index_path(&axis_path, 0), units)?;
    let b = point(b_node, &index_path(&axis_path, 1), units)?;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    if dx * dx + dy * dy <= 1e-18 {
        return Err(bad_value(
            axis_node,
            &axis_path,
            "a flip axis needs two distinct points in XY",
        ));
    }
    let default = match fields.get("default") {
        Some(n) => boolean(n, &fields.path_of("default"))?,
        None => false,
    };
    let (not_flipped_label, flipped_label) = match fields.get("labels") {
        Some(n) => {
            let lp = fields.path_of("labels");
            let items = sequence(n, &lp, "two labels [not_flipped, flipped]")?;
            let [l0, l1] = items else {
                return Err(bad_value(
                    n,
                    &lp,
                    "flip `labels` takes exactly two strings: [not_flipped_label, flipped_label]",
                ));
            };
            (
                scalar(l0, &index_path(&lp, 0), "not-flipped label")?.to_owned(),
                scalar(l1, &index_path(&lp, 1), "flipped label")?.to_owned(),
            )
        }
        None => ("Not Flipped".to_owned(), "Flipped".to_owned()),
    };
    let objects = match fields.get("objects") {
        Some(n) => parse_object_indices(n, &fields.path_of("objects"), object_count)?,
        None => Vec::new(),
    };
    Ok(DynamicParameterKind::Flip {
        axis: [a, b],
        default,
        not_flipped_label,
        flipped_label,
        objects,
    })
}

fn parse_linear(
    node: &Node,
    path: &str,
    units: Unit,
    object_count: usize,
) -> Result<DynamicParameterKind, Error> {
    let fields = Fields::of(node, path, &LINEAR_KEYS, "a linear parameter")?;
    let from = point(fields.require("from")?, &fields.path_of("from"), units)?;
    let to = point(fields.require("to")?, &fields.path_of("to"), units)?;
    let dist = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2) + (to.z - from.z).powi(2)).sqrt();
    if dist <= 1e-12 {
        return Err(bad_value(
            fields.require("to")?,
            &fields.path_of("to"),
            "a linear parameter's `from` and `to` must be distinct points",
        ));
    }
    let default = match fields.get("default") {
        Some(n) => length(n, &fields.path_of("default"), units)?,
        None => dist,
    };
    let min = match fields.get("min") {
        Some(n) => Some(length(n, &fields.path_of("min"), units)?),
        None => None,
    };
    let max = match fields.get("max") {
        Some(n) => Some(length(n, &fields.path_of("max"), units)?),
        None => None,
    };
    if let (Some(lo), Some(hi)) = (min, max) {
        if lo > hi {
            return Err(bad_value(
                fields.require("max")?,
                &fields.path_of("max"),
                format!("`max` ({hi}) cannot be less than `min` ({lo})"),
            ));
        }
    }
    let increment = match fields.get("increment") {
        Some(n) => {
            let p = fields.path_of("increment");
            let v = length(n, &p, units)?;
            if v <= 0.0 {
                return Err(bad_value(n, &p, "`increment` must be more than 0"));
            }
            Some(v)
        }
        None => None,
    };
    let allowed = match fields.get("allowed") {
        Some(n) => {
            let p = fields.path_of("allowed");
            let items = sequence(n, &p, "a list of allowed distances")?;
            let mut vals = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                vals.push(length(item, &index_path(&p, i), units)?);
            }
            vals
        }
        None => Vec::new(),
    };
    let stretch_box = match fields.get("stretch_box") {
        Some(n) => {
            let p = fields.path_of("stretch_box");
            let items = sequence(n, &p, "two opposite corners [[x1, y1], [x2, y2]]")?;
            let [c0, c1] = items else {
                return Err(bad_value(
                    n,
                    &p,
                    "a stretch_box is two opposite corners [[x1, y1], [x2, y2]]",
                ));
            };
            let a = corner_2d(c0, &index_path(&p, 0), units)?;
            let b = corner_2d(c1, &index_path(&p, 1), units)?;
            Some([a, b])
        }
        None => None,
    };
    let stretch_objects = match fields.get("stretch_objects") {
        Some(n) => parse_object_indices(n, &fields.path_of("stretch_objects"), object_count)?,
        None => Vec::new(),
    };
    let move_objects = match fields.get("move_objects") {
        Some(n) => parse_object_indices(n, &fields.path_of("move_objects"), object_count)?,
        None => Vec::new(),
    };
    Ok(DynamicParameterKind::Linear {
        from,
        to,
        default,
        min,
        max,
        increment,
        allowed,
        stretch_box,
        stretch_objects,
        move_objects,
    })
}

fn corner_2d(node: &Node, path: &str, units: Unit) -> Result<[f64; 2], Error> {
    let items = sequence(node, path, "a corner [x, y] or [x, y, z]")?;
    match items {
        [x, y] | [x, y, _] => Ok([
            length(x, &index_path(path, 0), units)?,
            length(y, &index_path(path, 1), units)?,
        ]),
        _ => Err(bad_value(node, path, "expected [x, y] or [x, y, z]")),
    }
}

fn parse_rotation(
    node: &Node,
    path: &str,
    units: Unit,
    object_count: usize,
) -> Result<DynamicParameterKind, Error> {
    let fields = Fields::of(node, path, &ROTATION_KEYS, "a rotation parameter")?;
    let base_point = match fields.get("base_point") {
        Some(n) => point(n, &fields.path_of("base_point"), units)?,
        None => Point3::new(0.0, 0.0, 0.0),
    };
    let default = match fields.get("default") {
        Some(n) => angle(n, &fields.path_of("default"))?,
        None => 0.0,
    };
    let min = match fields.get("min") {
        Some(n) => Some(plain_number(
            n,
            &fields.path_of("min"),
            "minimum angle in degrees",
        )?),
        None => None,
    };
    let max = match fields.get("max") {
        Some(n) => Some(plain_number(
            n,
            &fields.path_of("max"),
            "maximum angle in degrees",
        )?),
        None => None,
    };
    if let (Some(lo), Some(hi)) = (min, max) {
        if lo > hi {
            return Err(bad_value(
                fields.require("max")?,
                &fields.path_of("max"),
                format!("`max` ({hi}) cannot be less than `min` ({lo})"),
            ));
        }
    }
    let allowed = match fields.get("allowed") {
        Some(n) => {
            let p = fields.path_of("allowed");
            let items = sequence(n, &p, "a list of allowed angles in degrees")?;
            let mut vals = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                vals.push(angle(item, &index_path(&p, i))?);
            }
            vals
        }
        None => Vec::new(),
    };
    let objects = match fields.get("objects") {
        Some(n) => parse_object_indices(n, &fields.path_of("objects"), object_count)?,
        None => Vec::new(),
    };
    Ok(DynamicParameterKind::Rotation {
        base_point,
        default,
        min,
        max,
        allowed,
        objects,
    })
}

fn parse_lookup(node: &Node, path: &str) -> Result<DynamicParameterKind, Error> {
    let fields = Fields::of(node, path, &LOOKUP_KEYS, "a lookup parameter")?;
    let rows_node = fields.require("rows")?;
    let rows_path = fields.path_of("rows");
    let Kind::Mapping(entries) = &rows_node.kind else {
        return Err(wrong_type(
            rows_node,
            &rows_path,
            "a mapping from lookup row names to parameter value mappings",
        ));
    };
    if entries.is_empty() {
        return Err(bad_value(
            rows_node,
            &rows_path,
            "a lookup parameter needs at least one row",
        ));
    }
    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let row_path = key_path(&rows_path, &entry.key);
        let Kind::Mapping(kvals) = &entry.value.kind else {
            return Err(wrong_type(
                &entry.value,
                &row_path,
                "a mapping from parameter names to values",
            ));
        };
        let mut values = BTreeMap::new();
        for kv in kvals {
            let vpath = key_path(&row_path, &kv.key);
            values.insert(kv.key.clone(), parse_raw_dynamic_value(&kv.value, &vpath)?);
        }
        rows.push(LookupRow {
            name: entry.key.clone(),
            values,
        });
    }
    let default = match fields.text("default") {
        Some(def_node) => {
            let def_path = fields.path_of("default");
            let name = scalar(def_node, &def_path, "a lookup row name")?;
            if !rows.iter().any(|r| r.name == name) {
                return Err(bad_value(
                    def_node,
                    &def_path,
                    format!(
                        "default lookup row `{name}` is not among `rows`: {}",
                        rows.iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
            name.to_owned()
        }
        None => rows[0].name.clone(),
    };
    Ok(DynamicParameterKind::Lookup { default, rows })
}

fn parse_raw_dynamic_value(node: &Node, path: &str) -> Result<DynamicValue, Error> {
    let raw = scalar(node, path, "a parameter value (text, number, or boolean)")?;
    if raw == "true" {
        return Ok(DynamicValue::Bool(true));
    }
    if raw == "false" {
        return Ok(DynamicValue::Bool(false));
    }
    if let Ok(n) = raw.parse::<f64>() {
        if n.is_finite() {
            return Ok(DynamicValue::Number(n));
        }
    }
    Ok(DynamicValue::Text(raw.to_owned()))
}

fn coerce_dynamic_value(
    param: &DynamicParameter,
    val: &DynamicValue,
    units: Unit,
) -> Result<DynamicValue, String> {
    match (&param.kind, val) {
        (DynamicParameterKind::Linear { .. }, DynamicValue::Text(s)) => {
            crate::units::parse_length(s)
                .map(|(n, unit)| {
                    let converted = match unit {
                        Some(from) => from.convert(n, units),
                        None => n,
                    };
                    DynamicValue::Number(converted)
                })
                .map_err(|_| format!("invalid distance `{s}` for parameter `{}`", param.name))
        }
        (DynamicParameterKind::Rotation { .. }, DynamicValue::Text(s)) => {
            let trimmed = s
                .trim()
                .trim_end_matches("deg")
                .trim_end_matches('°')
                .trim();
            trimmed
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map(|n| DynamicValue::Number(crate::description::normalize_degrees(n)))
                .ok_or_else(|| format!("invalid angle `{s}` for parameter `{}`", param.name))
        }
        (
            DynamicParameterKind::Visibility { .. } | DynamicParameterKind::Lookup { .. },
            DynamicValue::Number(_) | DynamicValue::Bool(_),
        ) => Ok(DynamicValue::Text(val.display_value())),
        _ => Ok(val.clone()),
    }
}

/// Parses an insert's `state:` mapping and validates it against `block`.
pub(super) fn insert_state(
    fields: &Fields,
    block: &Block,
    units: Unit,
) -> Result<BTreeMap<String, DynamicValue>, Error> {
    let Some(node) = fields.get("state") else {
        return Ok(BTreeMap::new());
    };
    let path = fields.path_of("state");
    let Kind::Mapping(entries) = &node.kind else {
        return Err(wrong_type(
            node,
            &path,
            "a mapping from dynamic parameter names to values",
        ));
    };
    if entries.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut state = BTreeMap::new();
    for entry in entries {
        if entry.value.is_bare_null() {
            continue;
        }
        let entry_path = key_path(&path, &entry.key);
        let raw_val = parse_raw_dynamic_value(&entry.value, &entry_path)?;
        let val = match block.parameters.iter().find(|p| p.name == entry.key) {
            Some(param) => coerce_dynamic_value(param, &raw_val, units)
                .map_err(|msg| bad_value(&entry.value, &entry_path, msg))?,
            None => raw_val,
        };
        state.insert(entry.key.clone(), val);
    }
    if let Err(problem) = resolve_state(block, &state) {
        let entry_path = key_path(&path, &problem.parameter);
        let code = if problem.unknown {
            ErrorCode::UnknownName
        } else {
            ErrorCode::BadValue
        };
        let pos = entries
            .iter()
            .find(|e| e.key == problem.parameter)
            .map_or(node.at, |e| e.value.at);
        return Err(at(Error::new(code, problem.message), &entry_path, pos));
    }
    // Normalize explicit values (e.g. flip labels -> bool, rotation normalized).
    let mut normalized = BTreeMap::new();
    for (k, v) in state {
        if let Some(param) = block.parameters.iter().find(|p| p.name == k) {
            let norm = param.validate(&v).unwrap_or(v);
            normalized.insert(k, norm);
        }
    }
    Ok(normalized)
}

/// Adds JSON Schema definitions for dynamic block parameters and insert state.
pub(super) fn add_schema_defs(defs: &mut Map<String, Value>) {
    defs.insert(
        "dynamic_value".into(),
        json!({
            "description": "A dynamic block parameter value: a visibility state or lookup row \
                            name, a distance (number or length with unit), an angle in degrees, \
                            or a flip boolean/label.",
            "anyOf": [
                { "type": "string" },
                { "type": "number" },
                { "type": "boolean" },
            ],
        }),
    );
    defs.insert(
        "dynamic_state".into(),
        json!({
            "description": "Chosen values for a dynamic block's parameters, mapped from parameter \
                            name to value. Omitted or null parameters take their defaults.",
            "type": "object",
            "additionalProperties": {
                "anyOf": [
                    { "$ref": "#/$defs/dynamic_value" },
                    { "type": "null" },
                ],
            },
        }),
    );
    defs.insert(
        "dynamic_parameter".into(),
        json!({
            "description": "A dynamic parameter on a block: visibility, flip, linear (stretch), \
                            rotation, or lookup.",
            "oneOf": [
                {
                    "type": "object",
                    "required": ["kind", "states"],
                    "additionalProperties": false,
                    "properties": {
                        "kind": { "const": "visibility" },
                        "default": { "type": ["string", "number", "boolean"] },
                        "states": {
                            "type": "object",
                            "minProperties": 1,
                            "additionalProperties": {
                                "type": "array",
                                "items": { "type": "integer", "minimum": 0 },
                            },
                        },
                    },
                },
                {
                    "type": "object",
                    "required": ["kind", "axis"],
                    "additionalProperties": false,
                    "properties": {
                        "kind": { "const": "flip" },
                        "axis": {
                            "type": "array",
                            "items": { "$ref": "#/$defs/point" },
                            "minItems": 2,
                            "maxItems": 2,
                        },
                        "default": { "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }] },
                        "labels": {
                            "type": "array",
                            "items": { "type": "string" },
                            "minItems": 2,
                            "maxItems": 2,
                        },
                        "objects": {
                            "type": "array",
                            "items": { "type": "integer", "minimum": 0 },
                        },
                    },
                },
                {
                    "type": "object",
                    "required": ["kind", "from", "to"],
                    "additionalProperties": false,
                    "properties": {
                        "kind": { "const": "linear" },
                        "from": { "$ref": "#/$defs/point" },
                        "to": { "$ref": "#/$defs/point" },
                        "default": { "$ref": "#/$defs/length" },
                        "min": { "$ref": "#/$defs/length" },
                        "max": { "$ref": "#/$defs/length" },
                        "increment": { "$ref": "#/$defs/length_more_than_0" },
                        "allowed": {
                            "type": "array",
                            "items": { "$ref": "#/$defs/length" },
                        },
                        "stretch_box": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "items": { "$ref": "#/$defs/length" },
                                "minItems": 2,
                                "maxItems": 3,
                            },
                            "minItems": 2,
                            "maxItems": 2,
                        },
                        "stretch_objects": {
                            "type": "array",
                            "items": { "type": "integer", "minimum": 0 },
                        },
                        "move_objects": {
                            "type": "array",
                            "items": { "type": "integer", "minimum": 0 },
                        },
                    },
                },
                {
                    "type": "object",
                    "required": ["kind"],
                    "additionalProperties": false,
                    "properties": {
                        "kind": { "const": "rotation" },
                        "base_point": { "$ref": "#/$defs/point" },
                        "default": { "$ref": "#/$defs/angle" },
                        "min": { "type": "number" },
                        "max": { "type": "number" },
                        "allowed": {
                            "type": "array",
                            "items": { "$ref": "#/$defs/angle" },
                        },
                        "objects": {
                            "type": "array",
                            "items": { "type": "integer", "minimum": 0 },
                        },
                    },
                },
                {
                    "type": "object",
                    "required": ["kind", "rows"],
                    "additionalProperties": false,
                    "properties": {
                        "kind": { "const": "lookup" },
                        "default": { "type": ["string", "number", "boolean"] },
                        "rows": {
                            "type": "object",
                            "minProperties": 1,
                            "additionalProperties": {
                                "type": "object",
                                "additionalProperties": { "$ref": "#/$defs/dynamic_value" },
                            },
                        },
                    },
                },
            ],
        }),
    );
}
