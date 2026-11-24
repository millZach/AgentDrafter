//! Dynamic block parameters, state resolution, and geometry evaluation.
//!
//! A dynamic block in a block library declares named parameters — visibility
//! states, lookup tables, flips, linear/stretch distances, and rotations —
//! alongside its geometry. Each insert may name a chosen [`DynamicValue`] for
//! any parameter under `state`, and [`evaluate_block`] turns the block and
//! state into an evaluated [`Block`] whose geometry draws through the one
//! drawing pipeline (ADR-0011) into PNG, PDF, WebGL2 viewer, extents, snaps
//! and checks, and writes into DWG as an evaluated anonymous block (`*U<n>`).

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::block::{readable, Block};
use super::{
    Arc, Circle, Donut, Ellipse, Hatch, HatchEdge, HatchLoop, Line, Object, Point3, Polyline,
    RevisionCloud, RevisionCloudBoundary, Solid, Spline, Wipeout,
};
use crate::description::{normalize_degrees, sin_cos_degrees};
use crate::text::{Attachment, Justify, MText, Text};

/// How close two floating-point parameter values may be and still match an
/// allowed discrete value or default.
pub const PARAM_TOLERANCE: f64 = 1e-9;

fn default_not_flipped() -> String {
    "Not Flipped".to_owned()
}

fn default_flipped() -> String {
    "Flipped".to_owned()
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The value of one dynamic block parameter on an insert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DynamicValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl DynamicValue {
    /// Human-readable representation for messages and diagnostics.
    pub fn display_value(&self) -> String {
        match self {
            DynamicValue::Bool(b) => b.to_string(),
            DynamicValue::Number(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    format!("{}", *n as i64)
                } else {
                    format!("{n}")
                }
            }
            DynamicValue::Text(s) => s.clone(),
        }
    }

    /// Encodes a dynamic value into a short typed string safe for DWG XDATA
    /// (preserving the distinction between booleans, numbers, and text).
    pub(crate) fn encode_xdata(&self) -> String {
        match self {
            DynamicValue::Bool(true) => "b:true".to_owned(),
            DynamicValue::Bool(false) => "b:false".to_owned(),
            DynamicValue::Number(n) => format!("n:{n}"),
            DynamicValue::Text(s) => format!("s:{s}"),
        }
    }

    /// Decodes a string written by [`DynamicValue::encode_xdata`].
    pub(crate) fn decode_xdata(raw: &str) -> Option<Self> {
        if let Some(rest) = raw.strip_prefix("b:") {
            match rest {
                "true" => Some(DynamicValue::Bool(true)),
                "false" => Some(DynamicValue::Bool(false)),
                _ => None,
            }
        } else if let Some(rest) = raw.strip_prefix("n:") {
            let n: f64 = rest.parse().ok()?;
            n.is_finite().then_some(DynamicValue::Number(n))
        } else {
            raw.strip_prefix("s:")
                .map(|rest| DynamicValue::Text(rest.to_owned()))
        }
    }

    /// Whether two dynamic values are equal within numeric tolerance.
    pub(crate) fn approx_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (DynamicValue::Bool(a), DynamicValue::Bool(b)) => a == b,
            (DynamicValue::Number(a), DynamicValue::Number(b)) => {
                (a - b).abs() <= PARAM_TOLERANCE * a.abs().max(b.abs()).max(1.0)
            }
            (DynamicValue::Text(a), DynamicValue::Text(b)) => a == b,
            _ => false,
        }
    }
}

/// One named state of a visibility parameter: the indices in `block.objects`
/// that draw when this state is active.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityState {
    pub name: String,
    pub objects: Vec<usize>,
}

/// One named row of a lookup parameter: choosing `name` applies every
/// `(parameter_name, value)` pair in `values`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LookupRow {
    pub name: String,
    pub values: BTreeMap<String, DynamicValue>,
}

/// One dynamic parameter declared on a [`Block`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DynamicParameter {
    /// The parameter's name, as AutoCAD's Properties palette lists it.
    pub name: String,
    #[serde(flatten)]
    pub kind: DynamicParameterKind,
}

/// What a dynamic parameter controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DynamicParameterKind {
    /// Shows only the listed objects of the block for the chosen state.
    Visibility {
        default: String,
        states: Vec<VisibilityState>,
    },
    /// Mirrors targeted objects (or all objects when `objects` is empty) across
    /// the line through `axis[0]` and `axis[1]`.
    Flip {
        axis: [Point3; 2],
        #[serde(default, skip_serializing_if = "is_false")]
        default: bool,
        #[serde(default = "default_not_flipped")]
        not_flipped_label: String,
        #[serde(default = "default_flipped")]
        flipped_label: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        objects: Vec<usize>,
    },
    /// Stretches vertices inside `stretch_box` and/or translates whole
    /// `move_objects` along the axis from `from` to `to` by `(value - default)`.
    Linear {
        from: Point3,
        to: Point3,
        default: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        increment: Option<f64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed: Vec<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stretch_box: Option<[[f64; 2]; 2]>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stretch_objects: Vec<usize>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        move_objects: Vec<usize>,
    },
    /// Rotates targeted objects (or all objects when `objects` is empty) about
    /// `base_point` by `(value - default)` degrees anticlockwise.
    Rotation {
        base_point: Point3,
        #[serde(default)]
        default: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed: Vec<f64>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        objects: Vec<usize>,
    },
    /// Named lookup table: choosing a row sets multiple underlying parameters.
    Lookup {
        default: String,
        rows: Vec<LookupRow>,
    },
    /// Dynamic parameter read from an existing DWG whose actions live in the
    /// DWG's own `ACAD_ENHANCEDBLOCK` evaluation graph.
    External {
        parameter_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        default: Option<DynamicValue>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed: Vec<DynamicValue>,
        #[serde(default, skip_serializing_if = "is_false")]
        read_only: bool,
    },
}

/// Problem found while validating an insert's dynamic state against its block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicStateProblem {
    pub parameter: String,
    pub unknown: bool,
    pub message: String,
}

impl DynamicParameter {
    /// The parameter's default value, if it declares one.
    pub fn default_value(&self) -> Option<DynamicValue> {
        match &self.kind {
            DynamicParameterKind::Visibility { default, .. } => {
                Some(DynamicValue::Text(default.clone()))
            }
            DynamicParameterKind::Flip { default, .. } => Some(DynamicValue::Bool(*default)),
            DynamicParameterKind::Linear { default, .. } => Some(DynamicValue::Number(*default)),
            DynamicParameterKind::Rotation { default, .. } => Some(DynamicValue::Number(*default)),
            DynamicParameterKind::Lookup { default, .. } => {
                Some(DynamicValue::Text(default.clone()))
            }
            DynamicParameterKind::External { default, .. } => default.clone(),
        }
    }

    /// Validates and normalizes a caller-supplied value for this parameter.
    pub fn validate(&self, value: &DynamicValue) -> Result<DynamicValue, String> {
        match &self.kind {
            DynamicParameterKind::Visibility { states, .. } => {
                let DynamicValue::Text(name) = value else {
                    return Err(format!(
                        "visibility parameter `{}` takes a state name ({}), not `{}`",
                        self.name,
                        states
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        value.display_value()
                    ));
                };
                if states.iter().any(|s| s.name == *name) {
                    Ok(DynamicValue::Text(name.clone()))
                } else {
                    Err(format!(
                        "unknown visibility state `{name}` for parameter `{}`; allowed states: {}",
                        self.name,
                        states
                            .iter()
                            .map(|s| s.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            DynamicParameterKind::Flip {
                not_flipped_label,
                flipped_label,
                ..
            } => parse_flip_value(value, not_flipped_label, flipped_label)
                .map(DynamicValue::Bool)
                .ok_or_else(|| {
                    format!(
                        "flip parameter `{}` takes `true`/`false`, `0`/`1`, `{not_flipped_label}`, \
                         or `{flipped_label}`, not `{}`",
                        self.name,
                        value.display_value()
                    )
                }),
            DynamicParameterKind::Linear {
                default,
                min,
                max,
                increment,
                allowed,
                ..
            } => {
                let DynamicValue::Number(v) = value else {
                    return Err(format!(
                        "linear parameter `{}` takes a distance (number), not `{}`",
                        self.name,
                        value.display_value()
                    ));
                };
                if !v.is_finite() {
                    return Err(format!(
                        "linear parameter `{}` must be a finite number",
                        self.name
                    ));
                }
                if let Some(lo) = min {
                    if *v < lo - PARAM_TOLERANCE {
                        return Err(format!(
                            "linear parameter `{}` value `{v}` is less than its minimum `{lo}`",
                            self.name
                        ));
                    }
                }
                if let Some(hi) = max {
                    if *v > hi + PARAM_TOLERANCE {
                        return Err(format!(
                            "linear parameter `{}` value `{v}` is more than its maximum `{hi}`",
                            self.name
                        ));
                    }
                }
                if !allowed.is_empty()
                    && !allowed
                        .iter()
                        .any(|a| (*v - a).abs() <= PARAM_TOLERANCE * a.abs().max(1.0))
                {
                    let listed = allowed
                        .iter()
                        .map(|a| DynamicValue::Number(*a).display_value())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(format!(
                        "linear parameter `{}` value `{v}` is not in its allowed list: {listed}",
                        self.name
                    ));
                }
                if let Some(step) = increment {
                    if *step > 0.0 {
                        let base = min.unwrap_or(*default);
                        let steps = (*v - base) / step;
                        if (steps - steps.round()).abs() > 1e-6 {
                            return Err(format!(
                                "linear parameter `{}` value `{v}` does not align with increment \
                                 `{step}` from `{base}`",
                                self.name
                            ));
                        }
                    }
                }
                Ok(DynamicValue::Number(*v))
            }
            DynamicParameterKind::Rotation {
                min, max, allowed, ..
            } => {
                let DynamicValue::Number(v) = value else {
                    return Err(format!(
                        "rotation parameter `{}` takes an angle in degrees (number), not `{}`",
                        self.name,
                        value.display_value()
                    ));
                };
                if !v.is_finite() {
                    return Err(format!(
                        "rotation parameter `{}` must be a finite number",
                        self.name
                    ));
                }
                let angle = normalize_degrees(*v);
                if let Some(lo) = min {
                    if *v < lo - PARAM_TOLERANCE {
                        return Err(format!(
                            "rotation parameter `{}` angle `{v}` is less than its minimum `{lo}`",
                            self.name
                        ));
                    }
                }
                if let Some(hi) = max {
                    if *v > hi + PARAM_TOLERANCE {
                        return Err(format!(
                            "rotation parameter `{}` angle `{v}` is more than its maximum `{hi}`",
                            self.name
                        ));
                    }
                }
                if !allowed.is_empty()
                    && !allowed.iter().any(|a| {
                        let diff = (angle - normalize_degrees(*a)).abs();
                        diff <= 1e-6 || (360.0 - diff).abs() <= 1e-6
                    })
                {
                    let listed = allowed
                        .iter()
                        .map(|a| DynamicValue::Number(*a).display_value())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(format!(
                        "rotation parameter `{}` angle `{v}` is not in its allowed list: {listed}",
                        self.name
                    ));
                }
                Ok(DynamicValue::Number(angle))
            }
            DynamicParameterKind::Lookup { rows, .. } => {
                let DynamicValue::Text(name) = value else {
                    return Err(format!(
                        "lookup parameter `{}` takes a row name ({}), not `{}`",
                        self.name,
                        rows.iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        value.display_value()
                    ));
                };
                if rows.iter().any(|r| r.name == *name) {
                    Ok(DynamicValue::Text(name.clone()))
                } else {
                    Err(format!(
                        "unknown lookup row `{name}` for parameter `{}`; allowed rows: {}",
                        self.name,
                        rows.iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            DynamicParameterKind::External {
                allowed, read_only, ..
            } => {
                if *read_only {
                    return Err(format!("parameter `{}` is read-only", self.name));
                }
                if !allowed.is_empty() && !allowed.iter().any(|a| a.approx_eq(value)) {
                    let listed = allowed
                        .iter()
                        .map(DynamicValue::display_value)
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(format!(
                        "value `{}` is not allowed for parameter `{}`; allowed values: {listed}",
                        value.display_value(),
                        self.name
                    ));
                }
                Ok(value.clone())
            }
        }
    }
}

/// Parses a flip parameter value: `true`/`false`, `1`/`0`, or the flip labels.
pub(crate) fn parse_flip_value(
    value: &DynamicValue,
    not_flipped_label: &str,
    flipped_label: &str,
) -> Option<bool> {
    match value {
        DynamicValue::Bool(b) => Some(*b),
        DynamicValue::Number(n) if (*n - 0.0).abs() <= PARAM_TOLERANCE => Some(false),
        DynamicValue::Number(n) if (*n - 1.0).abs() <= PARAM_TOLERANCE => Some(true),
        DynamicValue::Text(s) => {
            if s.eq_ignore_ascii_case("true")
                || s == "1"
                || s.eq_ignore_ascii_case(flipped_label)
                || s.eq_ignore_ascii_case("flipped")
            {
                Some(true)
            } else if s.eq_ignore_ascii_case("false")
                || s == "0"
                || s.eq_ignore_ascii_case(not_flipped_label)
                || s.eq_ignore_ascii_case("not flipped")
            {
                Some(false)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Validates `state` against `block.parameters` and returns the normalized
/// map of all effective parameter values (with defaults filled in and lookup
/// rows expanded).
pub fn resolve_state(
    block: &Block,
    state: &BTreeMap<String, DynamicValue>,
) -> Result<BTreeMap<String, DynamicValue>, DynamicStateProblem> {
    if state.is_empty() && block.parameters.is_empty() {
        return Ok(BTreeMap::new());
    }
    if block.parameters.is_empty() {
        let first = state.keys().next().expect("non-empty state");
        return Err(DynamicStateProblem {
            parameter: first.clone(),
            unknown: true,
            message: format!(
                "block `{}` is not a dynamic block (it declares no `parameters`), so it takes no \
                 `state`",
                block.name
            ),
        });
    }

    // Validate every caller-provided key and value.
    let mut explicit: BTreeMap<String, DynamicValue> = BTreeMap::new();
    for (name, raw_val) in state {
        let Some(param) = block.parameters.iter().find(|p| p.name == *name) else {
            let listed = block
                .parameters
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(DynamicStateProblem {
                parameter: name.clone(),
                unknown: true,
                message: format!(
                    "block `{}` has no dynamic parameter `{name}`; its parameters: {listed}",
                    block.name
                ),
            });
        };
        let normalized = param
            .validate(raw_val)
            .map_err(|message| DynamicStateProblem {
                parameter: name.clone(),
                unknown: false,
                message,
            })?;
        explicit.insert(name.clone(), normalized);
    }

    // Start from each parameter's default.
    let mut resolved: BTreeMap<String, DynamicValue> = BTreeMap::new();
    for param in &block.parameters {
        if let Some(def) = param.default_value() {
            resolved.insert(param.name.clone(), def);
        }
    }

    // Expand Lookup parameters (default first, then any explicit lookup).
    for param in &block.parameters {
        let DynamicParameterKind::Lookup { default, rows } = &param.kind else {
            continue;
        };
        let chosen = match explicit.get(&param.name) {
            Some(DynamicValue::Text(name)) => name.as_str(),
            _ => default.as_str(),
        };
        resolved.insert(param.name.clone(), DynamicValue::Text(chosen.to_owned()));
        if let Some(row) = rows.iter().find(|r| r.name == chosen) {
            for (target_name, target_val) in &row.values {
                if let Some(target_param) = block.parameters.iter().find(|p| p.name == *target_name)
                {
                    if let Ok(norm) = target_param.validate(target_val) {
                        resolved.insert(target_name.clone(), norm);
                    }
                }
            }
        }
    }

    // Apply explicit non-lookup overrides last so they override lookup defaults.
    for param in &block.parameters {
        if matches!(param.kind, DynamicParameterKind::Lookup { .. }) {
            continue;
        }
        if let Some(val) = explicit.get(&param.name) {
            resolved.insert(param.name.clone(), val.clone());
        }
    }

    Ok(resolved)
}

/// Whether `state` resolves to the exact same parameter values as the block's
/// default state.
pub fn is_default_state(block: &Block, state: &BTreeMap<String, DynamicValue>) -> bool {
    if block.parameters.is_empty() || state.is_empty() {
        return true;
    }
    let Ok(def) = resolve_state(block, &BTreeMap::new()) else {
        return true;
    };
    let Ok(cur) = resolve_state(block, state) else {
        return true;
    };
    if def.len() != cur.len() {
        return false;
    }
    def.iter()
        .all(|(k, v)| cur.get(k).is_some_and(|cv| v.approx_eq(cv)))
}

/// Whether `block` has local dynamic actions that AgentDrafter evaluates.
pub fn has_evaluable_parameters(block: &Block) -> bool {
    block
        .parameters
        .iter()
        .any(|p| !matches!(p.kind, DynamicParameterKind::External { .. }))
}

/// Evaluates `block` at `state`, returning an evaluated [`Block`] whose
/// objects reflect active visibility, linear/stretch, rotation, and flip
/// parameters. Ordinary static blocks (or DWG-read blocks with only
/// `External` parameters) are borrowed with zero allocation.
pub fn evaluate_block<'a>(
    block: &'a Block,
    state: &BTreeMap<String, DynamicValue>,
) -> Cow<'a, Block> {
    if !has_evaluable_parameters(block) {
        return Cow::Borrowed(block);
    }
    let resolved = resolve_state(block, state)
        .or_else(|_| resolve_state(block, &BTreeMap::new()))
        .unwrap_or_default();

    let n = block.objects.len();
    let mut objects = block.objects.clone();
    let mut cells = block.cells.clone();
    let mut visible = vec![true; n];

    // 1. Visibility parameters.
    for param in &block.parameters {
        let DynamicParameterKind::Visibility { default, states } = &param.kind else {
            continue;
        };
        let active_name = match resolved.get(&param.name) {
            Some(DynamicValue::Text(s)) => s.as_str(),
            _ => default.as_str(),
        };
        if let Some(vs) = states.iter().find(|s| s.name == active_name) {
            for (i, vis) in visible.iter_mut().enumerate() {
                if !vs.objects.contains(&i) {
                    *vis = false;
                }
            }
        }
    }

    // 2. Linear / Stretch parameters.
    for param in &block.parameters {
        let DynamicParameterKind::Linear {
            from,
            to,
            default,
            stretch_box,
            stretch_objects,
            move_objects,
            ..
        } = &param.kind
        else {
            continue;
        };
        let value = match resolved.get(&param.name) {
            Some(DynamicValue::Number(v)) => *v,
            _ => *default,
        };
        let delta = value - default;
        if delta.abs() <= 1e-12 {
            continue;
        }
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let dz = to.z - from.z;
        let len = (dx * dx + dy * dy + dz * dz).sqrt();
        if len <= 1e-12 {
            continue;
        }
        let dv = Point3::new(dx / len * delta, dy / len * delta, dz / len * delta);

        // Whole-object translations (`move_objects`).
        for &idx in move_objects {
            if let Some(obj) = objects.get_mut(idx) {
                translate_object(obj, dv);
                for cell in &mut cells {
                    if cell.object == idx {
                        cell.min[0] += dv.x;
                        cell.min[1] += dv.y;
                        cell.max[0] += dv.x;
                        cell.max[1] += dv.y;
                    }
                }
            }
        }

        // Window stretch (`stretch_box` + `stretch_objects`).
        if let Some(raw_box) = stretch_box {
            let bmin = [
                raw_box[0][0].min(raw_box[1][0]),
                raw_box[0][1].min(raw_box[1][1]),
            ];
            let bmax = [
                raw_box[0][0].max(raw_box[1][0]),
                raw_box[0][1].max(raw_box[1][1]),
            ];
            let stretch_all = stretch_objects.is_empty() && move_objects.is_empty();
            for (idx, obj) in objects.iter_mut().enumerate() {
                if !stretch_all && !stretch_objects.contains(&idx) {
                    continue;
                }
                if move_objects.contains(&idx) {
                    continue;
                }
                let moved = stretch_object(obj, bmin, bmax, dv);
                if moved {
                    for cell in &mut cells {
                        if cell.object == idx {
                            let cx = 0.5 * (cell.min[0] + cell.max[0]);
                            let cy = 0.5 * (cell.min[1] + cell.max[1]);
                            if in_box(Point3::new(cx, cy, 0.0), bmin, bmax) {
                                cell.min[0] += dv.x;
                                cell.min[1] += dv.y;
                                cell.max[0] += dv.x;
                                cell.max[1] += dv.y;
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Rotation parameters.
    for param in &block.parameters {
        let DynamicParameterKind::Rotation {
            base_point,
            default,
            objects: targets,
            ..
        } = &param.kind
        else {
            continue;
        };
        let value = match resolved.get(&param.name) {
            Some(DynamicValue::Number(v)) => *v,
            _ => *default,
        };
        let delta_deg = normalize_degrees(value - default);
        if delta_deg <= 1e-12 || (360.0 - delta_deg) <= 1e-12 {
            continue;
        }
        let all = targets.is_empty();
        for (idx, obj) in objects.iter_mut().enumerate() {
            if !all && !targets.contains(&idx) {
                continue;
            }
            rotate_object(obj, *base_point, delta_deg);
            for cell in &mut cells {
                if cell.object == idx {
                    rotate_cell(cell, *base_point, delta_deg);
                }
            }
        }
    }

    // 4. Flip parameters.
    for param in &block.parameters {
        let DynamicParameterKind::Flip {
            axis,
            default,
            objects: targets,
            ..
        } = &param.kind
        else {
            continue;
        };
        let flipped = match resolved.get(&param.name) {
            Some(DynamicValue::Bool(b)) => *b,
            _ => *default,
        };
        if !flipped {
            continue;
        }
        let all = targets.is_empty();
        for (idx, obj) in objects.iter_mut().enumerate() {
            if !all && !targets.contains(&idx) {
                continue;
            }
            mirror_object(obj, axis[0], axis[1]);
            for cell in &mut cells {
                if cell.object == idx {
                    mirror_cell(cell, axis[0], axis[1]);
                }
            }
        }
    }

    // 5. Apply visibility filtering: keep carried AttributeDefinitions (marking
    // hidden ones `invisible = true` so `insert.attributes` order never shifts)
    // and drop hidden geometry objects, remapping cell indices.
    let mut final_objects = Vec::with_capacity(n);
    let mut index_map: Vec<Option<usize>> = vec![None; n];
    for (idx, mut obj) in objects.into_iter().enumerate() {
        if visible[idx] {
            index_map[idx] = Some(final_objects.len());
            final_objects.push(obj);
        } else if let Object::AttributeDefinition(ref mut def) = obj {
            def.invisible = true;
            index_map[idx] = Some(final_objects.len());
            final_objects.push(obj);
        }
    }
    let final_cells = cells
        .into_iter()
        .filter_map(|mut cell| {
            cell.object = index_map.get(cell.object).copied().flatten()?;
            Some(cell)
        })
        .collect();

    Cow::Owned(Block {
        name: block.name.clone(),
        base_point: block.base_point,
        objects: final_objects,
        cells: final_cells,
        parameters: block.parameters.clone(),
    })
}

fn in_box(p: Point3, min: [f64; 2], max: [f64; 2]) -> bool {
    let eps = 1e-9;
    p.x >= min[0] - eps && p.x <= max[0] + eps && p.y >= min[1] - eps && p.y <= max[1] + eps
}

fn add_pt(p: Point3, dv: Point3) -> Point3 {
    Point3::new(p.x + dv.x, p.y + dv.y, p.z + dv.z)
}

fn translate_object(obj: &mut Object, dv: Point3) {
    match obj {
        Object::Line(Line { from, to, .. }) => {
            *from = add_pt(*from, dv);
            *to = add_pt(*to, dv);
        }
        Object::Polyline(Polyline { points, .. }) => {
            for p in points {
                *p = add_pt(*p, dv);
            }
        }
        Object::Arc(Arc { center, .. })
        | Object::Circle(Circle { center, .. })
        | Object::Ellipse(Ellipse { center, .. })
        | Object::Donut(Donut { center, .. }) => {
            *center = add_pt(*center, dv);
        }
        Object::Spline(Spline {
            fit_points,
            control_points,
            ..
        }) => {
            for p in fit_points.iter_mut().chain(control_points.iter_mut()) {
                *p = add_pt(*p, dv);
            }
        }
        Object::Solid(Solid { corners, .. }) => {
            for c in corners {
                *c = add_pt(*c, dv);
            }
        }
        Object::Wipeout(Wipeout { points, .. }) => {
            for p in points {
                *p = add_pt(*p, dv);
            }
        }
        Object::RasterImage(img) => {
            img.position = add_pt(img.position, dv);
            if let Some(clip) = &mut img.clip {
                for p in clip {
                    p[0] += dv.x;
                    p[1] += dv.y;
                }
            }
        }
        Object::PdfUnderlay(pdf) => {
            pdf.position = add_pt(pdf.position, dv);
            if let Some(clip) = &mut pdf.clip {
                for p in clip {
                    p[0] += dv.x;
                    p[1] += dv.y;
                }
            }
        }
        Object::RevisionCloud(RevisionCloud { boundary, .. }) => match boundary {
            RevisionCloudBoundary::Rectangle { from, to } => {
                *from = add_pt(*from, dv);
                *to = add_pt(*to, dv);
            }
            RevisionCloudBoundary::Polygon(pts) => {
                for p in pts {
                    *p = add_pt(*p, dv);
                }
            }
        },
        Object::Hatch(Hatch {
            boundary, origin, ..
        }) => {
            origin[0] += dv.x;
            origin[1] += dv.y;
            for lp in boundary {
                translate_hatch_loop(lp, dv);
            }
        }
        Object::Text(Text { position, .. }) | Object::MText(MText { position, .. }) => {
            *position = add_pt(*position, dv);
        }
        Object::AttributeDefinition(d) => {
            d.position = add_pt(d.position, dv);
        }
        Object::Insert(ins) => {
            ins.position = add_pt(ins.position, dv);
        }
        Object::Table(_)
        | Object::MLeader(_)
        | Object::Leader(_)
        | Object::Dimension(_)
        | Object::Viewport(_) => {}
    }
}

fn translate_hatch_loop(lp: &mut HatchLoop, dv: Point3) {
    match lp {
        HatchLoop::Polyline { points } => {
            for p in points {
                *p = add_pt(*p, dv);
            }
        }
        HatchLoop::Edges(edges) => {
            for edge in edges {
                match edge {
                    HatchEdge::Line { from, to } => {
                        *from = add_pt(*from, dv);
                        *to = add_pt(*to, dv);
                    }
                    HatchEdge::Arc { center, .. } => {
                        *center = add_pt(*center, dv);
                    }
                }
            }
        }
    }
}

/// Stretches vertices of `obj` inside `[bmin, bmax]` by `dv`. Returns true if
/// any part of `obj` moved.
fn stretch_object(obj: &mut Object, bmin: [f64; 2], bmax: [f64; 2], dv: Point3) -> bool {
    let mut moved = false;
    match obj {
        Object::Line(Line { from, to, .. }) => {
            if in_box(*from, bmin, bmax) {
                *from = add_pt(*from, dv);
                moved = true;
            }
            if in_box(*to, bmin, bmax) {
                *to = add_pt(*to, dv);
                moved = true;
            }
        }
        Object::Polyline(Polyline { points, .. }) => {
            for p in points {
                if in_box(*p, bmin, bmax) {
                    *p = add_pt(*p, dv);
                    moved = true;
                }
            }
        }
        Object::Solid(Solid { corners, .. }) => {
            for c in corners {
                if in_box(*c, bmin, bmax) {
                    *c = add_pt(*c, dv);
                    moved = true;
                }
            }
        }
        Object::Wipeout(Wipeout { points, .. }) => {
            for p in points {
                if in_box(*p, bmin, bmax) {
                    *p = add_pt(*p, dv);
                    moved = true;
                }
            }
        }
        Object::RasterImage(img) => {
            if in_box(img.position, bmin, bmax) {
                img.position = add_pt(img.position, dv);
                if let Some(clip) = &mut img.clip {
                    for p in clip {
                        p[0] += dv.x;
                        p[1] += dv.y;
                    }
                }
                moved = true;
            }
        }
        Object::PdfUnderlay(pdf) => {
            if in_box(pdf.position, bmin, bmax) {
                pdf.position = add_pt(pdf.position, dv);
                if let Some(clip) = &mut pdf.clip {
                    for p in clip {
                        p[0] += dv.x;
                        p[1] += dv.y;
                    }
                }
                moved = true;
            }
        }
        Object::RevisionCloud(RevisionCloud { boundary, .. }) => match boundary {
            RevisionCloudBoundary::Rectangle { from, to } => {
                if in_box(*from, bmin, bmax) {
                    *from = add_pt(*from, dv);
                    moved = true;
                }
                if in_box(*to, bmin, bmax) {
                    *to = add_pt(*to, dv);
                    moved = true;
                }
            }
            RevisionCloudBoundary::Polygon(pts) => {
                for p in pts {
                    if in_box(*p, bmin, bmax) {
                        *p = add_pt(*p, dv);
                        moved = true;
                    }
                }
            }
        },
        Object::Spline(Spline {
            fit_points,
            control_points,
            ..
        }) => {
            for p in fit_points.iter_mut().chain(control_points.iter_mut()) {
                if in_box(*p, bmin, bmax) {
                    *p = add_pt(*p, dv);
                    moved = true;
                }
            }
        }
        Object::Hatch(Hatch { boundary, .. }) => {
            for lp in boundary {
                match lp {
                    HatchLoop::Polyline { points } => {
                        for p in points {
                            if in_box(*p, bmin, bmax) {
                                *p = add_pt(*p, dv);
                                moved = true;
                            }
                        }
                    }
                    HatchLoop::Edges(edges) => {
                        for edge in edges {
                            match edge {
                                HatchEdge::Line { from, to } => {
                                    if in_box(*from, bmin, bmax) {
                                        *from = add_pt(*from, dv);
                                        moved = true;
                                    }
                                    if in_box(*to, bmin, bmax) {
                                        *to = add_pt(*to, dv);
                                        moved = true;
                                    }
                                }
                                HatchEdge::Arc { center, .. } => {
                                    if in_box(*center, bmin, bmax) {
                                        *center = add_pt(*center, dv);
                                        moved = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Object::Arc(Arc { center, .. })
        | Object::Circle(Circle { center, .. })
        | Object::Ellipse(Ellipse { center, .. })
        | Object::Donut(Donut { center, .. }) => {
            if in_box(*center, bmin, bmax) {
                *center = add_pt(*center, dv);
                moved = true;
            }
        }
        Object::Text(Text { position, .. }) | Object::MText(MText { position, .. }) => {
            if in_box(*position, bmin, bmax) {
                *position = add_pt(*position, dv);
                moved = true;
            }
        }
        Object::AttributeDefinition(d) => {
            if in_box(d.position, bmin, bmax) {
                d.position = add_pt(d.position, dv);
                moved = true;
            }
        }
        Object::Insert(ins) => {
            if in_box(ins.position, bmin, bmax) {
                ins.position = add_pt(ins.position, dv);
                moved = true;
            }
        }
        Object::Table(_)
        | Object::MLeader(_)
        | Object::Leader(_)
        | Object::Dimension(_)
        | Object::Viewport(_) => {}
    }
    moved
}

fn rotate_pt(p: Point3, pivot: Point3, sin: f64, cos: f64) -> Point3 {
    let dx = p.x - pivot.x;
    let dy = p.y - pivot.y;
    Point3::new(
        pivot.x + dx * cos - dy * sin,
        pivot.y + dx * sin + dy * cos,
        p.z,
    )
}

fn rotate_2d(p: [f64; 2], pivot: Point3, sin: f64, cos: f64) -> [f64; 2] {
    let dx = p[0] - pivot.x;
    let dy = p[1] - pivot.y;
    [pivot.x + dx * cos - dy * sin, pivot.y + dx * sin + dy * cos]
}

fn rotate_cell(cell: &mut super::block::Cell, pivot: Point3, degrees: f64) {
    let (sin, cos) = sin_cos_degrees(degrees);
    let corners = [
        rotate_2d(cell.min, pivot, sin, cos),
        rotate_2d([cell.max[0], cell.min[1]], pivot, sin, cos),
        rotate_2d(cell.max, pivot, sin, cos),
        rotate_2d([cell.min[0], cell.max[1]], pivot, sin, cos),
    ];
    cell.min = [
        corners.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min),
        corners.iter().map(|c| c[1]).fold(f64::INFINITY, f64::min),
    ];
    cell.max = [
        corners
            .iter()
            .map(|c| c[0])
            .fold(f64::NEG_INFINITY, f64::max),
        corners
            .iter()
            .map(|c| c[1])
            .fold(f64::NEG_INFINITY, f64::max),
    ];
}

fn rotate_object(obj: &mut Object, pivot: Point3, degrees: f64) {
    let (sin, cos) = sin_cos_degrees(degrees);
    match obj {
        Object::Line(Line { from, to, .. }) => {
            *from = rotate_pt(*from, pivot, sin, cos);
            *to = rotate_pt(*to, pivot, sin, cos);
        }
        Object::Polyline(Polyline { points, .. }) => {
            for p in points {
                *p = rotate_pt(*p, pivot, sin, cos);
            }
        }
        Object::Arc(arc) => {
            arc.center = rotate_pt(arc.center, pivot, sin, cos);
            arc.start_angle = normalize_degrees(arc.start_angle + degrees);
            arc.end_angle = normalize_degrees(arc.end_angle + degrees);
        }
        Object::Circle(Circle { center, .. }) | Object::Donut(Donut { center, .. }) => {
            *center = rotate_pt(*center, pivot, sin, cos);
        }
        Object::Ellipse(e) => {
            e.center = rotate_pt(e.center, pivot, sin, cos);
            let mx = e.major_axis.x * cos - e.major_axis.y * sin;
            let my = e.major_axis.x * sin + e.major_axis.y * cos;
            e.major_axis = Point3::new(mx, my, e.major_axis.z);
        }
        Object::Spline(Spline {
            fit_points,
            control_points,
            ..
        }) => {
            for p in fit_points.iter_mut().chain(control_points.iter_mut()) {
                *p = rotate_pt(*p, pivot, sin, cos);
            }
        }
        Object::Solid(Solid { corners, .. }) => {
            for c in corners {
                *c = rotate_pt(*c, pivot, sin, cos);
            }
        }
        Object::Wipeout(Wipeout { points, .. }) => {
            for p in points {
                *p = rotate_pt(*p, pivot, sin, cos);
            }
        }
        Object::RasterImage(img) => {
            img.position = rotate_pt(img.position, pivot, sin, cos);
            img.rotation = normalize_degrees(img.rotation + degrees);
            if let Some(clip) = &mut img.clip {
                for p in clip {
                    *p = rotate_2d(*p, pivot, sin, cos);
                }
            }
        }
        Object::PdfUnderlay(pdf) => {
            pdf.position = rotate_pt(pdf.position, pivot, sin, cos);
            pdf.rotation = normalize_degrees(pdf.rotation + degrees);
            if let Some(clip) = &mut pdf.clip {
                for p in clip {
                    *p = rotate_2d(*p, pivot, sin, cos);
                }
            }
        }
        Object::RevisionCloud(cloud) => {
            let corners = match &cloud.boundary {
                RevisionCloudBoundary::Rectangle { from, to } => vec![
                    Point3::new(from.x, from.y, from.z),
                    Point3::new(to.x, from.y, from.z),
                    Point3::new(to.x, to.y, from.z),
                    Point3::new(from.x, to.y, from.z),
                ],
                RevisionCloudBoundary::Polygon(pts) => pts.clone(),
            };
            cloud.boundary = RevisionCloudBoundary::Polygon(
                corners
                    .into_iter()
                    .map(|p| rotate_pt(p, pivot, sin, cos))
                    .collect(),
            );
        }
        Object::Hatch(h) => {
            h.origin = rotate_2d(h.origin, pivot, sin, cos);
            h.angle = normalize_degrees(h.angle + degrees);
            for lp in &mut h.boundary {
                match lp {
                    HatchLoop::Polyline { points } => {
                        for p in points {
                            *p = rotate_pt(*p, pivot, sin, cos);
                        }
                    }
                    HatchLoop::Edges(edges) => {
                        for edge in edges {
                            match edge {
                                HatchEdge::Line { from, to } => {
                                    *from = rotate_pt(*from, pivot, sin, cos);
                                    *to = rotate_pt(*to, pivot, sin, cos);
                                }
                                HatchEdge::Arc {
                                    center,
                                    start_angle,
                                    end_angle,
                                    ..
                                } => {
                                    *center = rotate_pt(*center, pivot, sin, cos);
                                    *start_angle = normalize_degrees(*start_angle + degrees);
                                    *end_angle = normalize_degrees(*end_angle + degrees);
                                }
                            }
                        }
                    }
                }
            }
        }
        Object::Text(t) => {
            t.position = rotate_pt(t.position, pivot, sin, cos);
            t.rotation = normalize_degrees(t.rotation + degrees);
        }
        Object::MText(t) => {
            t.position = rotate_pt(t.position, pivot, sin, cos);
            t.rotation = normalize_degrees(t.rotation + degrees);
        }
        Object::AttributeDefinition(d) => {
            d.position = rotate_pt(d.position, pivot, sin, cos);
            d.rotation = normalize_degrees(d.rotation + degrees);
        }
        Object::Insert(ins) => {
            ins.position = rotate_pt(ins.position, pivot, sin, cos);
            ins.rotation = normalize_degrees(ins.rotation + degrees);
        }
        Object::Table(_)
        | Object::MLeader(_)
        | Object::Leader(_)
        | Object::Dimension(_)
        | Object::Viewport(_) => {}
    }
}

fn mirror_pt(p: Point3, a: Point3, b: Point3) -> Point3 {
    let [x, y] = mirror_2d([p.x, p.y], a, b);
    Point3::new(x, y, p.z)
}

fn mirror_2d(p: [f64; 2], a: Point3, b: Point3) -> [f64; 2] {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len2 = dx * dx + dy * dy;
    if len2 <= 1e-18 {
        return p;
    }
    let ux = dx / len2.sqrt();
    let uy = dy / len2.sqrt();
    let vx = p[0] - a.x;
    let vy = p[1] - a.y;
    let proj = vx * ux + vy * uy;
    let px = a.x + proj * ux;
    let py = a.y + proj * uy;
    [2.0 * px - p[0], 2.0 * py - p[1]]
}

fn axis_angle_degrees(a: Point3, b: Point3) -> f64 {
    (b.y - a.y).atan2(b.x - a.x).to_degrees()
}

fn mirror_justify(j: Justify) -> Justify {
    match j {
        Justify::Left => Justify::Right,
        Justify::Right => Justify::Left,
        Justify::TopLeft => Justify::TopRight,
        Justify::TopRight => Justify::TopLeft,
        Justify::MiddleLeft => Justify::MiddleRight,
        Justify::MiddleRight => Justify::MiddleLeft,
        Justify::BottomLeft => Justify::BottomRight,
        Justify::BottomRight => Justify::BottomLeft,
        other => other,
    }
}

fn mirror_attachment(a: Attachment) -> Attachment {
    match a {
        Attachment::TopLeft => Attachment::TopRight,
        Attachment::TopRight => Attachment::TopLeft,
        Attachment::MiddleLeft => Attachment::MiddleRight,
        Attachment::MiddleRight => Attachment::MiddleLeft,
        Attachment::BottomLeft => Attachment::BottomRight,
        Attachment::BottomRight => Attachment::BottomLeft,
        other => other,
    }
}

fn mirror_cell(cell: &mut super::block::Cell, a: Point3, b: Point3) {
    let corners = [
        mirror_2d(cell.min, a, b),
        mirror_2d([cell.max[0], cell.min[1]], a, b),
        mirror_2d(cell.max, a, b),
        mirror_2d([cell.min[0], cell.max[1]], a, b),
    ];
    cell.min = [
        corners.iter().map(|c| c[0]).fold(f64::INFINITY, f64::min),
        corners.iter().map(|c| c[1]).fold(f64::INFINITY, f64::min),
    ];
    cell.max = [
        corners
            .iter()
            .map(|c| c[0])
            .fold(f64::NEG_INFINITY, f64::max),
        corners
            .iter()
            .map(|c| c[1])
            .fold(f64::NEG_INFINITY, f64::max),
    ];
}

fn mirror_object(obj: &mut Object, a: Point3, b: Point3) {
    let axis_deg = axis_angle_degrees(a, b);
    match obj {
        Object::Line(Line { from, to, .. }) => {
            *from = mirror_pt(*from, a, b);
            *to = mirror_pt(*to, a, b);
        }
        Object::Polyline(pl) => {
            for p in &mut pl.points {
                *p = mirror_pt(*p, a, b);
            }
            for bulge in &mut pl.bulges {
                *bulge = -*bulge;
            }
        }
        Object::Arc(arc) => {
            arc.center = mirror_pt(arc.center, a, b);
            let new_start = normalize_degrees(2.0 * axis_deg - arc.end_angle);
            let new_end = normalize_degrees(2.0 * axis_deg - arc.start_angle);
            arc.start_angle = new_start;
            arc.end_angle = new_end;
        }
        Object::Circle(Circle { center, .. }) | Object::Donut(Donut { center, .. }) => {
            *center = mirror_pt(*center, a, b);
        }
        Object::Ellipse(e) => {
            let tip = add_pt(e.center, e.major_axis);
            let new_center = mirror_pt(e.center, a, b);
            let new_tip = mirror_pt(tip, a, b);
            e.center = new_center;
            e.major_axis = Point3::new(
                new_tip.x - new_center.x,
                new_tip.y - new_center.y,
                e.major_axis.z,
            );
            if !e.is_whole() {
                let new_start = normalize_degrees(360.0 - e.end_angle);
                let new_end = normalize_degrees(360.0 - e.start_angle);
                e.start_angle = new_start;
                e.end_angle = new_end;
            }
        }
        Object::Spline(Spline {
            fit_points,
            control_points,
            ..
        }) => {
            for p in fit_points.iter_mut().chain(control_points.iter_mut()) {
                *p = mirror_pt(*p, a, b);
            }
        }
        Object::Solid(Solid { corners, .. }) => {
            for c in corners {
                *c = mirror_pt(*c, a, b);
            }
        }
        Object::Wipeout(Wipeout { points, .. }) => {
            for p in points {
                *p = mirror_pt(*p, a, b);
            }
        }
        Object::RasterImage(img) => {
            img.position = mirror_pt(img.position, a, b);
            if let Some(clip) = &mut img.clip {
                for p in clip {
                    *p = mirror_2d(*p, a, b);
                }
            }
        }
        Object::PdfUnderlay(pdf) => {
            pdf.position = mirror_pt(pdf.position, a, b);
            if let Some(clip) = &mut pdf.clip {
                for p in clip {
                    *p = mirror_2d(*p, a, b);
                }
            }
        }
        Object::RevisionCloud(cloud) => {
            let corners = match &cloud.boundary {
                RevisionCloudBoundary::Rectangle { from, to } => vec![
                    Point3::new(from.x, from.y, from.z),
                    Point3::new(to.x, from.y, from.z),
                    Point3::new(to.x, to.y, from.z),
                    Point3::new(from.x, to.y, from.z),
                ],
                RevisionCloudBoundary::Polygon(pts) => pts.clone(),
            };
            let mut mirrored: Vec<Point3> =
                corners.into_iter().map(|p| mirror_pt(p, a, b)).collect();
            mirrored.reverse();
            cloud.boundary = RevisionCloudBoundary::Polygon(mirrored);
        }
        Object::Hatch(h) => {
            h.origin = mirror_2d(h.origin, a, b);
            h.angle = normalize_degrees(2.0 * axis_deg - h.angle);
            for lp in &mut h.boundary {
                match lp {
                    HatchLoop::Polyline { points } => {
                        for p in points {
                            *p = mirror_pt(*p, a, b);
                        }
                    }
                    HatchLoop::Edges(edges) => {
                        for edge in edges {
                            match edge {
                                HatchEdge::Line { from, to } => {
                                    *from = mirror_pt(*from, a, b);
                                    *to = mirror_pt(*to, a, b);
                                }
                                HatchEdge::Arc {
                                    center,
                                    start_angle,
                                    end_angle,
                                    ..
                                } => {
                                    *center = mirror_pt(*center, a, b);
                                    let new_start = normalize_degrees(2.0 * axis_deg - *end_angle);
                                    let new_end = normalize_degrees(2.0 * axis_deg - *start_angle);
                                    *start_angle = new_start;
                                    *end_angle = new_end;
                                }
                            }
                        }
                    }
                }
            }
        }
        Object::Text(t) => {
            t.position = mirror_pt(t.position, a, b);
            let raw = normalize_degrees(2.0 * axis_deg - t.rotation);
            let read = readable(raw);
            if (read - raw).abs() > 1e-6 {
                t.justify = mirror_justify(t.justify);
            }
            t.rotation = read;
        }
        Object::MText(t) => {
            t.position = mirror_pt(t.position, a, b);
            let raw = normalize_degrees(2.0 * axis_deg - t.rotation);
            let read = readable(raw);
            if (read - raw).abs() > 1e-6 {
                t.justify = mirror_attachment(t.justify);
            }
            t.rotation = read;
        }
        Object::AttributeDefinition(d) => {
            d.position = mirror_pt(d.position, a, b);
            let raw = normalize_degrees(2.0 * axis_deg - d.rotation);
            let read = readable(raw);
            if (read - raw).abs() > 1e-6 {
                d.justify = mirror_justify(d.justify);
            }
            d.rotation = read;
        }
        Object::Insert(ins) => {
            ins.position = mirror_pt(ins.position, a, b);
            ins.mirrored = !ins.mirrored;
            ins.rotation = normalize_degrees(2.0 * axis_deg - ins.rotation + 180.0);
        }
        Object::Table(_)
        | Object::MLeader(_)
        | Object::Leader(_)
        | Object::Dimension(_)
        | Object::Viewport(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Properties, LAYER_ZERO};

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Object {
        Object::Line(Line {
            from: Point3::new(x0, y0, 0.0),
            to: Point3::new(x1, y1, 0.0),
            layer: LAYER_ZERO.to_owned(),
            properties: Properties::default(),
        })
    }

    #[test]
    fn visibility_filters_objects_by_state() {
        let block = Block {
            name: "SW".into(),
            base_point: Point3::new(0.0, 0.0, 0.0),
            objects: vec![line(0.0, 0.0, 10.0, 0.0), line(0.0, 0.0, 0.0, 10.0)],
            cells: Vec::new(),
            parameters: vec![DynamicParameter {
                name: "Visibility".into(),
                kind: DynamicParameterKind::Visibility {
                    default: "H".into(),
                    states: vec![
                        VisibilityState {
                            name: "H".into(),
                            objects: vec![0],
                        },
                        VisibilityState {
                            name: "V".into(),
                            objects: vec![1],
                        },
                    ],
                },
            }],
        };
        let mut st = BTreeMap::new();
        st.insert("Visibility".into(), DynamicValue::Text("V".into()));
        let eval = evaluate_block(&block, &st);
        assert_eq!(eval.objects.len(), 1);
        assert_eq!(eval.objects[0], line(0.0, 0.0, 0.0, 10.0));
    }

    #[test]
    fn linear_stretch_moves_vertices_inside_window() {
        let block = Block {
            name: "BAR".into(),
            base_point: Point3::new(0.0, 0.0, 0.0),
            objects: vec![line(0.0, 0.0, 10.0, 0.0)],
            cells: Vec::new(),
            parameters: vec![DynamicParameter {
                name: "Length".into(),
                kind: DynamicParameterKind::Linear {
                    from: Point3::new(0.0, 0.0, 0.0),
                    to: Point3::new(10.0, 0.0, 0.0),
                    default: 10.0,
                    min: Some(5.0),
                    max: Some(50.0),
                    increment: None,
                    allowed: Vec::new(),
                    stretch_box: Some([[8.0, -2.0], [12.0, 2.0]]),
                    stretch_objects: vec![0],
                    move_objects: Vec::new(),
                },
            }],
        };
        let mut st = BTreeMap::new();
        st.insert("Length".into(), DynamicValue::Number(25.0));
        let eval = evaluate_block(&block, &st);
        assert_eq!(eval.objects[0], line(0.0, 0.0, 25.0, 0.0));
    }

    #[test]
    fn flip_and_rotation_and_lookup_evaluate_in_order() {
        let block = Block {
            name: "VALVE".into(),
            base_point: Point3::new(0.0, 0.0, 0.0),
            objects: vec![line(0.0, 0.0, 10.0, 0.0)],
            cells: Vec::new(),
            parameters: vec![
                DynamicParameter {
                    name: "Angle".into(),
                    kind: DynamicParameterKind::Rotation {
                        base_point: Point3::new(0.0, 0.0, 0.0),
                        default: 0.0,
                        min: None,
                        max: None,
                        allowed: Vec::new(),
                        objects: Vec::new(),
                    },
                },
                DynamicParameter {
                    name: "Flip".into(),
                    kind: DynamicParameterKind::Flip {
                        axis: [Point3::new(0.0, 0.0, 0.0), Point3::new(0.0, 10.0, 0.0)],
                        default: false,
                        not_flipped_label: "Not Flipped".into(),
                        flipped_label: "Flipped".into(),
                        objects: Vec::new(),
                    },
                },
                DynamicParameter {
                    name: "Preset".into(),
                    kind: DynamicParameterKind::Lookup {
                        default: "Right".into(),
                        rows: vec![
                            LookupRow {
                                name: "Right".into(),
                                values: BTreeMap::from([(
                                    "Flip".into(),
                                    DynamicValue::Bool(false),
                                )]),
                            },
                            LookupRow {
                                name: "Left".into(),
                                values: BTreeMap::from([("Flip".into(), DynamicValue::Bool(true))]),
                            },
                        ],
                    },
                },
            ],
        };
        let mut st = BTreeMap::new();
        st.insert("Preset".into(), DynamicValue::Text("Left".into()));
        let eval = evaluate_block(&block, &st);
        let Object::Line(l) = &eval.objects[0] else {
            panic!("expected line");
        };
        assert!((l.to.x - (-10.0)).abs() < 1e-9);
        assert!(l.to.y.abs() < 1e-9);
    }
}
