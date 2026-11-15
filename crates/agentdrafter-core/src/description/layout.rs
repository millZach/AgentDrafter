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

use super::strict_yaml::Node;
use super::{at, bad_value, boolean, length, point, scalar, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{Paper, Point3, Viewport};
use crate::units::{parse_length, LengthError, Unit};

/// The key a sheet description or template declares its layout under.
pub(crate) const LAYOUT_KEY: &str = "layout";

/// The keys of a layout.
pub(crate) const LAYOUT_KEYS: [&str; 1] = ["name"];

/// The key an object of a sheet says which space it is in by.
pub(crate) const SPACE_KEY: &str = "space";

/// The space an object is in unless it says otherwise.
pub(crate) const MODEL: &str = "model";

/// The space a sheet's border, title block and viewports are in.
pub(crate) const PAPER: &str = "paper";

/// The keys a viewport takes.
pub(crate) const VIEWPORT_KEYS: [&str; 11] = [
    "center",
    "width",
    "height",
    "view_center",
    "scale",
    "locked",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
];

/// The longest name a layout can have, as AutoCAD's: 255 characters.
const LONGEST_NAME: usize = 255;

/// The characters AutoCAD refuses in a layout's name.
const NOT_IN_A_NAME: &str = "<>/\\\":;?*|,=`";

/// The name of the model-space tab, which no layout takes.
const MODEL_TAB: &str = "Model";

/// The name of the layout `node` declares, on a sheet in `units` whose
/// paper is `paper`, declared at `paper_node`: a layout's paper is a
/// standard size by name, and its unit millimetres or inches.
pub(super) fn layout(
    node: &Node,
    path: &str,
    units: Unit,
    paper: Option<(&Paper, &Node)>,
) -> Result<String, Error> {
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
        Some(_) => Ok(name.to_owned()),
    }
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

/// Whether the object whose keys are `fields` is in paper space, on a sheet
/// that has a layout when `layout` says so. A viewport always is.
pub(super) fn in_paper_space(fields: &Fields, layout: bool, viewport: bool) -> Result<bool, Error> {
    let Some(node) = fields.text(SPACE_KEY) else {
        if viewport && !layout {
            return Err(no_layout(fields, "a viewport"));
        }
        return Ok(viewport);
    };
    let path = fields.path_of(SPACE_KEY);
    let paper = match scalar(node, &path, "`model` or `paper`")? {
        MODEL if viewport => {
            return Err(bad_value(
                node,
                &path,
                "a viewport is in paper space, a window on the paper onto model space",
            ))
        }
        MODEL => false,
        PAPER => true,
        other => {
            return Err(at(
                Error::new(
                    ErrorCode::BadValue,
                    format!("`{other}` is not a space; `model` or `paper`"),
                ),
                &path,
                node.at,
            ))
        }
    };
    if paper && !layout {
        return Err(no_layout(fields, "an object in paper space"));
    }
    Ok(paper)
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

/// A viewport, on its layer.
pub(super) fn viewport(fields: &Fields, units: Unit, layer: String) -> Result<Viewport, Error> {
    let center = flat_point(fields, "center", units)?;
    let width = positive(fields, "width", units, "a viewport's width")?;
    let height = positive(fields, "height", units, "a viewport's height")?;
    let view_center = flat_point(fields, "view_center", units)?;
    let scale_node = fields.require("scale")?;
    let scale = scale(scale_node, &fields.path_of("scale"))?;
    let locked = match fields.get("locked") {
        Some(node) => boolean(node, &fields.path_of("locked"))?,
        None => false,
    };
    Ok(Viewport {
        center,
        width,
        height,
        view_center,
        scale,
        locked,
        layer,
        properties: Default::default(),
    })
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
            "name": {
                "description": format!(
                    "The layout's name, as AutoCAD's tab shows it: 1 to {LONGEST_NAME} \
                     characters, none of {NOT_IN_A_NAME}, and not `{MODEL_TAB}`."
                ),
                "type": "string",
                "minLength": 1,
                "maxLength": LONGEST_NAME,
                "pattern": "^[^<>/\\\\\":;?*|,=`]*\\S[^<>/\\\\\":;?*|,=`]*$",
                "not": { "pattern": "^[Mm][Oo][Dd][Ee][Ll]$" },
            }
        }
    })
}

/// The schema of the `space` key every object of a sheet takes.
pub(crate) fn space_property() -> Value {
    json!({
        "description": "The space the object is in, on a sheet drawn in a layout: `model`, at \
                        full size, where an object is unless it says otherwise, or `paper`, at \
                        the paper's size, as a sheet's border and title block are. A viewport is \
                        in paper space. A sheet without a layout has model space alone.",
        "enum": [MODEL, PAPER],
    })
}

/// The schema of a viewport, as a sheet's objects list it.
pub(crate) fn viewport_def() -> Value {
    super::schema::object_def(
        "A window on the paper that shows model space at a scale, centred on a model point, as          AutoCAD's MVIEW makes one: rectangular, seen from above. It is in the sheet's paper          space, on a sheet drawn in a layout, and its frame draws there with its own pen;          model space draws inside it, cut at its edge. Put it on a layer that does not plot to          leave its frame off a PDF.",
        &["center", "width", "height", "view_center", "scale"],
        json!({
            "center": {
                "description": "The middle of the window, on the paper, with 0 for z.",
                "$ref": "#/$defs/point",
            },
            "width": {
                "description": "How wide the window is on the paper, a length more than 0.",
                "$ref": "#/$defs/length",
            },
            "height": {
                "description": "How tall the window is on the paper, a length more than 0.",
                "$ref": "#/$defs/length",
            },
            "view_center": {
                "description": "The point of model space at the middle of the window, with 0                                 for z: AutoCAD's view centre.",
                "$ref": "#/$defs/point",
            },
            "scale": {
                "description": format!(
                    "{SCALE_SHAPE}. A length of model space draws this many times as long on                      the paper: 1:4 draws a quarter size. Text, hatch patterns and the geometry                      scale with it; lineweights do not, and a linetype's dashes are as long on                      the paper as in paper space, as AutoCAD's PSLTSCALE 1 draws them."
                ),
                "anyOf": [
                    { "type": "number", "exclusiveMinimum": 0 },
                    {
                        "type": "string",
                        "pattern": r"^\s*[0-9.eE+-]+\s*(:\s*[0-9.eE+-]+\s*)?$",
                    },
                ],
            },
            "locked": {
                "description": "Whether the viewport's view is locked, as AutoCAD's display                                 lock keeps it from being zoomed or panned: change_viewport                                 refuses to change the scale or the view centre of a locked                                 viewport unless it unlocks it too. It changes nothing drawn.",
                "type": "boolean",
                "default": false,
            },
        }),
    )
}
