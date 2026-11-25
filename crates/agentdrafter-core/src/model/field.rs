//! AutoCAD fields (`FIELD`): live text values in single-line text, multiline
//! text and block insert attributes (#87).
//!
//! A [`TextField`] describes a template string (defaulting to `"{0}"`) whose
//! numbered slots `{0}`, `{1}`, ... are filled by one or more [`FieldItem`]
//! definitions: the drawing's file name, a save/plot/create/current date, a
//! sheet/layout tab name, a custom drawing property (`DWGPROPS`), a property
//! of another object on the sheet, or a raw AutoCAD field code preserved from
//! an existing DWG.
//!
//! When written to a DWG, each field becomes AutoCAD's `ACAD_FIELD` / `FIELD`
//! / `FIELDLIST` structure so AutoCAD updates it live. When rendered, plotted,
//! queried or shown in the viewer, the core draws the field's evaluated value
//! as it stands through the one drawing pipeline (ADR-0011).

use std::collections::BTreeMap;
use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

use super::{Object, ObjectColor, ObjectLinetype, Sheet};

/// AutoCAD's standard placeholder for a field whose value has not yet been
/// evaluated (for example a save or plot date before AutoCAD saves or plots
/// the file).
pub const UNEVALUATED_FIELD: &str = "----";

/// The default template for a single-item field: just the first item's value.
pub const DEFAULT_FIELD_TEMPLATE: &str = "{0}";

pub(crate) fn default_field_template() -> String {
    DEFAULT_FIELD_TEMPLATE.to_owned()
}

pub(crate) fn is_default_field_template(template: &str) -> bool {
    template == DEFAULT_FIELD_TEMPLATE
}

/// The kind of value an AutoCAD field evaluates to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    /// The drawing's file name (`\AcVar Filename`).
    Filename,
    /// The date the drawing was last saved (`\AcVar SaveDate`).
    SaveDate,
    /// The date the drawing was last plotted (`\AcVar PlotDate`).
    PlotDate,
    /// The date the drawing was created (`\AcVar CreateDate`).
    CreateDate,
    /// The current date on demand (`\AcVar Date`).
    Date,
    /// The sheet/layout tab name (`\AcVar ctab`).
    SheetNumber,
    /// A custom drawing property from `DWGPROPS` (`\AcVar CustomDP.<name>`).
    CustomProperty,
    /// A property of another object on the sheet
    /// (`\AcObjProp Object(%<\_ObjIdx 0>%).<property>`).
    ObjectProperty,
    /// A raw AutoCAD field code preserved verbatim (for example `\AcDiesel`,
    /// `\AcSm`, or system variables not modelled as a dedicated kind).
    Code,
}

impl FieldKind {
    /// Every field kind, in schema order.
    pub const ALL: [FieldKind; 9] = [
        FieldKind::Filename,
        FieldKind::SaveDate,
        FieldKind::PlotDate,
        FieldKind::CreateDate,
        FieldKind::Date,
        FieldKind::SheetNumber,
        FieldKind::CustomProperty,
        FieldKind::ObjectProperty,
        FieldKind::Code,
    ];

    /// The sheet-description and JSON name of this field kind.
    pub const fn name(self) -> &'static str {
        match self {
            FieldKind::Filename => "filename",
            FieldKind::SaveDate => "save_date",
            FieldKind::PlotDate => "plot_date",
            FieldKind::CreateDate => "create_date",
            FieldKind::Date => "date",
            FieldKind::SheetNumber => "sheet_number",
            FieldKind::CustomProperty => "custom_property",
            FieldKind::ObjectProperty => "object_property",
            FieldKind::Code => "code",
        }
    }

    /// Parses a field kind from its sheet-description / JSON name.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Comma-separated list of all kind names for error messages.
    pub fn choices() -> String {
        Self::ALL
            .iter()
            .map(|k| k.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One AutoCAD child field inside a [`TextField`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldItem {
    /// What kind of value the field pulls.
    pub kind: FieldKind,
    /// For `custom_property`, the custom drawing property name; for
    /// `object_property`, the AutoCAD object property name (such as `Length`,
    /// `Area`, `Radius`, `Diameter`, `Circumference`, `Layer`, `Color`,
    /// `Linetype`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    /// For `object_property`, the local ID of the referenced object on the
    /// sheet (or `#<HEX>` when read from a DWG whose referenced object carries
    /// no AgentDrafter ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// For `code`, the raw AutoCAD field expression (without outer `%<...>%`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// For `code`, optional referenced object IDs matching `\_ObjIdx 0`, `1`, ...
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    /// Optional AutoCAD `\f "..."` format string (for example `"%fn2"`,
    /// `"%fn6"`, `"yyyy-MM-dd"`, `"%lu2%pr2"`, `"%tc1"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Cached or explicit display value for this field item. When omitted on a
    /// sheet description, the core evaluates the value deterministically where
    /// possible and uses [`UNEVALUATED_FIELD`] (`"----"`) otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl FieldItem {
    /// Builds the AutoCAD child field code (the inner expression inside
    /// `%<...>%` stored in the child `AcDbField` record).
    pub fn autocad_code(&self) -> String {
        let base = match self.kind {
            FieldKind::Filename => "\\AcVar Filename".to_owned(),
            FieldKind::SaveDate => "\\AcVar SaveDate".to_owned(),
            FieldKind::PlotDate => "\\AcVar PlotDate".to_owned(),
            FieldKind::CreateDate => "\\AcVar CreateDate".to_owned(),
            FieldKind::Date => "\\AcVar Date".to_owned(),
            FieldKind::SheetNumber => "\\AcVar ctab".to_owned(),
            FieldKind::CustomProperty => {
                let prop = self.property.as_deref().unwrap_or("");
                format!("\\AcVar CustomDP.{prop}")
            }
            FieldKind::ObjectProperty => {
                let prop = self.property.as_deref().unwrap_or("");
                format!("\\AcObjProp Object(%<\\_ObjIdx 0>%).{prop}")
            }
            FieldKind::Code => {
                let raw = self.code.as_deref().unwrap_or("").trim();
                if let Some(fmt) = self.format.as_deref().filter(|f| !f.is_empty()) {
                    if !raw.contains("\\f ") {
                        return format!("{raw} \\f \"{fmt}\"");
                    }
                }
                return raw.to_owned();
            }
        };
        match self.format.as_deref().filter(|f| !f.is_empty()) {
            Some(fmt) => format!("{base} \\f \"{fmt}\""),
            None => base,
        }
    }

    /// The local object IDs (or `#<HEX>` handles) referenced by this item, in
    /// `\_ObjIdx` order.
    pub fn referenced_objects(&self) -> Vec<&str> {
        match self.kind {
            FieldKind::ObjectProperty => self.object.as_deref().into_iter().collect(),
            FieldKind::Code => self.objects.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        }
    }
}

/// An AutoCAD field attached to a `text`, `mtext`, or insert `attribute`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextField {
    /// Template text with `{0}`, `{1}`, ... slots for each item in [`Self::items`].
    /// Defaults to `"{0}"`.
    #[serde(
        default = "default_field_template",
        skip_serializing_if = "is_default_field_template"
    )]
    pub template: String,
    /// The child fields that fill `{0}`, `{1}`, ... in [`Self::template`].
    pub items: Vec<FieldItem>,
}

impl TextField {
    /// A single-item field with the default `"{0}"` template.
    pub fn single(item: FieldItem) -> Self {
        Self {
            template: default_field_template(),
            items: vec![item],
        }
    }

    /// Splits `self.template` into alternating literal text chunks and item
    /// indices (`0..self.items.len()`).
    pub(crate) fn template_parts(&self) -> Vec<TemplatePart<'_>> {
        let mut parts = Vec::new();
        let mut rest = self.template.as_str();
        let mut literal_start = 0usize;
        let bytes = self.template.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if let Some(rel_end) = self.template[i + 1..].find('}') {
                    let inner = &self.template[i + 1..i + 1 + rel_end];
                    if !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit()) {
                        if let Ok(idx) = inner.parse::<usize>() {
                            if idx < self.items.len() {
                                if i > literal_start {
                                    parts.push(TemplatePart::Literal(
                                        &self.template[literal_start..i],
                                    ));
                                }
                                parts.push(TemplatePart::Slot(idx));
                                i = i + 1 + rel_end + 1;
                                literal_start = i;
                                rest = &self.template[literal_start..];
                                continue;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        if !rest.is_empty() {
            parts.push(TemplatePart::Literal(rest));
        }
        parts
    }

    /// Renders the full display string given one evaluated display string per
    /// item in [`Self::items`].
    pub fn render_display(&self, slot_values: &[String]) -> String {
        let mut out = String::new();
        for part in self.template_parts() {
            match part {
                TemplatePart::Literal(lit) => out.push_str(lit),
                TemplatePart::Slot(idx) => {
                    if let Some(val) = slot_values.get(idx) {
                        out.push_str(val);
                    }
                }
            }
        }
        out
    }

    /// Validates that every `{N}` slot used in `template` is in range and that
    /// `items` is non-empty.
    pub(crate) fn validate_slots(&self) -> Result<(), String> {
        if self.items.is_empty() {
            return Err("a field needs at least one item".to_owned());
        }
        let mut used = vec![false; self.items.len()];
        let bytes = self.template.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                if let Some(rel_end) = self.template[i + 1..].find('}') {
                    let inner = &self.template[i + 1..i + 1 + rel_end];
                    if !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit()) {
                        let idx: usize = inner.parse().unwrap_or(usize::MAX);
                        if idx >= self.items.len() {
                            return Err(format!(
                                "field template refers to `{{{idx}}}`, but the field has only {} item{}",
                                self.items.len(),
                                if self.items.len() == 1 { "" } else { "s" }
                            ));
                        }
                        used[idx] = true;
                        i = i + 1 + rel_end + 1;
                        continue;
                    }
                }
            }
            i += 1;
        }
        if !used.iter().any(|&u| u) {
            return Err(
                "field `template` must contain at least one slot placeholder such as `{0}`"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

/// One segment of a parsed [`TextField::template`].
pub(crate) enum TemplatePart<'a> {
    Literal(&'a str),
    Slot(usize),
}

/// Context needed to evaluate a [`TextField`] deterministically from sheet data
/// (without consulting the wall clock, so builds stay byte-for-byte identical
/// across platforms).
pub(crate) struct EvalContext<'a> {
    /// The sheet's file stem / name (for example `"DEMO-001"`).
    pub sheet_name: Option<&'a str>,
    /// The active layout name for the host object (for example `"Layout1"` or
    /// `"Model"`).
    pub layout_name: Option<&'a str>,
    /// The sheet's custom drawing properties (`DWGPROPS`).
    pub custom_properties: &'a BTreeMap<String, String>,
    /// The sheet's objects and their local IDs (parallel slices), used to
    /// evaluate `object_property` fields.
    pub objects: &'a [Object],
    pub object_ids: &'a [String],
    /// Optional hex handles parallel to `objects` (for originals read from DWG).
    pub object_handles: &'a [String],
}

impl<'a> EvalContext<'a> {
    pub(crate) fn from_sheet(
        sheet: &'a Sheet,
        sheet_name: Option<&'a str>,
        layout_name: Option<&'a str>,
        object_ids: &'a [String],
        object_handles: &'a [String],
    ) -> Self {
        Self {
            sheet_name,
            layout_name,
            custom_properties: &sheet.custom_properties,
            objects: &sheet.objects,
            object_ids,
            object_handles,
        }
    }

    /// Finds an object referenced by `reference` (a local ID, full
    /// `<sheet>/<local>` ID, or `#<HEX>` handle).
    pub(crate) fn find_object(&self, reference: &str) -> Option<&'a Object> {
        if let Some(hex) = reference.strip_prefix('#') {
            if let Some(pos) = self
                .object_handles
                .iter()
                .position(|h| h.eq_ignore_ascii_case(hex))
            {
                return self.objects.get(pos);
            }
        }
        let local = reference
            .rsplit_once('/')
            .map_or(reference, |(_, tail)| tail);
        if let Some(pos) = self
            .object_ids
            .iter()
            .position(|id| id.eq_ignore_ascii_case(reference) || id.eq_ignore_ascii_case(local))
        {
            return self.objects.get(pos);
        }
        None
    }
}

/// Evaluates every item of `field` deterministically, returning `(slot_values,
/// full_display)`.
pub(crate) fn evaluate_text_field(
    field: &TextField,
    ctx: &EvalContext<'_>,
) -> (Vec<String>, String) {
    let slots: Vec<String> = field
        .items
        .iter()
        .map(|item| evaluate_field_item(item, ctx))
        .collect();
    let full = field.render_display(&slots);
    (slots, full)
}

/// Evaluates one [`FieldItem`] deterministically.
pub(crate) fn evaluate_field_item(item: &FieldItem, ctx: &EvalContext<'_>) -> String {
    if let Some(explicit) = item.value.as_deref().filter(|v| !v.is_empty()) {
        return explicit.to_owned();
    }
    let fmt = item.format.as_deref().unwrap_or("");
    let raw = match item.kind {
        FieldKind::Filename => {
            let stem = ctx.sheet_name.filter(|s| !s.is_empty());
            match stem {
                Some(stem) => format_filename(stem, fmt),
                None => UNEVALUATED_FIELD.to_owned(),
            }
        }
        FieldKind::SheetNumber => {
            let tab = ctx
                .layout_name
                .filter(|s| !s.is_empty())
                .or_else(|| ctx.sheet_name.filter(|s| !s.is_empty()))
                .unwrap_or("Model");
            apply_text_case(tab, fmt)
        }
        FieldKind::CustomProperty => {
            let wanted = item.property.as_deref().unwrap_or("");
            let found = ctx
                .custom_properties
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(wanted))
                .map(|(_, v)| v.as_str())
                .filter(|v| !v.is_empty());
            match found {
                Some(v) => apply_text_case(v, fmt),
                None => UNEVALUATED_FIELD.to_owned(),
            }
        }
        FieldKind::ObjectProperty => {
            let target = item.object.as_deref().and_then(|r| ctx.find_object(r));
            let prop = item.property.as_deref().unwrap_or("");
            match target.and_then(|obj| eval_object_property(obj, prop, fmt)) {
                Some(v) => v,
                None => UNEVALUATED_FIELD.to_owned(),
            }
        }
        FieldKind::SaveDate
        | FieldKind::PlotDate
        | FieldKind::CreateDate
        | FieldKind::Date
        | FieldKind::Code => UNEVALUATED_FIELD.to_owned(),
    };
    raw
}

fn format_filename(stem: &str, fmt: &str) -> String {
    let base = if fmt.contains("%fn2") {
        // File name only, without extension.
        stem.to_owned()
    } else if fmt.contains("%fn1") {
        // Path only (unknown before saving).
        String::new()
    } else {
        // `%fn6`, `%fn7`, or default: filename with `.dwg` extension.
        if stem.to_ascii_lowercase().ends_with(".dwg") {
            stem.to_owned()
        } else {
            format!("{stem}.dwg")
        }
    };
    if base.is_empty() {
        return UNEVALUATED_FIELD.to_owned();
    }
    apply_text_case(&base, fmt)
}

fn apply_text_case(text: &str, fmt: &str) -> String {
    if fmt.contains("%tc1") {
        text.to_uppercase()
    } else if fmt.contains("%tc2") {
        text.to_lowercase()
    } else if fmt.contains("%tc3") {
        let mut chars = text.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
            None => String::new(),
        }
    } else if fmt.contains("%tc4") {
        let mut out = String::with_capacity(text.len());
        let mut start = true;
        for c in text.chars() {
            if c.is_whitespace() {
                start = true;
                out.push(c);
            } else if start {
                start = false;
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
        }
        out
    } else {
        text.to_owned()
    }
}

fn eval_object_property(object: &Object, prop: &str, fmt: &str) -> Option<String> {
    let lower = prop.to_ascii_lowercase();
    match lower.as_str() {
        "layer" => return Some(apply_text_case(object.layer(), fmt)),
        "color" => {
            let text = match object.properties().color {
                ObjectColor::ByLayer => "ByLayer".to_owned(),
                ObjectColor::ByBlock => "ByBlock".to_owned(),
                ObjectColor::Index(i) => i.to_string(),
                ObjectColor::Rgb([r, g, b]) => format!("{r},{g},{b}"),
            };
            return Some(apply_text_case(&text, fmt));
        }
        "linetype" => {
            let text = match &object.properties().linetype {
                ObjectLinetype::ByLayer => "ByLayer",
                ObjectLinetype::ByBlock => "ByBlock",
                ObjectLinetype::Named(n) => n.as_str(),
            };
            return Some(apply_text_case(text, fmt));
        }
        _ => {}
    }

    let number = match (lower.as_str(), object) {
        ("radius", Object::Circle(c)) => Some(c.radius),
        ("radius", Object::Arc(a)) => Some(a.radius),
        ("diameter", Object::Circle(c)) => Some(c.radius * 2.0),
        ("diameter", Object::Arc(a)) => Some(a.radius * 2.0),
        ("circumference", Object::Circle(c)) => Some(2.0 * PI * c.radius),
        ("length", obj) => object_length(obj),
        ("area", obj) => object_area(obj),
        _ => None,
    }?;

    Some(format_field_number(number, fmt))
}

fn object_length(object: &Object) -> Option<f64> {
    match object {
        Object::Line(l) => Some(
            ((l.to.x - l.from.x).powi(2)
                + (l.to.y - l.from.y).powi(2)
                + (l.to.z - l.from.z).powi(2))
            .sqrt(),
        ),
        Object::Circle(c) => Some(2.0 * PI * c.radius),
        Object::Arc(a) => {
            let sweep = (a.end_angle - a.start_angle).rem_euclid(360.0);
            let sweep = if sweep == 0.0 { 360.0 } else { sweep };
            Some(a.radius * sweep.to_radians())
        }
        Object::Polyline(p) => {
            let n = p.points.len();
            if n < 2 {
                return Some(0.0);
            }
            let mut total = 0.0;
            let count = if p.closed { n } else { n - 1 };
            for i in 0..count {
                if let Some(curve) = crate::geometry::polyline_segment(p, i) {
                    total += curve.length();
                }
            }
            Some(total)
        }
        Object::Donut(d) => object_length(&Object::Polyline(d.polyline())),
        Object::RevisionCloud(rc) => object_length(&Object::Polyline(rc.polyline())),
        Object::Ellipse(e) => Some(crate::geometry::ellipse_curve(e).length()),
        Object::Spline(s) => Some(crate::geometry::spline_curve(s)?.length()),
        _ => None,
    }
}

fn object_area(object: &Object) -> Option<f64> {
    match object {
        Object::Circle(c) => Some(PI * c.radius * c.radius),
        Object::Ellipse(e) if e.is_whole() => {
            let a = e.major_axis.x.hypot(e.major_axis.y);
            Some(PI * a * a * e.ratio)
        }
        Object::Polyline(p) if p.closed && p.points.len() >= 3 => {
            let mut area2 = 0.0;
            let n = p.points.len();
            for i in 0..n {
                let a = p.points[i];
                let b = p.points[(i + 1) % n];
                area2 += a.x * b.y - b.x * a.y;
                let bulge = p.bulge(i);
                if bulge.abs() > 1e-12 {
                    let chord = (b.x - a.x).hypot(b.y - a.y);
                    let theta = 4.0 * bulge.atan();
                    let r = chord / (2.0 * (theta / 2.0).sin());
                    let seg = 0.5 * r * r * (theta - theta.sin());
                    area2 += 2.0 * seg;
                }
            }
            Some((area2 * 0.5).abs())
        }
        Object::Donut(d) => {
            let r_out = d.outside_diameter * 0.5;
            let r_in = d.inside_diameter * 0.5;
            Some(PI * (r_out * r_out - r_in * r_in))
        }
        Object::Solid(s) if s.corners.len() >= 3 => {
            let mut area2 = 0.0;
            let n = s.corners.len();
            for i in 0..n {
                let a = s.corners[i];
                let b = s.corners[(i + 1) % n];
                area2 += a.x * b.y - b.x * a.y;
            }
            Some((area2 * 0.5).abs())
        }
        Object::Wipeout(w) if w.points.len() >= 3 => {
            let mut area2 = 0.0;
            let n = w.points.len();
            for i in 0..n {
                let a = w.points[i];
                let b = w.points[(i + 1) % n];
                area2 += a.x * b.y - b.x * a.y;
            }
            Some((area2 * 0.5).abs())
        }
        _ => None,
    }
}

fn format_field_number(value: f64, fmt: &str) -> String {
    let precision = parse_precision(fmt).unwrap_or(2);
    format!("{value:.precision$}")
}

fn parse_precision(fmt: &str) -> Option<usize> {
    let idx = fmt.find("%pr")?;
    let rest = &fmt[idx + 3..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let p: usize = digits.parse().ok()?;
    Some(p.min(8))
}

/// Parses an AutoCAD child field code (the inner expression without `%<...>%`)
/// into a [`FieldItem`].
pub(crate) fn parse_autocad_code(
    code: &str,
    ref_ids: &[String],
    cached_value: Option<String>,
) -> FieldItem {
    let trimmed = code.trim();
    let (body, format) = split_format(trimmed);
    let body = body.trim();
    let value = cached_value.filter(|v| !v.is_empty());

    if let Some(var_rest) = strip_prefix_ignore_ascii_case(body, "\\AcVar") {
        let var = var_rest.trim();
        if var.eq_ignore_ascii_case("Filename") {
            return FieldItem {
                kind: FieldKind::Filename,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format,
                value,
            };
        }
        if var.eq_ignore_ascii_case("SaveDate") {
            return FieldItem {
                kind: FieldKind::SaveDate,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format,
                value,
            };
        }
        if var.eq_ignore_ascii_case("PlotDate") {
            return FieldItem {
                kind: FieldKind::PlotDate,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format,
                value,
            };
        }
        if var.eq_ignore_ascii_case("CreateDate") {
            return FieldItem {
                kind: FieldKind::CreateDate,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format,
                value,
            };
        }
        if var.eq_ignore_ascii_case("Date") {
            return FieldItem {
                kind: FieldKind::Date,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format,
                value,
            };
        }
        if var.eq_ignore_ascii_case("ctab") {
            return FieldItem {
                kind: FieldKind::SheetNumber,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format,
                value,
            };
        }
        if let Some(prop) = strip_prefix_ignore_ascii_case(var, "CustomDP.") {
            let prop = prop.trim();
            if !prop.is_empty() {
                return FieldItem {
                    kind: FieldKind::CustomProperty,
                    property: Some(prop.to_owned()),
                    object: None,
                    code: None,
                    objects: Vec::new(),
                    format,
                    value,
                };
            }
        }
    }

    if let Some(rest) = strip_prefix_ignore_ascii_case(body, "\\AcObjProp") {
        // Handles both `\AcObjProp Object(%<\_ObjIdx 0>%).Length` and
        // `\AcObjProp.16.2 Object(%<\_ObjIdx 0>%).Length`.
        if let Some(obj_pos) = rest.find("Object(") {
            let after_obj = &rest[obj_pos + "Object(".len()..];
            if let Some(close_pos) = after_obj.find(").") {
                let inner = &after_obj[..close_pos];
                let prop = after_obj[close_pos + 2..].trim();
                let obj_idx = parse_obj_idx(inner).unwrap_or(0);
                if !prop.is_empty() {
                    return FieldItem {
                        kind: FieldKind::ObjectProperty,
                        property: Some(prop.to_owned()),
                        object: ref_ids.get(obj_idx).cloned(),
                        code: None,
                        objects: Vec::new(),
                        format,
                        value,
                    };
                }
            }
        }
    }

    FieldItem {
        kind: FieldKind::Code,
        property: None,
        object: None,
        code: Some(trimmed.to_owned()),
        objects: ref_ids.to_vec(),
        format: None,
        value,
    }
}

fn split_format(code: &str) -> (&str, Option<String>) {
    // Find top-level `\f "..."` outside nested `%<...>%`.
    let bytes = code.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"%<") {
            depth += 1;
            i += 2;
        } else if bytes[i..].starts_with(b">%") {
            depth = depth.saturating_sub(1);
            i += 2;
        } else if depth == 0 && bytes[i..].starts_with(b"\\f ") {
            let rest = code[i + 3..].trim();
            if let Some(unquoted) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                return (code[..i].trim(), Some(unquoted.to_owned()));
            }
            return (code[..i].trim(), Some(rest.to_owned()));
        } else {
            i += 1;
        }
    }
    (code, None)
}

fn strip_prefix_ignore_ascii_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() {
        return None;
    }
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn parse_obj_idx(inner: &str) -> Option<usize> {
    let pos = inner.find("_ObjIdx")?;
    let after = inner[pos + "_ObjIdx".len()..].trim();
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Circle, Line, Point3, Properties};

    #[test]
    fn field_codes_round_trip_through_parser() {
        let cases = [
            FieldItem {
                kind: FieldKind::Filename,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format: Some("%fn2".into()),
                value: None,
            },
            FieldItem {
                kind: FieldKind::SaveDate,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format: Some("yyyy-MM-dd".into()),
                value: None,
            },
            FieldItem {
                kind: FieldKind::PlotDate,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format: None,
                value: None,
            },
            FieldItem {
                kind: FieldKind::SheetNumber,
                property: None,
                object: None,
                code: None,
                objects: Vec::new(),
                format: Some("%tc1".into()),
                value: None,
            },
            FieldItem {
                kind: FieldKind::CustomProperty,
                property: Some("Project".into()),
                object: None,
                code: None,
                objects: Vec::new(),
                format: None,
                value: None,
            },
            FieldItem {
                kind: FieldKind::ObjectProperty,
                property: Some("Length".into()),
                object: Some("pipe-1".into()),
                code: None,
                objects: Vec::new(),
                format: Some("%lu2%pr2".into()),
                value: None,
            },
        ];
        for item in cases {
            let code = item.autocad_code();
            let refs: Vec<String> = item
                .referenced_objects()
                .into_iter()
                .map(str::to_owned)
                .collect();
            let parsed = parse_autocad_code(&code, &refs, None);
            assert_eq!(parsed, item, "code={code}");
        }
    }

    #[test]
    fn evaluates_filename_custom_property_sheet_number_and_object_property() {
        let mut custom = BTreeMap::new();
        custom.insert("Project".into(), "Demo Plant".into());
        let objects = vec![
            Object::Line(Line {
                from: Point3::new(0.0, 0.0, 0.0),
                to: Point3::new(30.0, 40.0, 0.0),
                layer: "pipes".into(),
                properties: Properties::default(),
            }),
            Object::Circle(Circle {
                center: Point3::new(0.0, 0.0, 0.0),
                radius: 10.0,
                layer: "0".into(),
                properties: Properties::default(),
            }),
        ];
        let ids = vec!["pipe-1".to_owned(), "tank-1".to_owned()];
        let ctx = EvalContext {
            sheet_name: Some("DEMO-001"),
            layout_name: Some("SH-01"),
            custom_properties: &custom,
            objects: &objects,
            object_ids: &ids,
            object_handles: &[],
        };
        let tf = TextField {
            template: "{0} | {1} | {2} | L={3} | R={4}".into(),
            items: vec![
                FieldItem {
                    kind: FieldKind::Filename,
                    property: None,
                    object: None,
                    code: None,
                    objects: Vec::new(),
                    format: Some("%fn6".into()),
                    value: None,
                },
                FieldItem {
                    kind: FieldKind::SheetNumber,
                    property: None,
                    object: None,
                    code: None,
                    objects: Vec::new(),
                    format: None,
                    value: None,
                },
                FieldItem {
                    kind: FieldKind::CustomProperty,
                    property: Some("project".into()),
                    object: None,
                    code: None,
                    objects: Vec::new(),
                    format: Some("%tc1".into()),
                    value: None,
                },
                FieldItem {
                    kind: FieldKind::ObjectProperty,
                    property: Some("Length".into()),
                    object: Some("pipe-1".into()),
                    code: None,
                    objects: Vec::new(),
                    format: Some("%lu2%pr1".into()),
                    value: None,
                },
                FieldItem {
                    kind: FieldKind::ObjectProperty,
                    property: Some("Radius".into()),
                    object: Some("tank-1".into()),
                    code: None,
                    objects: Vec::new(),
                    format: Some("%lu2%pr0".into()),
                    value: None,
                },
            ],
        };
        let (_, display) = evaluate_text_field(&tf, &ctx);
        assert_eq!(display, "DEMO-001.dwg | SH-01 | DEMO PLANT | L=50.0 | R=10");
    }
}
