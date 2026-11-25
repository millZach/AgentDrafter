//! Reads a sheet's paper-space layout, which space each of its objects is
//! in, and its viewports, and states them in the schema.
//!
//! A sheet drawn in a layout says so under `layout`, with the layout's
//! name. Its paper is then a standard size by name, and its unit
//! millimetres or inches, as AutoCAD's paper is. Each object may say
//! `space: paper`; the others are in model space, at full size. A
//! viewport is always in paper space: a window on the paper, `width` by
//! `height` round its `center`, that shows model space round `view_center`
//! at its `scale`, such as `1:4`.

use serde_json::{json, Value};

use super::schema::{POSITIVE_NUMBER, TEXT_TYPES};
use super::strict_yaml::Node;
use super::{angle, at, bad_value, boolean, length, point, scalar, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{Clip, Layer, Paper, Point3, Viewport};
use crate::units::{parse_length, LengthError, Unit};

/// The key a sheet description or template declares its layout under.
pub(crate) const LAYOUT_KEY: &str = "layout";

/// The keys of a layout.
pub(crate) const LAYOUT_KEYS: [&str; 2] = ["name", super::page_setup::PAGE_SETUP_KEY];

/// The key a sheet description or template lists its layouts under, when
/// it is drawn in more than one.
pub(crate) const LAYOUTS_KEY: &str = "layouts";

/// The keys of each layout of a list.
pub(crate) const LIST_KEYS: [&str; 3] = ["name", "paper", super::page_setup::PAGE_SETUP_KEY];

/// The key an object of a sheet says which space it is in by.
pub(crate) const SPACE_KEY: &str = "space";

/// The space an object is in unless it says otherwise.
pub(crate) const MODEL: &str = "model";

/// The space a sheet's border, title block and viewports are in.
pub(crate) const PAPER: &str = "paper";

/// The keys a viewport takes.
pub(crate) const VIEWPORT_KEYS: [&str; 18] = [
    "center",
    "width",
    "height",
    "view_center",
    "scale",
    "rotation",
    "view_direction",
    super::annotation_scale::ANNOTATION_SCALE_KEY,
    "locked",
    "clip",
    "frozen_layers",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
    "plot_style",
    "transparency",
];

/// The keys a viewport's clipping boundary takes: a closed polyline's
/// points, or a circle's centre and radius.
pub(crate) const CLIP_KEYS: [&str; 3] = ["points", "center", "radius"];

/// The keys of a viewport's window, which a clipped viewport takes from
/// its boundary instead.
const WINDOW_KEYS: [&str; 3] = ["center", "width", "height"];

/// The longest name a layout can have, as AutoCAD's: 255 characters.
const LONGEST_NAME: usize = 255;

/// The characters AutoCAD refuses in a layout's name.
const NOT_IN_A_NAME: &str = "<>/\\\":;?*|,=`";

/// The name of the model-space tab, which no layout takes.
const MODEL_TAB: &str = "Model";

/// The name and optional page setup of the layout `node` declares, on a
/// sheet in `units` whose paper is `paper`, declared at `paper_node`: a
/// layout's paper is a standard size by name, and its unit millimetres or
/// inches.
pub(super) fn layout(
    node: &Node,
    path: &str,
    units: Unit,
    paper: Option<(&Paper, &Node)>,
) -> Result<(String, Option<crate::model::PageSetup>), Error> {
    let fields = Fields::of(node, path, &LAYOUT_KEYS, "a layout")?;
    let name_node = fields.require_text("name")?;
    let name_path = fields.path_of("name");
    let name = scalar(
        name_node,
        &name_path,
        "a layout's name, text such as `Panel`",
    )?;
    if let Some(problem) = name_problem(name) {
        return Err(bad_value(name_node, &name_path, problem));
    }
    if !matches!(units, Unit::Mm | Unit::In) {
        return Err(bad_value(
            node,
            path,
            format!(
                "a sheet drawn in a layout is in mm or in, as AutoCAD's paper is, and this one is \
                 in {units}"
            ),
        ));
    }
    let setup = match fields.get(super::page_setup::PAGE_SETUP_KEY) {
        Some(ps_node) => Some(super::page_setup::page_setup(
            ps_node,
            &fields.path_of(super::page_setup::PAGE_SETUP_KEY),
            units,
            true,
        )?),
        None => None,
    };
    match paper {
        None => Err(at(
            Error::new(
                ErrorCode::MissingKey,
                "a sheet drawn in a layout declares its paper under `paper`, a standard size by \
                 name such as `ISO A3 landscape`, which its page setup plots",
            ),
            path,
            node.at,
        )),
        Some((paper, paper_node)) if paper.name.is_none() => Err(bad_value(
            paper_node,
            super::paper::PAPER_KEY,
            "a sheet drawn in a layout declares a standard paper by name, such as `ISO A3 \
             landscape`, which AutoCAD's DWG To PDF plotter has, to plot whole",
        )),
        Some(_) => Ok((name.to_owned(), setup)),
    }
}

/// The layouts `node` lists, each with its name, paper and optional page
/// setup, on a sheet in `units` whose own paper, if it declares one, is
/// `paper`: two or more, each named once, whatever the case, as AutoCAD
/// names its tabs, and each on a standard paper by name, its own or the
/// sheet's.
pub(super) fn layouts(
    node: &Node,
    path: &str,
    units: Unit,
    paper: Option<&Paper>,
) -> Result<Vec<(String, Paper, Option<crate::model::PageSetup>)>, Error> {
    let items = super::sequence(node, path, "a list of layouts, each with its name")?;
    if items.len() < 2 {
        return Err(bad_value(
            node,
            path,
            format!(
                "`{LAYOUTS_KEY}` lists two or more layouts; a sheet drawn in one says \
                 `{LAYOUT_KEY}: {{name: ...}}`"
            ),
        ));
    }
    if !matches!(units, Unit::Mm | Unit::In) {
        return Err(bad_value(
            node,
            path,
            format!(
                "a sheet drawn in a layout is in mm or in, as AutoCAD's paper is, and this one is \
                 in {units}"
            ),
        ));
    }
    let mut out: Vec<(String, Paper, Option<crate::model::PageSetup>)> =
        Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at_path = super::strict_yaml::index_path(path, i);
        let fields = Fields::of(item, &at_path, &LIST_KEYS, "a layout")?;
        let name_node = fields.require_text("name")?;
        let name_path = fields.path_of("name");
        let name = scalar(
            name_node,
            &name_path,
            "a layout's name, text such as `Panel`",
        )?;
        if let Some(problem) = name_problem(name) {
            return Err(bad_value(name_node, &name_path, problem));
        }
        if let Some((other, _, _)) = out.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(name)) {
            return Err(at(
                Error::new(
                    ErrorCode::DuplicateName,
                    format!(
                        "two layouts are named `{other}` and `{name}`; AutoCAD names each tab \
                         once, whatever its case"
                    ),
                ),
                &name_path,
                name_node.at,
            ));
        }
        let own =
            match fields.get(super::paper::PAPER_KEY) {
                Some(paper_node) => {
                    let paper_path = fields.path_of(super::paper::PAPER_KEY);
                    let own = super::paper::paper(paper_node, &paper_path, units)?;
                    if own.name.is_none() {
                        return Err(bad_value(
                            paper_node,
                            &paper_path,
                            "a layout's paper is a standard paper by name, such as `ISO A3 \
                         landscape`, which AutoCAD's DWG To PDF plotter has, to plot whole",
                        ));
                    }
                    own
                }
                None => match paper {
                    Some(sheet) if sheet.name.is_some() => sheet.clone(),
                    Some(_) => return Err(bad_value(
                        item,
                        &at_path,
                        "a layout takes the sheet's paper when it gives none of its own, and the \
                         sheet's is not a standard paper by name; give this layout's under \
                         `paper`, such as `ISO A3 landscape`",
                    )),
                    None => {
                        return Err(at(
                            Error::new(
                                ErrorCode::MissingKey,
                                "each layout plots on a standard paper by name, such as `ISO A3 \
                             landscape`: its own under `paper`, or the sheet's",
                            ),
                            &at_path,
                            item.at,
                        ))
                    }
                },
            };
        let setup = match fields.get(super::page_setup::PAGE_SETUP_KEY) {
            Some(ps_node) => Some(super::page_setup::page_setup(
                ps_node,
                &fields.path_of(super::page_setup::PAGE_SETUP_KEY),
                units,
                true,
            )?),
            None => None,
        };
        out.push((name.to_owned(), own, setup));
    }
    Ok(out)
}

/// What is wrong with `name` as a layout's name, if anything.
pub(crate) fn name_problem(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("a layout's name is not empty".to_owned());
    }
    if name.chars().count() > LONGEST_NAME {
        return Some(format!(
            "a layout's name is at most {LONGEST_NAME} characters, as AutoCAD's is"
        ));
    }
    if let Some(c) = name.chars().find(|c| NOT_IN_A_NAME.contains(*c)) {
        return Some(format!(
            "a layout's name holds none of {NOT_IN_A_NAME}, as AutoCAD's does not; this one \
             holds `{c}`"
        ));
    }
    if name.eq_ignore_ascii_case(MODEL_TAB) {
        return Some(format!(
            "`{MODEL_TAB}` is AutoCAD's model-space tab, which no layout is named"
        ));
    }
    None
}

/// The layout whose paper space `space` names, among `layouts`, the names
/// of a sheet's layouts in order: a layout by its exact name, or, on a
/// sheet with one layout, `paper` for it. None when it names none of them.
pub(crate) fn layout_named(space: &str, layouts: &[String]) -> Option<usize> {
    layouts
        .iter()
        .position(|name| name == space)
        .or_else(|| (space == PAPER && layouts.len() == 1).then_some(0))
}

/// What `space` may be on a sheet whose layouts are `layouts`, for the
/// errors that refuse one.
pub(crate) fn spaces_allowed(layouts: &[String]) -> String {
    match layouts {
        [] => "`model`; a sheet without a layout has model space alone".to_owned(),
        [_] => "`model` or `paper`".to_owned(),
        _ => format!(
            "`model`, or the name of one of the sheet's layouts: {}",
            layouts
                .iter()
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The space the object whose keys are `fields` is in, on a sheet whose
/// layouts are named `layouts`, in order: the place of the layout whose
/// paper space holds it, or none for model space. A viewport is in a
/// layout's paper space, the only layout's unless it names another.
pub(super) fn space(
    fields: &Fields,
    layouts: &[String],
    viewport: bool,
) -> Result<Option<usize>, Error> {
    // A choice, as `justify` is: a bare null is no choice.
    let Some(node) = fields.get(SPACE_KEY) else {
        return match (viewport, layouts.len()) {
            (false, _) => Ok(None),
            (true, 0) => Err(no_layout(fields, "a viewport")),
            (true, 1) => Ok(Some(0)),
            (true, _) => Err(Error::new(
                ErrorCode::MissingKey,
                format!(
                    "a viewport on a sheet drawn in several layouts names the one it is in under \
                     `space`: {}",
                    spaces_allowed(layouts)
                ),
            )
            .at_field(fields.path.to_owned())
            .at_position(fields.at.line, fields.at.column)),
        };
    };
    let path = fields.path_of(SPACE_KEY);
    if node.is_bare_null() {
        return Err(bad_value(
            node,
            &path,
            format!(
                "a bare null is no space; {}, or leave `space` out for model space",
                spaces_allowed(layouts)
            ),
        ));
    }
    let named = scalar(
        node,
        &path,
        "a space, `model` or `paper`, or a layout's name",
    )?;
    if named == MODEL {
        if viewport {
            return Err(bad_value(
                node,
                &path,
                "a viewport is in paper space, a window on the paper onto model space",
            ));
        }
        return Ok(None);
    }
    if let Some(k) = layout_named(named, layouts) {
        return Ok(Some(k));
    }
    if layouts.is_empty() && named == PAPER {
        return Err(no_layout(fields, "an object in paper space"));
    }
    Err(at(
        Error::new(
            ErrorCode::BadValue,
            format!(
                "`{named}` is not a space of this sheet; {}",
                spaces_allowed(layouts)
            ),
        ),
        &path,
        node.at,
    ))
}

/// The error for `what` on a sheet with no layout.
fn no_layout(fields: &Fields, what: &str) -> Error {
    Error::new(
        ErrorCode::BadValue,
        format!(
            "{what} is on a sheet drawn in a layout; declare the sheet's layout under `layout`, \
             such as `layout: {{name: Panel}}`"
        ),
    )
    .at_field(fields.path.to_owned())
    .at_position(fields.at.line, fields.at.column)
}

/// A viewport, on its layer, on a sheet whose layers are `layers`.
pub(super) fn viewport(
    fields: &Fields,
    units: Unit,
    layer: String,
    layers: &[Layer],
    declared_scales: &[crate::model::AnnotationScale],
) -> Result<Viewport, Error> {
    let clip = match fields.get("clip") {
        Some(node) => Some(clip(node, &fields.path_of("clip"), units)?),
        None => None,
    };
    let (center, width, height) = match &clip {
        // A clipped viewport's window is the box round its boundary, as
        // AutoCAD's VPCLIP and MVIEW keep it.
        Some(clip) => {
            if let Some(key) = WINDOW_KEYS.iter().find(|k| fields.get(k).is_some()) {
                return Err(bad_value(
                    fields.get(key).expect("given"),
                    &fields.path_of(key),
                    "a clipped viewport's window is the box round its boundary, as AutoCAD keeps \
                     it, so it takes no `center`, `width` or `height`",
                ));
            }
            let [low, high] = clip.bounds();
            (
                Point3::new((low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 0.0),
                high[0] - low[0],
                high[1] - low[1],
            )
        }
        None => (
            flat_point(fields, "center", units)?,
            positive(fields, "width", units, "a viewport's width")?,
            positive(fields, "height", units, "a viewport's height")?,
        ),
    };
    let view_direction = match fields.get("view_direction") {
        Some(node) => parse_view_direction(node, &fields.path_of("view_direction"), units)?,
        None => None,
    };
    let view_center = if view_direction.is_some() {
        point(
            fields.require("view_center")?,
            &fields.path_of("view_center"),
            units,
        )?
    } else {
        flat_point(fields, "view_center", units)?
    };
    let scale_node = fields.require("scale")?;
    let scale_path = fields.path_of("scale");
    let scale = scale(scale_node, &scale_path)?;
    let rotation = match fields.get("rotation") {
        Some(node) => angle(node, &fields.path_of("rotation"))?,
        None => 0.0,
    };
    let annotation_scale = match fields.text(super::annotation_scale::ANNOTATION_SCALE_KEY) {
        Some(node) => Some(super::annotation_scale::resolve_scale_name(
            node,
            &fields.path_of(super::annotation_scale::ANNOTATION_SCALE_KEY),
            declared_scales,
        )?),
        None => None,
    };
    let locked = match fields.get("locked") {
        Some(node) => boolean(node, &fields.path_of("locked"))?,
        None => false,
    };
    let frozen_layers = match fields.get("frozen_layers") {
        Some(node) => frozen_layers(node, &fields.path_of("frozen_layers"), layers)?,
        None => Vec::new(),
    };
    let viewport = Viewport {
        center,
        width,
        height,
        view_center,
        scale,
        rotation,
        view_direction,
        annotation_scale,
        locked,
        clip,
        frozen_layers,
        layer,
        properties: Default::default(),
    };
    if let Some(problem) = viewport.scale_problem() {
        return Err(at(
            Error::new(ErrorCode::BadNumber, problem),
            &scale_path,
            scale_node.at,
        ));
    }
    Ok(viewport)
}

/// A viewport's optional 3D orthographic view direction: either a named
/// standard AutoCAD view (`top`, `front`, `se_isometric`, etc.) or a non-zero
/// 3D vector `[x, y, z]`. Top view (`[0, 0, +z]`) normalizes to `None`.
pub(super) fn parse_view_direction(
    node: &Node,
    path: &str,
    units: Unit,
) -> Result<Option<Point3>, Error> {
    if let super::strict_yaml::Kind::Scalar(text) = &node.kind {
        let Some(v) = crate::model::preset_view_direction(text) else {
            let names: Vec<&str> = crate::model::VIEW_DIRECTION_PRESETS
                .iter()
                .map(|(k, _)| *k)
                .collect();
            return Err(bad_value(
                node,
                path,
                format!(
                    "unknown `view_direction` `{text}`; expected a 3D vector `[x, y, z]` or one of {}",
                    names.join(", ")
                ),
            ));
        };
        if v.x == 0.0 && v.y == 0.0 && v.z > 0.0 {
            return Ok(None);
        }
        return Ok(Some(v));
    }
    let v = point(node, path, units)?;
    if v.x * v.x + v.y * v.y + v.z * v.z <= 1e-18 {
        return Err(bad_value(
            node,
            path,
            "`view_direction` cannot be [0, 0, 0]",
        ));
    }
    if v.x.abs() <= 1e-12 && v.y.abs() <= 1e-12 && v.z > 0.0 {
        return Ok(None);
    }
    Ok(Some(v))
}

/// A viewport's clipping boundary: `{points: [...]}`, a closed polyline's
/// points, or `{center: [x, y, 0], radius: r}`, a circle, on the paper.
pub(super) fn clip(node: &Node, path: &str, units: Unit) -> Result<Clip, Error> {
    let fields = Fields::of(
        node,
        path,
        &CLIP_KEYS,
        "a clipping boundary, `{points: [...]}` or `{center: [x, y, 0], radius: r}`",
    )?;
    let clip =
        match (fields.get("points"), fields.get("center")) {
            (Some(points), None) => {
                if let Some(radius) = fields.get("radius") {
                    return Err(bad_value(
                    radius,
                    &fields.path_of("radius"),
                    "a clipping boundary is a closed polyline's `points` or a circle's `center` \
                     and `radius`, not both",
                ));
                }
                let (points, bulges) =
                    super::polyline::clip_points(points, &fields.path_of("points"), units)?;
                Clip::Polyline { points, bulges }
            }
            (None, Some(_)) => {
                let center = flat_point(&fields, "center", units)?;
                let radius = positive(&fields, "radius", units, "a clipping circle's radius")?;
                Clip::Circle { center, radius }
            }
            (Some(_), Some(center)) => return Err(bad_value(
                center,
                &fields.path_of("center"),
                "a clipping boundary is a closed polyline's `points` or a circle's `center` and \
                 `radius`, not both",
            )),
            (None, None) => {
                return Err(at(
                    Error::new(
                        ErrorCode::MissingKey,
                        "a clipping boundary gives a closed polyline's `points`, or a circle's \
                     `center` and `radius`",
                    ),
                    path,
                    node.at,
                ))
            }
        };
    let [low, high] = clip.bounds();
    let finite = low.iter().chain(&high).all(|v| v.is_finite());
    if !(finite && high[0] > low[0] && high[1] > low[1]) {
        return Err(bad_value(
            node,
            path,
            "a clipping boundary encloses an area on the paper; this one is a line or a point",
        ));
    }
    Ok(clip)
}

/// The clipping boundary a clip_viewport sends, as JSON, read as a
/// viewport's `clip` is.
pub(crate) fn clip_from(value: &serde_json::Value, units: Unit) -> Result<Clip, Error> {
    clip(&super::edit::node_of(value), "clip", units)
}

/// The layers a viewport freezes, each named exactly as the sheet defines
/// it, or `0`, and each once, whatever the case, in the order of the
/// drawing's layer table: layer `0`, then the sheet's, as it lists them.
fn frozen_layers(node: &Node, path: &str, layers: &[Layer]) -> Result<Vec<String>, Error> {
    let items = super::sequence(node, path, "a list of layer names")?;
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at_path = super::strict_yaml::index_path(path, i);
        let name = scalar(item, &at_path, "a layer's name")?;
        let known = name == crate::model::LAYER_ZERO || layers.iter().any(|l| l.name == name);
        if !known {
            return Err(at(
                Error::new(
                    ErrorCode::UnknownName,
                    format!(
                        "layer `{name}` is not defined; a viewport freezes layer `0` or one under \
                         `layers`, named exactly as it is defined"
                    ),
                ),
                &at_path,
                item.at,
            ));
        }
        if out.iter().any(|l| l.eq_ignore_ascii_case(name)) {
            return Err(bad_value(
                item,
                &at_path,
                format!("layer `{name}` is listed twice; a viewport freezes each layer once"),
            ));
        }
        out.push(name.to_owned());
    }
    // In the order of the drawing's layer table, layer 0 first, as AutoCAD
    // 2026 kept a viewport's frozen layers when it saved them.
    out.sort_by_key(|name| {
        layers
            .iter()
            .position(|l| l.name == *name)
            .map_or(0, |i| i + 1)
    });
    Ok(out)
}

/// A point at `key` whose `z` is 0.
fn flat_point(fields: &Fields, key: &str, units: Unit) -> Result<Point3, Error> {
    let node = fields.require(key)?;
    let path = fields.path_of(key);
    let p = point(node, &path, units)?;
    if p.z != 0.0 {
        return Err(bad_value(
            node,
            &path,
            "a viewport lies on the paper and looks straight down on model space, so its points \
             have 0 for z",
        ));
    }
    Ok(p)
}

/// A length more than 0 at `key`.
fn positive(fields: &Fields, key: &str, units: Unit, what: &str) -> Result<f64, Error> {
    let node = fields.require(key)?;
    let path = fields.path_of(key);
    let value = length(node, &path, units)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(bad_value(node, &path, format!("{what} is more than 0")))
    }
}

/// What a scale is, for the errors that refuse one.
const SCALE_SHAPE: &str = "a scale, paper to model: a ratio such as `1:4` or `2:1`, or a number \
                           more than 0 such as `0.25`";

/// A viewport's scale: `p:q`, `p` units of paper to `q` of model, or a
/// number, the paper's length of one unit of model.
pub(super) fn scale(node: &Node, path: &str) -> Result<f64, Error> {
    let text = scalar(node, path, SCALE_SHAPE)?;
    let number = |part: &str| match parse_length(part.trim()) {
        Ok((value, None)) if value.is_finite() && value > 0.0 => Some(value),
        Ok(_) | Err(LengthError::NotANumber | LengthError::OutOfRange) => None,
        Err(LengthError::UnknownUnit(_)) => None,
    };
    let scale = match text.split_once(':') {
        Some((paper, model)) => number(paper).zip(number(model)).map(|(p, m)| p / m),
        None => number(text),
    };
    match scale {
        Some(scale) if scale.is_finite() && scale > 0.0 => Ok(scale),
        _ => Err(at(
            Error::new(
                ErrorCode::BadNumber,
                format!("expected {SCALE_SHAPE}, found `{text}`"),
            ),
            path,
            node.at,
        )),
    }
}

/// The schema of the `layout` key of a sheet description or template.
pub(crate) fn layout_property() -> Value {
    json!({
        "description": "The paper-space layout the sheet is drawn in, as AutoCAD's layout tab \
                        holds it. With it, the sheet's objects are in model space, at full size, \
                        unless they say `space: paper`; its border and title block go in paper \
                        space, at the paper's size, with viewports that show model space at a \
                        scale. The sheet's paper is then a standard size by name, which the \
                        layout's page setup plots whole at full size on AutoCAD's DWG To PDF \
                        plotter through monochrome.ctb, and its unit is mm or in. A build, a \
                        render, a plot and the checker draw the layout: paper space as it is \
                        drawn and model space through each viewport, cut at its edge. Without \
                        it, the sheet is drawn in model space alone.",
        "type": "object",
        "required": ["name"],
        "additionalProperties": false,
        "properties": {
            "name": name_schema(),
            "page_setup": { "$ref": "#/$defs/page_setup" },
        }
    })
}

/// The schema of a layout's name.
fn name_schema() -> Value {
    json!({
        "description": format!(
            "The layout's name, as AutoCAD's tab shows it: 1 to {LONGEST_NAME} characters, none \
             of {NOT_IN_A_NAME}, and not `{MODEL_TAB}`."
        ),
        // A plain number, true or false is read as its text, as every name
        // is.
        "type": TEXT_TYPES,
        "minLength": 1,
        "maxLength": LONGEST_NAME,
        "pattern": r#"^[^<>/\\":;?*|,=`]*[^<>/\\":;?*|,=`\s][^<>/\\":;?*|,=`]*$"#,
        "not": { "type": "string", "pattern": "^[Mm][Oo][Dd][Ee][Ll]$" },
    })
}

/// The schema of the `layouts` key of a sheet description or template.
pub(crate) fn layouts_property() -> Value {
    json!({
        "description": "The paper-space layouts the sheet is drawn in, when it is drawn in more \
                        than one, in the order of their tabs, as AutoCAD's layout tabs hold \
                        them: two or more, which share the sheet's model space. Each has its own \
                        name, given once whatever its case, and its own paper, a standard size \
                        by name, or the sheet's `paper` when it gives none; its paper-space \
                        objects and viewports say its name under `space`. A build writes each \
                        as its own AutoCAD layout, in this order, the first the one the DWG \
                        opens on; a render draws each to a PNG of its own, a plot each to a page \
                        of its own, and the checker checks each. Not with `layout`.",
        "type": "array",
        "minItems": 2,
        "items": {
            "type": "object",
            "required": ["name"],
            "additionalProperties": false,
            "properties": {
                "name": name_schema(),
                "paper": {
                    "description": "The paper the layout's page setup plots whole at full size, \
                                    a standard size by name such as `ISO A3 landscape`. Without \
                                    it, the sheet's `paper`.",
                    "type": "string",
                    "enum": super::paper::paper_names()
                        .into_iter()
                        .map(|(name, ..)| name)
                        .collect::<Vec<_>>(),
                },
                "page_setup": { "$ref": "#/$defs/page_setup" },
            },
        },
    })
}

/// The schema of the `space` key every object of a sheet takes.
pub(crate) fn space_property() -> Value {
    json!({
        "description": "The space the object is in, on a sheet drawn in a layout: `model`, at \
                        full size, where an object is unless it says otherwise, or a layout's \
                        paper space, at its paper's size, as a sheet's border and title block \
                        are: `paper` on a sheet drawn in one layout, and on one drawn in \
                        several, the name of the layout under `layouts`, exactly as it is \
                        given. A viewport is in paper space. A sheet without a layout has model \
                        space alone.",
        // A layout's name, which a plain number may be, as every name may.
        "type": TEXT_TYPES,
        "minLength": 1,
    })
}

/// The schema of the `space` key a viewport takes.
pub(crate) fn viewport_space_property() -> Value {
    json!({
        "description": "The layout whose paper space the viewport is in: `paper`, or leave it \
                        out, on a sheet drawn in one layout, and on one drawn in several, the \
                        name of the layout under `layouts`, which it then gives. Never `model`.",
        "type": TEXT_TYPES,
        "minLength": 1,
        "not": { "const": MODEL },
    })
}

/// The schema of a viewport's clipping boundary: a closed polyline's
/// points, or a circle.
pub(crate) fn clip_def() -> Value {
    json!({
        "description": "A viewport's clipping boundary, on the paper: a closed polyline's \
                        `points`, at least 3, each `[x, y, 0]` or a polyline point with a \
                        `bulge` or an `arc_angle` and no width, or a circle's `center` and \
                        `radius`. It encloses an area.",
        "oneOf": [
                {
                    "type": "object",
                    "required": ["points"],
                    "additionalProperties": false,
                    "properties": {
                        "points": {
                            "type": "array",
                            "minItems": 3,
                            "items": {
                                "anyOf": [
                                    { "$ref": "#/$defs/point" },
                                    {
                                        "$ref": "#/$defs/polyline_point",
                                        "not": {
                                            "anyOf": [
                                                { "required": ["start_width"] },
                                                { "required": ["end_width"] },
                                            ],
                                        },
                                    },
                                ],
                            },
                        },
                    },
                },
                {
                    "type": "object",
                    "required": ["center", "radius"],
                    "additionalProperties": false,
                    "properties": {
                        "center": { "$ref": "#/$defs/point" },
                        "radius": {
                            "description": "A length more than 0.",
                            "$ref": "#/$defs/length_more_than_0",
                        },
                    },
                },
            ],
    })
}

/// The schema of a viewport, as a sheet's objects list it.
pub(crate) fn viewport_def() -> Value {
    let ratio = format!(r"^\s*{POSITIVE_NUMBER}\s*(:\s*{POSITIVE_NUMBER}\s*)?$");
    let mut def = super::schema::object_def(
        "A window on the paper that shows model space at a scale, centred on a model point, as \
         AutoCAD's MVIEW makes one: rectangular, or clipped to a boundary, seen from above. It \
         is in the paper space of one of the sheet's layouts, and its frame draws there with \
         its own pen; model space draws inside it, cut at its edge, but for the layers frozen \
         in it. Put it on a layer that does not plot to leave its frame off a PDF.",
        &["view_center", "scale"],
        json!({
            "center": {
                "description": "The middle of the window, on the paper, with 0 for z. Not on a \
                                clipped viewport.",
                "$ref": "#/$defs/point",
            },
            "width": {
                "description": "How wide the window is on the paper, a length more than 0. Not \
                                on a clipped viewport.",
                "$ref": "#/$defs/length_more_than_0",
            },
            "height": {
                "description": "How tall the window is on the paper, a length more than 0. Not \
                                on a clipped viewport.",
                "$ref": "#/$defs/length_more_than_0",
            },
            "view_center": {
                "description": "The point of model space at the middle of the window, with 0 \
                                for z: AutoCAD's view centre.",
                "$ref": "#/$defs/point",
            },
            "scale": {
                "description": format!(
                    "{SCALE_SHAPE}, each side of a ratio a number more than 0. Quote a ratio, \
                     '1:4', since a YAML reader that guesses types may read 1:4 as a number. A \
                     length of model space draws this many times as long on the paper: 1:4 \
                     draws a quarter size. Text, hatch patterns and the geometry scale with it; \
                     lineweights do not, and a linetype's dashes are as long on the paper as in \
                     paper space, as AutoCAD's PSLTSCALE 1 draws them. The scale, one over it, \
                     and the viewport's height and width over it are each a finite number more \
                     than 0 that a DWG holds whole."
                ),
                "anyOf": [
                    { "type": "number", "exclusiveMinimum": 0 },
                    { "type": "string", "pattern": ratio },
                ],
            },
            "rotation": {
                "$ref": "#/$defs/angle",
                "description": "View twist angle in degrees anticlockwise (AutoCAD viewport \
                                twist angle). Without it, 0.",
            },
            "view_direction": {
                "description": "Optional 3D orthographic view direction (AutoCAD `VIEWDIR`), \
                                given as a non-zero 3D vector `[x, y, z]` or one of the standard \
                                view names (`top`, `bottom`, `front`, `back`, `left`, `right`, \
                                `sw_isometric`, `se_isometric`, `ne_isometric`, `nw_isometric`). \
                                Without it, top/plan view (`[0, 0, 1]`).",
                "anyOf": [
                    { "$ref": "#/$defs/point" },
                    {
                        "enum": crate::model::VIEW_DIRECTION_PRESETS
                            .iter()
                            .map(|(k, _)| *k)
                            .collect::<Vec<_>>(),
                    },
                ],
            },
            "annotation_scale": super::annotation_scale::viewport_annotation_scale_property(),
            "locked": {
                "description": "Whether the viewport's view is locked, as AutoCAD's display \
                                lock keeps it from being zoomed or panned: change_viewport \
                                refuses to change the scale or the view centre of a locked \
                                viewport unless it unlocks it too. It changes nothing drawn. \
                                Without it, false.",
                "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
            },
            "clip": {
                "description": "The boundary the viewport is clipped to instead of its \
                                rectangle, on the paper, as AutoCAD's VPCLIP and MVIEW's Object \
                                and Polygonal options clip one: a closed polyline's `points`, at \
                                least 3, each `[x, y, 0]` or a polyline point with a `bulge` or \
                                an `arc_angle` and no width, or a circle's `center` and \
                                `radius`. Model space shows inside it alone, its ink cut there, \
                                and it is the viewport's frame, which plots with the viewport's \
                                pen at its lineweight, as AutoCAD plots a clipping boundary. The \
                                viewport's window is the box round it, so a clipped viewport \
                                takes no `center`, `width` or `height`, and `view_center` is the \
                                model point at the middle of that box. The boundary encloses an \
                                area.",
                "$ref": "#/$defs/viewport_clip",
            },
            "frozen_layers": {
                "description": "The layers frozen in this viewport alone, as AutoCAD's VPLAYER \
                                freezes them: their objects neither draw nor plot through it, \
                                and the checker does not check them through it, but they draw \
                                through the sheet's other viewports. Each is layer `0` or one \
                                under `layers`, named exactly as it is defined, and listed once \
                                whatever its case.",
                "type": "array",
                "items": { "type": TEXT_TYPES },
            },
        }),
    );
    // A rectangular viewport gives its window; a clipped one takes it
    // from its boundary.
    def["oneOf"] = json!([
        { "required": ["center", "width", "height"], "not": { "required": ["clip"] } },
        {
            "required": ["clip"],
            "not": {
                "anyOf": [
                    { "required": ["center"] },
                    { "required": ["width"] },
                    { "required": ["height"] },
                ],
            },
        },
    ]);
    def
}
