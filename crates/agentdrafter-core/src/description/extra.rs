//! Sheet-description readers and JSON Schemas for points (`PDMODE`, `PDSIZE`,
//! `point`), construction lines (`xline`), rays (`ray`), multilines and their
//! styles (`mline_styles`, `mline`), geometric tolerances (`tolerance`), centre
//! marks (`center_mark`), centre lines (`center_line`), and SHX shapes (`shape`).

use serde_json::{json, Value};

use super::layers::{named_entries, object_color, LONGEST_LAYER_DESCRIPTION, PROPERTY_KEYS};
use super::schema::{object_def, optional_text, NUMBER, POSITIVE_NUMBER, TEXT_TYPES};
use super::strict_yaml::{index_path, key_path, Node};
use super::{
    angle, at, bad_value, boolean, length, plain_number, point, points, scalar, sequence,
    wrong_type, Fields,
};
use crate::error::{Error, ErrorCode};
use crate::model::{
    is_valid_pdmode, normalize_point3, CenterLine, CenterMark, DimensionStyle, Linetype, MLine,
    MLineElement, MLineJustification, MLineStyle, ObjectColor, ObjectLinetype, Point, Point3, Ray,
    ShxShape, Tolerance, XLine, BY_BLOCK, BY_LAYER, CONTINUOUS, STANDARD_DIMENSION_STYLE,
    STANDARD_MLINE_STYLE,
};
use crate::units::Unit;

pub(crate) const PDMODE_KEY: &str = "pdmode";
pub(crate) const PDSIZE_KEY: &str = "pdsize";
pub(crate) const MLINE_STYLES_KEY: &str = "mline_styles";

pub(crate) const MLINE_STYLE_KEYS: [&str; 12] = [
    "description",
    "elements",
    "fill_color",
    "display_joints",
    "start_angle",
    "end_angle",
    "start_square_cap",
    "start_round_cap",
    "start_inner_arcs_cap",
    "end_square_cap",
    "end_round_cap",
    "end_inner_arcs_cap",
];

const MLINE_ELEMENT_KEYS: [&str; 3] = ["offset", "color", "linetype"];

pub(super) const POINT_KEYS: [&str; 9] = [
    "position",
    "angle",
    "layer",
    PROPERTY_KEYS[0],
    PROPERTY_KEYS[1],
    PROPERTY_KEYS[2],
    PROPERTY_KEYS[3],
    PROPERTY_KEYS[4],
    PROPERTY_KEYS[5],
];

pub(super) const XLINE_KEYS: [&str; 11] = [
    "through",
    "direction",
    "angle",
    "second_point",
    "layer",
    PROPERTY_KEYS[0],
    PROPERTY_KEYS[1],
    PROPERTY_KEYS[2],
    PROPERTY_KEYS[3],
    PROPERTY_KEYS[4],
    PROPERTY_KEYS[5],
];

pub(super) const RAY_KEYS: [&str; 11] = [
    "from",
    "direction",
    "angle",
    "through",
    "layer",
    PROPERTY_KEYS[0],
    PROPERTY_KEYS[1],
    PROPERTY_KEYS[2],
    PROPERTY_KEYS[3],
    PROPERTY_KEYS[4],
    PROPERTY_KEYS[5],
];

pub(super) const MLINE_KEYS: [&str; 14] = [
    "points",
    "style",
    "scale",
    "justification",
    "closed",
    "suppress_start_caps",
    "suppress_end_caps",
    "layer",
    PROPERTY_KEYS[0],
    PROPERTY_KEYS[1],
    PROPERTY_KEYS[2],
    PROPERTY_KEYS[3],
    PROPERTY_KEYS[4],
    PROPERTY_KEYS[5],
];

pub(super) const TOLERANCE_KEYS: [&str; 12] = [
    "position",
    "text",
    "direction",
    "dimension_style",
    "text_height",
    "layer",
    PROPERTY_KEYS[0],
    PROPERTY_KEYS[1],
    PROPERTY_KEYS[2],
    PROPERTY_KEYS[3],
    PROPERTY_KEYS[4],
    PROPERTY_KEYS[5],
];

pub(super) const CENTER_MARK_KEYS: [&str; 14] = [
    "center",
    "radius",
    "cross_size",
    "cross_gap",
    "extension",
    "show_extensions",
    "rotation",
    "layer",
    PROPERTY_KEYS[0],
    PROPERTY_KEYS[1],
    PROPERTY_KEYS[2],
    PROPERTY_KEYS[3],
    PROPERTY_KEYS[4],
    PROPERTY_KEYS[5],
];

pub(super) const CENTER_LINE_KEYS: [&str; 11] = [
    "from",
    "to",
    "start_extension",
    "end_extension",
    "layer",
    PROPERTY_KEYS[0],
    PROPERTY_KEYS[1],
    PROPERTY_KEYS[2],
    PROPERTY_KEYS[3],
    PROPERTY_KEYS[4],
    PROPERTY_KEYS[5],
];

pub(super) const SHAPE_KEYS: [&str; 15] = [
    "file",
    "shape",
    "number",
    "position",
    "size",
    "rotation",
    "width_factor",
    "oblique",
    "layer",
    PROPERTY_KEYS[0],
    PROPERTY_KEYS[1],
    PROPERTY_KEYS[2],
    PROPERTY_KEYS[3],
    PROPERTY_KEYS[4],
    PROPERTY_KEYS[5],
];

/// Reads top-level `pdmode`.
pub(super) fn read_pdmode(node: &Node, path: &str) -> Result<i16, Error> {
    let text = scalar(node, path, "a point display mode integer")?;
    let val: i16 = text.trim().parse().map_err(|_| {
        bad_value(
            node,
            path,
            format!("`{text}` is not a valid PDMODE integer (0..4, 32..36, 64..68, or 96..100)"),
        )
    })?;
    if !is_valid_pdmode(val) {
        return Err(bad_value(
            node,
            path,
            format!(
                "`{val}` is not a valid PDMODE value; expected 0..4, 32..36, 64..68, or 96..100"
            ),
        ));
    }
    Ok(val)
}

/// Reads top-level `pdsize` (positive length, 0, or negative percentage).
pub(super) fn read_pdsize(node: &Node, path: &str, units: Unit) -> Result<f64, Error> {
    let text = scalar(node, path, "a point display size")?;
    if let Ok(n) = text.trim().parse::<f64>() {
        if n.is_finite() {
            return Ok(n);
        }
    }
    length(node, path, units)
}

/// Reads top-level `mline_styles` mapping.
pub(super) fn read_mline_styles(
    node: &Node,
    units: Unit,
    linetypes: &[Linetype],
) -> Result<Vec<MLineStyle>, Error> {
    let entries = named_entries(node, MLINE_STYLES_KEY, "multiline style")?;
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let path = key_path(MLINE_STYLES_KEY, &entry.key);
        out.push(read_mline_style(
            &entry.key,
            &entry.value,
            &path,
            units,
            linetypes,
        )?);
    }
    Ok(out)
}

fn read_mline_style(
    name: &str,
    node: &Node,
    path: &str,
    units: Unit,
    linetypes: &[Linetype],
) -> Result<MLineStyle, Error> {
    let fields = Fields::of(node, path, &MLINE_STYLE_KEYS, "a multiline style")?;
    let description = match fields.text("description") {
        Some(n) => {
            let dp = fields.path_of("description");
            let text = scalar(n, &dp, "a description, one line of text")?;
            if text.chars().any(char::is_control) {
                return Err(bad_value(
                    n,
                    &dp,
                    "a multiline style's description cannot contain control characters",
                ));
            }
            if text.chars().count() > LONGEST_LAYER_DESCRIPTION {
                return Err(bad_value(
                    n,
                    &dp,
                    format!(
                        "a multiline style's description is at most {LONGEST_LAYER_DESCRIPTION} characters"
                    ),
                ));
            }
            text.to_owned()
        }
        None => String::new(),
    };
    let elements_node = fields.require("elements")?;
    let elements_path = fields.path_of("elements");
    let items = sequence(
        elements_node,
        &elements_path,
        "a list of multiline elements",
    )?;
    if items.is_empty() {
        return Err(bad_value(
            elements_node,
            &elements_path,
            "a multiline style needs at least one element",
        ));
    }
    let mut elements = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let ip = index_path(&elements_path, i);
        let ef = Fields::of(item, &ip, &MLINE_ELEMENT_KEYS, "a multiline element")?;
        let offset = length(ef.require("offset")?, &ef.path_of("offset"), units)?;
        let color = match ef.get("color") {
            Some(c) => object_color(c, &ef.path_of("color"))?,
            None => ObjectColor::ByLayer,
        };
        let linetype = match ef.text("linetype") {
            Some(lt) => read_element_linetype(lt, &ef.path_of("linetype"), linetypes)?,
            None => ObjectLinetype::ByLayer,
        };
        elements.push(MLineElement {
            offset,
            color,
            linetype,
        });
    }
    let fill_color = match fields.get("fill_color") {
        Some(n) => Some(object_color(n, &fields.path_of("fill_color"))?),
        None => None,
    };
    let display_joints = match fields.get("display_joints") {
        Some(n) => boolean(n, &fields.path_of("display_joints"))?,
        None => false,
    };
    let start_angle = match fields.get("start_angle") {
        Some(n) => cap_angle(n, &fields.path_of("start_angle"))?,
        None => 90.0,
    };
    let end_angle = match fields.get("end_angle") {
        Some(n) => cap_angle(n, &fields.path_of("end_angle"))?,
        None => 90.0,
    };
    let start_square_cap = match fields.get("start_square_cap") {
        Some(n) => boolean(n, &fields.path_of("start_square_cap"))?,
        None => false,
    };
    let start_round_cap = match fields.get("start_round_cap") {
        Some(n) => boolean(n, &fields.path_of("start_round_cap"))?,
        None => false,
    };
    let start_inner_arcs_cap = match fields.get("start_inner_arcs_cap") {
        Some(n) => boolean(n, &fields.path_of("start_inner_arcs_cap"))?,
        None => false,
    };
    let end_square_cap = match fields.get("end_square_cap") {
        Some(n) => boolean(n, &fields.path_of("end_square_cap"))?,
        None => false,
    };
    let end_round_cap = match fields.get("end_round_cap") {
        Some(n) => boolean(n, &fields.path_of("end_round_cap"))?,
        None => false,
    };
    let end_inner_arcs_cap = match fields.get("end_inner_arcs_cap") {
        Some(n) => boolean(n, &fields.path_of("end_inner_arcs_cap"))?,
        None => false,
    };
    Ok(MLineStyle {
        name: name.to_owned(),
        description,
        elements,
        fill_color,
        display_joints,
        start_angle,
        end_angle,
        start_square_cap,
        start_round_cap,
        start_inner_arcs_cap,
        end_square_cap,
        end_round_cap,
        end_inner_arcs_cap,
    })
}

fn read_element_linetype(
    node: &Node,
    path: &str,
    linetypes: &[Linetype],
) -> Result<ObjectLinetype, Error> {
    let text = scalar(node, path, "a linetype name")?;
    if text == BY_LAYER {
        return Ok(ObjectLinetype::ByLayer);
    }
    if text == BY_BLOCK {
        return Ok(ObjectLinetype::ByBlock);
    }
    if text == CONTINUOUS || linetypes.iter().any(|l| l.name == text) {
        return Ok(ObjectLinetype::Named(text.to_owned()));
    }
    Err(bad_value(
        node,
        path,
        format!("unknown linetype `{text}`; expected `{BY_LAYER}`, `{BY_BLOCK}`, `{CONTINUOUS}`, or a defined linetype"),
    ))
}

fn cap_angle(node: &Node, path: &str) -> Result<f64, Error> {
    let a = plain_number(node, path, "an angle in degrees (10 to 170)")?;
    if !(10.0..=170.0).contains(&a) {
        return Err(bad_value(
            node,
            path,
            format!("multiline cap angle `{a}` must be between 10 and 170 degrees"),
        ));
    }
    Ok(a)
}

pub(super) fn read_point(fields: &Fields, units: Unit, layer: String) -> Result<Point, Error> {
    let position = point(
        fields.require("position")?,
        &fields.path_of("position"),
        units,
    )?;
    let angle_deg = match fields.get("angle") {
        Some(n) => angle(n, &fields.path_of("angle"))?,
        None => 0.0,
    };
    Ok(Point {
        position,
        angle: angle_deg,
        layer,
        properties: Default::default(),
    })
}

fn read_direction(
    fields: &Fields,
    base: Point3,
    second_key: &str,
    units: Unit,
) -> Result<Point3, Error> {
    let dir_node = fields.get("direction");
    let ang_node = fields.get("angle");
    let second_node = fields.get(second_key);
    let count = usize::from(dir_node.is_some())
        + usize::from(ang_node.is_some())
        + usize::from(second_node.is_some());
    if count == 0 {
        return Err(at(
            Error::new(
                ErrorCode::MissingKey,
                format!("needs `direction`, `angle`, or `{second_key}`"),
            ),
            fields.path,
            fields.at,
        ));
    }
    if count > 1 {
        let offender = second_node.or(ang_node).or(dir_node).expect("count > 1");
        return Err(bad_value(
            offender,
            fields.path,
            format!("give only one of `direction`, `angle`, or `{second_key}`"),
        ));
    }
    if let Some(d) = dir_node {
        let dp = fields.path_of("direction");
        let v = point(d, &dp, units)?;
        let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
        if len <= 1e-12 {
            return Err(bad_value(d, &dp, "direction cannot be zero"));
        }
        return Ok(normalize_point3(v));
    }
    if let Some(a) = ang_node {
        let deg = angle(a, &fields.path_of("angle"))?;
        let (sin, cos) = super::sin_cos_degrees(deg);
        return Ok(Point3::new(cos, sin, 0.0));
    }
    let s = second_node.expect("checked count == 1");
    let sp = fields.path_of(second_key);
    let p2 = point(s, &sp, units)?;
    let v = Point3::new(p2.x - base.x, p2.y - base.y, p2.z - base.z);
    let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if len <= 1e-12 {
        return Err(bad_value(
            s,
            &sp,
            format!("`{second_key}` cannot coincide with the base point"),
        ));
    }
    Ok(normalize_point3(v))
}

pub(super) fn read_xline(fields: &Fields, units: Unit, layer: String) -> Result<XLine, Error> {
    let through = point(
        fields.require("through")?,
        &fields.path_of("through"),
        units,
    )?;
    let direction = read_direction(fields, through, "second_point", units)?;
    Ok(XLine {
        through,
        direction,
        layer,
        properties: Default::default(),
    })
}

pub(super) fn read_ray(fields: &Fields, units: Unit, layer: String) -> Result<Ray, Error> {
    let from = point(fields.require("from")?, &fields.path_of("from"), units)?;
    let direction = read_direction(fields, from, "through", units)?;
    Ok(Ray {
        from,
        direction,
        layer,
        properties: Default::default(),
    })
}

pub(super) fn read_mline(
    fields: &Fields,
    units: Unit,
    layer: String,
    mline_styles: &[MLineStyle],
) -> Result<MLine, Error> {
    let pts_node = fields.require("points")?;
    let pts_path = fields.path_of("points");
    let pts = points(pts_node, &pts_path, units, 2)?;
    let style = match fields.text("style") {
        Some(n) => {
            let sp = fields.path_of("style");
            let name = scalar(n, &sp, "a multiline style name")?.to_owned();
            let exists = name.eq_ignore_ascii_case(STANDARD_MLINE_STYLE)
                || mline_styles
                    .iter()
                    .any(|s| s.name.eq_ignore_ascii_case(&name));
            if !exists {
                let mut choices: Vec<&str> = vec![STANDARD_MLINE_STYLE];
                for s in mline_styles {
                    if !s.name.eq_ignore_ascii_case(STANDARD_MLINE_STYLE) {
                        choices.push(&s.name);
                    }
                }
                return Err(bad_value(
                    n,
                    &sp,
                    format!(
                        "unknown multiline style `{name}`; defined styles: {}",
                        choices.join(", ")
                    ),
                ));
            }
            name
        }
        None => STANDARD_MLINE_STYLE.to_owned(),
    };
    let scale = match fields.get("scale") {
        Some(n) => {
            let sp = fields.path_of("scale");
            let s = plain_number(n, &sp, "a non-zero scale")?;
            if s.abs() <= 1e-12 {
                return Err(bad_value(n, &sp, "multiline scale cannot be zero"));
            }
            s
        }
        None => 1.0,
    };
    let justification = match fields.get("justification") {
        Some(n) => {
            let jp = fields.path_of("justification");
            match scalar(n, &jp, "top, zero, or bottom")? {
                "top" => MLineJustification::Top,
                "zero" => MLineJustification::Zero,
                "bottom" => MLineJustification::Bottom,
                other => {
                    return Err(bad_value(
                        n,
                        &jp,
                        format!(
                        "unknown multiline justification `{other}`; expected top, zero, or bottom"
                    ),
                    ))
                }
            }
        }
        None => MLineJustification::Zero,
    };
    let closed = match fields.get("closed") {
        Some(n) => boolean(n, &fields.path_of("closed"))?,
        None => false,
    };
    let suppress_start_caps = match fields.get("suppress_start_caps") {
        Some(n) => boolean(n, &fields.path_of("suppress_start_caps"))?,
        None => false,
    };
    let suppress_end_caps = match fields.get("suppress_end_caps") {
        Some(n) => boolean(n, &fields.path_of("suppress_end_caps"))?,
        None => false,
    };
    Ok(MLine {
        points: pts,
        style,
        scale,
        justification,
        closed,
        suppress_start_caps,
        suppress_end_caps,
        layer,
        properties: Default::default(),
    })
}

pub(super) fn read_tolerance(
    fields: &Fields,
    units: Unit,
    layer: String,
    dimension_styles: &[DimensionStyle],
) -> Result<Tolerance, Error> {
    let position = point(
        fields.require("position")?,
        &fields.path_of("position"),
        units,
    )?;
    let text = scalar(
        fields.require("text")?,
        &fields.path_of("text"),
        "tolerance frame text",
    )?
    .to_owned();
    let direction = match fields.get("direction") {
        Some(n) => angle(n, &fields.path_of("direction"))?,
        None => 0.0,
    };
    let dimension_style = match fields.text("dimension_style") {
        Some(n) => {
            let dp = fields.path_of("dimension_style");
            let name = scalar(n, &dp, "a dimension style name")?.to_owned();
            let exists = DimensionStyle::built_in(&name, units).is_some()
                || dimension_styles.iter().any(|s| s.name == name);
            if !exists {
                return Err(bad_value(
                    n,
                    &dp,
                    format!("unknown dimension style `{name}`"),
                ));
            }
            name
        }
        None => STANDARD_DIMENSION_STYLE.to_owned(),
    };
    let text_height = match fields.get("text_height") {
        Some(n) => {
            let hp = fields.path_of("text_height");
            let h = length(n, &hp, units)?;
            if h <= 0.0 {
                return Err(bad_value(n, &hp, "text_height must be positive"));
            }
            Some(h)
        }
        None => None,
    };
    Ok(Tolerance {
        position,
        direction,
        dimension_style,
        text,
        text_height,
        layer,
        properties: Default::default(),
    })
}

pub(super) fn read_center_mark(
    fields: &Fields,
    units: Unit,
    layer: String,
) -> Result<CenterMark, Error> {
    let center = point(fields.require("center")?, &fields.path_of("center"), units)?;
    let rad_node = fields.require("radius")?;
    let rad_path = fields.path_of("radius");
    let radius = length(rad_node, &rad_path, units)?;
    if radius <= 0.0 {
        return Err(bad_value(rad_node, &rad_path, "radius must be positive"));
    }
    let cross_size = match fields.get("cross_size") {
        Some(n) => {
            let p = fields.path_of("cross_size");
            let v = length(n, &p, units)?;
            if v <= 0.0 {
                return Err(bad_value(n, &p, "cross_size must be positive"));
            }
            Some(v)
        }
        None => None,
    };
    let cross_gap = match fields.get("cross_gap") {
        Some(n) => {
            let p = fields.path_of("cross_gap");
            let v = length(n, &p, units)?;
            if v < 0.0 {
                return Err(bad_value(n, &p, "cross_gap cannot be negative"));
            }
            Some(v)
        }
        None => None,
    };
    let extension = match fields.get("extension") {
        Some(n) => {
            let p = fields.path_of("extension");
            let v = length(n, &p, units)?;
            if v < 0.0 {
                return Err(bad_value(n, &p, "extension cannot be negative"));
            }
            Some(v)
        }
        None => None,
    };
    let show_extensions = match fields.get("show_extensions") {
        Some(n) => boolean(n, &fields.path_of("show_extensions"))?,
        None => true,
    };
    let rotation = match fields.get("rotation") {
        Some(n) => angle(n, &fields.path_of("rotation"))?,
        None => 0.0,
    };
    Ok(CenterMark {
        center,
        radius,
        cross_size,
        cross_gap,
        extension,
        show_extensions,
        rotation,
        layer,
        properties: Default::default(),
    })
}

pub(super) fn read_center_line(
    fields: &Fields,
    units: Unit,
    layer: String,
) -> Result<CenterLine, Error> {
    let from = point(fields.require("from")?, &fields.path_of("from"), units)?;
    let to = point(fields.require("to")?, &fields.path_of("to"), units)?;
    let start_extension = match fields.get("start_extension") {
        Some(n) => length(n, &fields.path_of("start_extension"), units)?,
        None => 0.0,
    };
    let end_extension = match fields.get("end_extension") {
        Some(n) => length(n, &fields.path_of("end_extension"), units)?,
        None => 0.0,
    };
    Ok(CenterLine {
        from,
        to,
        start_extension,
        end_extension,
        layer,
        properties: Default::default(),
    })
}

pub(super) fn read_shape(fields: &Fields, units: Unit, layer: String) -> Result<ShxShape, Error> {
    let file_node = fields.require("file")?;
    let fp = fields.path_of("file");
    let file = scalar(file_node, &fp, "an SHX shape file name")?
        .trim()
        .to_owned();
    if file.is_empty() {
        return Err(bad_value(file_node, &fp, "shape `file` cannot be empty"));
    }
    let name = match fields.text("shape") {
        Some(n) => scalar(n, &fields.path_of("shape"), "a shape name")?
            .trim()
            .to_owned(),
        None => String::new(),
    };
    let number = match fields.get("number") {
        Some(n) => {
            let np = fields.path_of("number");
            let text = scalar(n, &np, "a positive shape number")?;
            let num: i32 = text
                .trim()
                .parse()
                .map_err(|_| bad_value(n, &np, format!("`{text}` is not a shape number")))?;
            if num <= 0 || num > i32::from(u16::MAX) {
                return Err(bad_value(
                    n,
                    &np,
                    format!("shape number `{num}` must be between 1 and 65535"),
                ));
            }
            Some(num)
        }
        None => None,
    };
    if name.is_empty() && number.is_none() {
        return Err(at(
            Error::new(
                ErrorCode::MissingKey,
                "a shape needs `shape`, `number`, or both",
            ),
            fields.path,
            fields.at,
        ));
    }
    let position = point(
        fields.require("position")?,
        &fields.path_of("position"),
        units,
    )?;
    let size_node = fields.require("size")?;
    let sp = fields.path_of("size");
    let size = length(size_node, &sp, units)?;
    if size <= 0.0 {
        return Err(bad_value(size_node, &sp, "shape `size` must be positive"));
    }
    let rotation = match fields.get("rotation") {
        Some(n) => angle(n, &fields.path_of("rotation"))?,
        None => 0.0,
    };
    let width_factor = match fields.get("width_factor") {
        Some(n) => {
            let wp = fields.path_of("width_factor");
            let w = plain_number(n, &wp, "a positive width factor")?;
            if w <= 0.0 {
                return Err(bad_value(n, &wp, "width_factor must be positive"));
            }
            w
        }
        None => 1.0,
    };
    let oblique = match fields.get("oblique") {
        Some(n) => angle(n, &fields.path_of("oblique"))?,
        None => 0.0,
    };
    Ok(ShxShape {
        file,
        name,
        number,
        position,
        size,
        rotation,
        width_factor,
        oblique,
        layer,
        properties: Default::default(),
    })
}

#[allow(dead_code)]
fn _keep_wrong_type() {
    let _ = wrong_type;
}

// ── JSON Schema definitions ────────────────────────────────────────────────

fn bool_schema() -> Value {
    json!({
        "anyOf": [
            { "type": "boolean" },
            { "enum": ["true", "false"] }
        ]
    })
}

fn plain_number_schema() -> Value {
    json!({
        "anyOf": [
            { "type": "number" },
            { "type": "string", "pattern": format!("^{NUMBER}$") }
        ]
    })
}

pub(super) fn pdmode_property() -> Value {
    json!({
        "description": "AutoCAD PDMODE point display mode: 0 (dot), 1 (none), 2 (+), 3 (x), 4 (|), optionally combined with +32 (circle), +64 (square), or +96 (circle and square).",
        "anyOf": [
            { "type": "integer", "enum": [0, 1, 2, 3, 4, 32, 33, 34, 35, 36, 64, 65, 66, 67, 68, 96, 97, 98, 99, 100] },
            { "type": "string", "enum": ["0","1","2","3","4","32","33","34","35","36","64","65","66","67","68","96","97","98","99","100"] }
        ]
    })
}

pub(super) fn pdsize_property() -> Value {
    json!({
        "description": "AutoCAD PDSIZE point display size: positive for absolute drawing units, 0 for 5% of viewport height, negative for percentage of viewport height.",
        "$ref": "#/$defs/length"
    })
}

pub(super) fn mline_styles_property() -> Value {
    json!({
        "description": "Multiline styles (MLSTYLE) defined on this sheet, by name. `Standard` (+0.5 / -0.5) is built in.",
        "type": "object",
        "propertyNames": { "$ref": "#/$defs/name" },
        "additionalProperties": { "$ref": "#/$defs/mline_style" }
    })
}

pub(super) fn mline_style_def() -> Value {
    json!({
        "description": "A multiline style (MLSTYLE): parallel elements, caps, joints and optional background fill.",
        "type": "object",
        "required": ["elements"],
        "additionalProperties": false,
        "properties": {
            "description": optional_text(json!({ "type": TEXT_TYPES, "maxLength": LONGEST_LAYER_DESCRIPTION })),
            "elements": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "required": ["offset"],
                    "additionalProperties": false,
                    "properties": {
                        "offset": { "$ref": "#/$defs/length" },
                        "color": { "$ref": "#/$defs/object_color" },
                        "linetype": { "$ref": "#/$defs/object_linetype" }
                    }
                }
            },
            "fill_color": { "$ref": "#/$defs/object_color" },
            "display_joints": bool_schema(),
            "start_angle": plain_number_schema(),
            "end_angle": plain_number_schema(),
            "start_square_cap": bool_schema(),
            "start_round_cap": bool_schema(),
            "start_inner_arcs_cap": bool_schema(),
            "end_square_cap": bool_schema(),
            "end_round_cap": bool_schema(),
            "end_inner_arcs_cap": bool_schema()
        }
    })
}

pub(super) fn add_object_defs(defs: &mut serde_json::Map<String, Value>) {
    defs.insert("mline_style".into(), mline_style_def());
    defs.insert(
        "point_object".into(),
        object_def(
            "A point entity (POINT), drawn with the sheet's PDMODE and PDSIZE.",
            &["position"],
            json!({
                "position": { "$ref": "#/$defs/point" },
                "angle": { "$ref": "#/$defs/angle" },
            }),
        ),
    );
    defs.insert(
        "xline".into(),
        object_def(
            "An infinite construction line (XLINE) through a base point along a direction, angle, or second point.",
            &["through"],
            json!({
                "through": { "$ref": "#/$defs/point" },
                "direction": { "$ref": "#/$defs/point" },
                "angle": { "$ref": "#/$defs/angle" },
                "second_point": { "$ref": "#/$defs/point" },
            }),
        ),
    );
    defs.insert(
        "ray".into(),
        object_def(
            "A semi-infinite ray (RAY) starting at `from` along a direction, angle, or through point.",
            &["from"],
            json!({
                "from": { "$ref": "#/$defs/point" },
                "direction": { "$ref": "#/$defs/point" },
                "angle": { "$ref": "#/$defs/angle" },
                "through": { "$ref": "#/$defs/point" },
            }),
        ),
    );
    defs.insert(
        "mline".into(),
        object_def(
            "A multiline (MLINE) of parallel lines controlled by a multiline style.",
            &["points"],
            json!({
                "points": { "type": "array", "minItems": 2, "items": { "$ref": "#/$defs/point" } },
                "style": optional_text(json!({ "type": TEXT_TYPES })),
                "scale": plain_number_schema(),
                "justification": { "enum": ["top", "zero", "bottom"] },
                "closed": bool_schema(),
                "suppress_start_caps": bool_schema(),
                "suppress_end_caps": bool_schema(),
            }),
        ),
    );
    defs.insert(
        "tolerance".into(),
        object_def(
            "A geometric tolerance feature control frame (TOLERANCE).",
            &["position", "text"],
            json!({
                "position": { "$ref": "#/$defs/point" },
                "text": { "type": TEXT_TYPES },
                "direction": { "$ref": "#/$defs/angle" },
                "dimension_style": optional_text(json!({ "type": TEXT_TYPES })),
                "text_height": { "$ref": "#/$defs/length_more_than_0" },
            }),
        ),
    );
    defs.insert(
        "center_mark".into(),
        object_def(
            "A circular centre mark (CENTERMARK) with central crosshair and optional radial extension lines.",
            &["center", "radius"],
            json!({
                "center": { "$ref": "#/$defs/point" },
                "radius": { "$ref": "#/$defs/length_more_than_0" },
                "cross_size": { "$ref": "#/$defs/length_more_than_0" },
                "cross_gap": { "$ref": "#/$defs/length_not_negative" },
                "extension": { "$ref": "#/$defs/length_not_negative" },
                "show_extensions": bool_schema(),
                "rotation": { "$ref": "#/$defs/angle" },
            }),
        ),
    );
    defs.insert(
        "center_line".into(),
        object_def(
            "A straight centre line (CENTERLINE) with optional end extensions.",
            &["from", "to"],
            json!({
                "from": { "$ref": "#/$defs/point" },
                "to": { "$ref": "#/$defs/point" },
                "start_extension": { "$ref": "#/$defs/length" },
                "end_extension": { "$ref": "#/$defs/length" },
            }),
        ),
    );
    defs.insert(
        "shape".into(),
        object_def(
            "An SHX shape (SHAPE) placed from a compiled `.shx` shape file.",
            &["file", "position", "size"],
            json!({
                "file": { "type": TEXT_TYPES },
                "shape": optional_text(json!({ "type": TEXT_TYPES })),
                "number": {
                    "anyOf": [
                        { "type": "integer", "minimum": 1, "maximum": 65535 },
                        { "type": "string", "pattern": "^[1-9][0-9]*$" }
                    ]
                },
                "position": { "$ref": "#/$defs/point" },
                "size": { "$ref": "#/$defs/length_more_than_0" },
                "rotation": { "$ref": "#/$defs/angle" },
                "width_factor": {
                    "anyOf": [
                        { "type": "number", "exclusiveMinimum": 0 },
                        { "type": "string", "pattern": format!("^{POSITIVE_NUMBER}$") }
                    ]
                },
                "oblique": { "$ref": "#/$defs/angle" },
            }),
        ),
    );
}
