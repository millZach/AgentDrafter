//! Raster images (`image:`) and PDF underlays (`pdf_underlay:`) in a sheet
//! description, template or block library (#91).

use serde_json::{json, Value};

use super::block_schema::relative_path_property;
use super::blocks::{relative_path, RelativePath};
use super::schema::{object_def, NUMBER};
use super::strict_yaml::{index_path, Node};
use super::wipeout::WIPEOUT_FRAME_NAMES;
use super::{
    angle, bad_value, boolean, choice, length, plain_number, point, scalar, sequence, wrong_type,
    Fields,
};
use crate::error::Error;
use crate::model::revision_cloud::signed_area;
use crate::model::{
    PdfUnderlay, Point3, Properties, RasterImage, WipeoutFrame, DEFAULT_IMAGE_BRIGHTNESS,
    DEFAULT_IMAGE_CONTRAST, DEFAULT_PDF_CONTRAST, MAX_PDF_FADE,
};
use crate::units::Unit;

/// The top-level key controlling raster image frame visibility (`IMAGEFRAME`).
pub(crate) const IMAGE_FRAME_KEY: &str = "image_frame";

/// The keys a raster image (`image:`) takes.
pub(crate) const IMAGE_KEYS: [&str; 19] = [
    "path",
    "position",
    "width",
    "height",
    "rotation",
    "clip",
    "clip_inverted",
    "brightness",
    "contrast",
    "fade",
    "transparent",
    "show",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
    "plot_style",
    "transparency",
];

/// The keys a PDF underlay (`pdf_underlay:`) takes.
pub(crate) const PDF_UNDERLAY_KEYS: [&str; 19] = [
    "path",
    "page",
    "position",
    "scale",
    "rotation",
    "clip",
    "clip_inverted",
    "contrast",
    "fade",
    "monochrome",
    "show",
    "unloaded",
    "layer",
    "color",
    "linetype",
    "lineweight",
    "linetype_scale",
    "plot_style",
    "transparency",
];

/// Reads the sheet-level `image_frame` setting (`off`, `on`, or `display`).
pub(super) fn image_frame(node: &Node, path: &str) -> Result<WipeoutFrame, Error> {
    let text = choice(node, path, "an image_frame setting", "off, on or display")?;
    match text {
        "off" => Ok(WipeoutFrame::Off),
        "on" => Ok(WipeoutFrame::On),
        "display" => Ok(WipeoutFrame::Display),
        _ => Err(bad_value(
            node,
            path,
            format!(
                "unknown image_frame `{text}`; known values: {}",
                WIPEOUT_FRAME_NAMES.join(", ")
            ),
        )),
    }
}

/// Reads an `image` object from `fields`.
pub(super) fn raster_image(
    fields: &Fields,
    units: Unit,
    layer: String,
) -> Result<RasterImage, Error> {
    let path_node = fields.require_text("path")?;
    let file_path = relative_path(
        path_node,
        &fields.path_of("path"),
        &RelativePath {
            shape: "the path of a raster image file, such as `assets/logo.png`",
            whose: "the raster image's path",
            from: "the sheet description's or block library's folder",
            names: "a file",
        },
    )?;
    let position = point(
        fields.require("position")?,
        &fields.path_of("position"),
        units,
    )?;
    let width_node = fields.require("width")?;
    let width_path = fields.path_of("width");
    let width = length(width_node, &width_path, units)?;
    if width <= 0.0 {
        return Err(bad_value(
            width_node,
            &width_path,
            "an image's width must be more than 0",
        ));
    }
    let height = match fields.get("height") {
        Some(node) => {
            let p = fields.path_of("height");
            let h = length(node, &p, units)?;
            if h <= 0.0 {
                return Err(bad_value(node, &p, "an image's height must be more than 0"));
            }
            Some(h)
        }
        None => None,
    };
    let rotation = match fields.get("rotation") {
        Some(node) => angle(node, &fields.path_of("rotation"))?,
        None => 0.0,
    };
    let clip = match fields.get("clip") {
        Some(node) => Some(read_clip(node, &fields.path_of("clip"), units, "image")?),
        None => None,
    };
    let clip_inverted = match fields.get("clip_inverted") {
        Some(node) => boolean(node, &fields.path_of("clip_inverted"))?,
        None => false,
    };
    let brightness = match fields.get("brightness") {
        Some(node) => read_u8_range(node, &fields.path_of("brightness"), 0, 100, "brightness")?,
        None => DEFAULT_IMAGE_BRIGHTNESS,
    };
    let contrast = match fields.get("contrast") {
        Some(node) => read_u8_range(node, &fields.path_of("contrast"), 0, 100, "contrast")?,
        None => DEFAULT_IMAGE_CONTRAST,
    };
    let fade = match fields.get("fade") {
        Some(node) => read_u8_range(node, &fields.path_of("fade"), 0, 100, "fade")?,
        None => 0,
    };
    let transparent = match fields.get("transparent") {
        Some(node) => boolean(node, &fields.path_of("transparent"))?,
        None => true,
    };
    let show = match fields.get("show") {
        Some(node) => boolean(node, &fields.path_of("show"))?,
        None => true,
    };
    Ok(RasterImage {
        path: file_path.to_owned(),
        position,
        width,
        height,
        rotation,
        clip,
        clip_inverted,
        brightness,
        contrast,
        fade,
        transparent,
        show,
        layer,
        properties: Properties::default(),
    })
}

/// Reads a `pdf_underlay` object from `fields`.
pub(super) fn pdf_underlay(
    fields: &Fields,
    units: Unit,
    layer: String,
) -> Result<PdfUnderlay, Error> {
    let path_node = fields.require_text("path")?;
    let file_path = relative_path(
        path_node,
        &fields.path_of("path"),
        &RelativePath {
            shape: "the path of a PDF file, such as `markups/rev-a.pdf`",
            whose: "the PDF underlay's path",
            from: "the sheet description's or block library's folder",
            names: "a file",
        },
    )?;
    let page = match fields.get("page") {
        Some(node) => {
            let p = fields.path_of("page");
            let text = scalar(node, &p, "a 1-based page number such as `1`")?;
            let n: usize = text.parse().ok().filter(|&n| n >= 1).ok_or_else(|| {
                bad_value(
                    node,
                    &p,
                    format!("`{text}` is not a valid 1-based PDF page number"),
                )
            })?;
            n.to_string()
        }
        None => "1".to_owned(),
    };
    let position = point(
        fields.require("position")?,
        &fields.path_of("position"),
        units,
    )?;
    let scale = match fields.get("scale") {
        Some(node) => {
            let p = fields.path_of("scale");
            let s = plain_number(node, &p, "a scale factor more than 0, such as `1`")?;
            if s <= 0.0 {
                return Err(bad_value(
                    node,
                    &p,
                    "a PDF underlay's scale must be more than 0",
                ));
            }
            s
        }
        None => 1.0,
    };
    let rotation = match fields.get("rotation") {
        Some(node) => angle(node, &fields.path_of("rotation"))?,
        None => 0.0,
    };
    let clip = match fields.get("clip") {
        Some(node) => Some(read_clip(
            node,
            &fields.path_of("clip"),
            units,
            "PDF underlay",
        )?),
        None => None,
    };
    let clip_inverted = match fields.get("clip_inverted") {
        Some(node) => boolean(node, &fields.path_of("clip_inverted"))?,
        None => false,
    };
    let contrast = match fields.get("contrast") {
        Some(node) => read_u8_range(node, &fields.path_of("contrast"), 0, 100, "contrast")?,
        None => DEFAULT_PDF_CONTRAST,
    };
    let fade = match fields.get("fade") {
        Some(node) => read_u8_range(node, &fields.path_of("fade"), 0, MAX_PDF_FADE, "fade")?,
        None => 0,
    };
    let monochrome = match fields.get("monochrome") {
        Some(node) => boolean(node, &fields.path_of("monochrome"))?,
        None => false,
    };
    let show = match fields.get("show") {
        Some(node) => boolean(node, &fields.path_of("show"))?,
        None => true,
    };
    let unloaded = match fields.get("unloaded") {
        Some(node) => boolean(node, &fields.path_of("unloaded"))?,
        None => false,
    };
    Ok(PdfUnderlay {
        path: file_path.to_owned(),
        page,
        position,
        scale,
        rotation,
        clip,
        clip_inverted,
        contrast,
        fade,
        monochrome,
        show,
        unloaded,
        layer,
        properties: Properties::default(),
    })
}

fn read_u8_range(node: &Node, path: &str, min: u8, max: u8, label: &str) -> Result<u8, Error> {
    let text = scalar(
        node,
        path,
        &format!("an integer from {min} to {max} for {label}"),
    )?;
    let n: i64 = text.parse().map_err(|_| {
        bad_value(
            node,
            path,
            format!("expected an integer from {min} to {max} for {label}, found `{text}`"),
        )
    })?;
    if n < i64::from(min) || n > i64::from(max) {
        return Err(bad_value(
            node,
            path,
            format!("{label} must be from {min} to {max}; found {n}"),
        ));
    }
    Ok(n as u8)
}

fn read_clip(node: &Node, path: &str, units: Unit, what: &str) -> Result<Vec<[f64; 2]>, Error> {
    let items = sequence(node, path, "a list of at least 3 clip points")?;
    if items.len() < 3 {
        return Err(wrong_type(
            node,
            path,
            &format!(
                "a list of at least 3 points round the {what} clip boundary; this one has {}",
                items.len()
            ),
        ));
    }
    let mut pts: Vec<Point3> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        pts.push(point(item, &index_path(path, i), units)?);
    }
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        if a.x == b.x && a.y == b.y {
            let at_idx = (i + 1) % pts.len();
            return Err(bad_value(
                &items[at_idx],
                &index_path(path, at_idx),
                format!("two consecutive {what} clip points cannot be the same"),
            ));
        }
    }
    if signed_area(&pts).abs() <= 1e-12 {
        return Err(bad_value(
            node,
            path,
            format!("a {what}'s clip boundary must enclose an area more than 0"),
        ));
    }
    Ok(pts.into_iter().map(|p| [p.x, p.y]).collect())
}

fn bool_schema(description: &str) -> Value {
    json!({
        "description": description,
        "anyOf": [
            { "type": "boolean" },
            { "enum": ["true", "false"] },
        ],
    })
}

fn u8_range_schema(description: &str, max: u8) -> Value {
    json!({
        "description": description,
        "anyOf": [
            { "type": "integer", "minimum": 0, "maximum": max },
            { "type": "string", "pattern": "^(0|[1-9][0-9]*)$" },
        ],
    })
}

/// JSON Schema definition for `image` (`RasterImage`).
pub(super) fn image_def() -> Value {
    object_def(
        "A raster image, AutoCAD's IMAGE / IMAGEATTACH: an external PNG, JPEG, BMP, GIF or TIFF \
         placed with its lower-left corner at `position`, spanning `width` (and optional `height`, \
         preserving the pixel aspect ratio when omitted) and turned by `rotation`.",
        &["path", "position", "width"],
        json!({
            "path": relative_path_property(
                "Path to the raster image file (PNG, JPEG, BMP, GIF or TIFF), relative to the \
                 sheet description's or block library's folder, with `/` between folders."
                    .to_owned(),
            ),
            "position": {
                "$ref": "#/$defs/point",
                "description": "The image's lower-left corner in drawing units.",
            },
            "width": {
                "$ref": "#/$defs/length",
                "description": "The displayed width in drawing units, more than 0.",
            },
            "height": {
                "$ref": "#/$defs/length",
                "description": "Optional displayed height in drawing units, more than 0. Omitting \
                                it preserves the image's natural pixel aspect ratio.",
            },
            "rotation": {
                "$ref": "#/$defs/angle",
                "description": "Anticlockwise rotation around `position`, in degrees. Without it, 0.",
            },
            "clip": {
                "type": "array",
                "description": "Optional polygonal clip boundary of at least 3 points in drawing \
                                coordinates enclosing a non-zero area.",
                "items": { "$ref": "#/$defs/point" },
                "minItems": 3,
            },
            "clip_inverted": bool_schema(
                "When true and `clip` is given, masks the inside of `clip` and shows the outside. \
                 Without it, false.",
            ),
            "brightness": u8_range_schema(
                "Image brightness from 0 to 100. Without it, 50.",
                100,
            ),
            "contrast": u8_range_schema(
                "Image contrast from 0 to 100. Without it, 50.",
                100,
            ),
            "fade": u8_range_schema(
                "Image fade towards the background from 0 to 100. Without it, 0.",
                100,
            ),
            "transparent": bool_schema(
                "Whether the image's alpha channel is honoured (AutoCAD TRANSPARENCY_ON). \
                 Without it, true.",
            ),
            "show": bool_schema(
                "Whether the image pixels draw (AutoCAD SHOW_IMAGE). Without it, true.",
            ),
        }),
    )
}

/// JSON Schema definition for `pdf_underlay` (`PdfUnderlay`).
pub(super) fn pdf_underlay_def() -> Value {
    object_def(
        "A PDF underlay, AutoCAD's PDFUNDERLAY / PDFATTACH: one page of an external PDF placed \
         with its lower-left corner at `position`, at `scale` times its physical page size in the \
         sheet's units, turned by `rotation`.",
        &["path", "position"],
        json!({
            "path": relative_path_property(
                "Path to the PDF file, relative to the sheet description's or block library's \
                 folder, with `/` between folders."
                    .to_owned(),
            ),
            "page": {
                "description": "1-based page number of the PDF to display (such as `1`). Without \
                                it, `1`.",
                "anyOf": [
                    { "type": "string", "pattern": "^[1-9][0-9]*$" },
                    { "type": "integer", "minimum": 1 },
                ],
            },
            "position": {
                "$ref": "#/$defs/point",
                "description": "The PDF underlay's lower-left corner in drawing units.",
            },
            "scale": {
                "description": "Uniform scale factor applied to the PDF page's physical size, \
                                more than 0. Without it, 1.",
                "anyOf": [
                    { "type": "number", "exclusiveMinimum": 0 },
                    { "type": "string", "pattern": format!("^{NUMBER}$") },
                ],
            },
            "rotation": {
                "$ref": "#/$defs/angle",
                "description": "Anticlockwise rotation around `position`, in degrees. Without it, 0.",
            },
            "clip": {
                "type": "array",
                "description": "Optional polygonal clip boundary of at least 3 points in drawing \
                                coordinates enclosing a non-zero area.",
                "items": { "$ref": "#/$defs/point" },
                "minItems": 3,
            },
            "clip_inverted": bool_schema(
                "When true and `clip` is given, masks the inside of `clip` and shows the outside. \
                 Without it, false.",
            ),
            "contrast": u8_range_schema(
                "PDF underlay contrast from 0 to 100. Without it, 100.",
                100,
            ),
            "fade": u8_range_schema(
                "PDF underlay fade towards the background from 0 to 80. Without it, 0.",
                MAX_PDF_FADE,
            ),
            "monochrome": bool_schema(
                "When true, draws the PDF underlay in monochrome (grayscale). Without it, false.",
            ),
            "show": bool_schema(
                "Whether the PDF underlay draws (AutoCAD ON flag). Without it, true.",
            ),
            "unloaded": bool_schema(
                "When true, keeps the PDF underlay reference in the DWG without loading or \
                 drawing the PDF file. Without it, false.",
            ),
        }),
    )
}
