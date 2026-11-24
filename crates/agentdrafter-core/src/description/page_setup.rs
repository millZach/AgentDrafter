//! Reads a sheet's or layout's `page_setup` (`PAGESETUP`), named plot style
//! references, and transparency, and states them in the JSON Schema.

use serde_json::{json, Value};

use super::schema::{POSITIVE_NUMBER, TEXT_TYPES};
use super::strict_yaml::{index_path, Kind, Node};
use super::{at, bad_value, boolean, length, scalar, sequence, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{
    ObjectPlotStyle, ObjectTransparency, PageSetup, PlotArea, PlotScale, BY_BLOCK, BY_LAYER,
    MAX_TRANSPARENCY,
};
use crate::units::Unit;

/// The key a sheet description, template or layout declares its page setup under.
pub(crate) const PAGE_SETUP_KEY: &str = "page_setup";

/// The keys a `page_setup` takes.
pub(crate) const PAGE_SETUP_KEYS: [&str; 8] = [
    "paper",
    "area",
    "window",
    "scale",
    "center",
    "offset",
    "plot_style",
    "plot_transparency",
];

/// Longest named plot style AutoCAD allows in an STB file.
const LONGEST_STYLE_NAME: usize = 255;

/// Pattern for a CTB or STB plot style file name (no path separators).
pub(crate) const PLOT_STYLE_FILE_PATTERN: &str = "^[^/\\\\:\\x00-\\x1f]+\\.[CcSs][Tt][Bb]$";

/// Whether `name` is a plain CTB or STB plot style file name, such as
/// `monochrome.ctb` or `plant.stb`, with no path separators or control
/// characters.
pub(crate) fn is_plot_style_file(name: &str) -> bool {
    if name.len() <= 4 || name.contains(['/', '\\', ':']) || name.chars().any(char::is_control) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".ctb") || lower.ends_with(".stb")
}

/// Validates a CTB or STB file name in YAML (used by `page_setup.plot_style`
/// and `set.yaml`'s `plot_style`).
pub(super) fn plot_style_file(node: &Node, path: &str) -> Result<String, Error> {
    let name = scalar(
        node,
        path,
        "a CTB or STB plot style file name, such as `monochrome.ctb` or `plant.stb`",
    )?;
    if !is_plot_style_file(name) {
        return Err(bad_value(
            node,
            path,
            format!(
                "a plot style is named by its CTB or STB file name, such as `monochrome.ctb` or \
                 `plant.stb`, with no folder path; `{name}` is not such a name"
            ),
        ));
    }
    Ok(name.to_owned())
}

/// Validates a named plot style (from an STB table, such as `Normal` or `Heavy`).
pub(super) fn plot_style_name(node: &Node, path: &str) -> Result<String, Error> {
    let name = scalar(
        node,
        path,
        "a named plot style, such as `Normal` or `Heavy`",
    )?;
    if let Some(problem) = style_name_problem(name) {
        return Err(bad_value(node, path, problem));
    }
    Ok(name.to_owned())
}

/// What is wrong with `name` as a named plot style, if anything.
pub(crate) fn style_name_problem(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("a plot style name cannot be empty".to_owned());
    }
    if name.starts_with(char::is_whitespace) || name.ends_with(char::is_whitespace) {
        return Some(format!(
            "plot style name `{name}` cannot start or end with a space"
        ));
    }
    if name.chars().count() > LONGEST_STYLE_NAME {
        return Some(format!(
            "a plot style name is at most {LONGEST_STYLE_NAME} characters"
        ));
    }
    if name.chars().any(char::is_control) {
        return Some("a plot style name cannot hold control characters".to_owned());
    }
    None
}

/// Reads an object's `plot_style` property: `ByLayer`, `ByBlock`, or a named
/// plot style from an STB table.
pub(super) fn object_plot_style(node: &Node, path: &str) -> Result<ObjectPlotStyle, Error> {
    let name = scalar(
        node,
        path,
        "`ByLayer`, `ByBlock`, or a named plot style such as `Heavy`",
    )?;
    if name == BY_LAYER {
        return Ok(ObjectPlotStyle::ByLayer);
    }
    if name == BY_BLOCK {
        return Ok(ObjectPlotStyle::ByBlock);
    }
    if let Some(problem) = style_name_problem(name) {
        return Err(bad_value(node, path, problem));
    }
    Ok(ObjectPlotStyle::Named(name.to_owned()))
}

/// Reads a layer's transparency percentage: an integer from `0` to `90`.
pub(super) fn layer_transparency(node: &Node, path: &str) -> Result<u8, Error> {
    let text = scalar(
        node,
        path,
        "a transparency percentage, a whole number from 0 (opaque) to 90",
    )?;
    parse_transparency_percent(text, node, path)
}

/// Reads an object's transparency: `ByLayer`, `ByBlock`, or an integer `0..=90`.
pub(super) fn object_transparency(node: &Node, path: &str) -> Result<ObjectTransparency, Error> {
    match &node.kind {
        Kind::Scalar(text) if text == BY_LAYER => Ok(ObjectTransparency::ByLayer),
        Kind::Scalar(text) if text == BY_BLOCK => Ok(ObjectTransparency::ByBlock),
        _ => {
            let text = scalar(
                node,
                path,
                "`ByLayer`, `ByBlock`, or a transparency percentage from 0 to 90",
            )?;
            Ok(ObjectTransparency::Percent(parse_transparency_percent(
                text, node, path,
            )?))
        }
    }
}

fn parse_transparency_percent(text: &str, node: &Node, path: &str) -> Result<u8, Error> {
    let value: i64 = text.trim().parse().map_err(|_| {
        bad_value(
            node,
            path,
            format!(
                "`{text}` is not a transparency percentage; give a whole number from 0 (opaque) \
                 to {MAX_TRANSPARENCY}"
            ),
        )
    })?;
    if !(0..=i64::from(MAX_TRANSPARENCY)).contains(&value) {
        return Err(bad_value(
            node,
            path,
            format!(
                "transparency `{value}` is out of range; AutoCAD allows whole numbers from 0 \
                 (opaque) to {MAX_TRANSPARENCY}"
            ),
        ));
    }
    Ok(u8::try_from(value).expect("0..=90"))
}

/// Reads a `page_setup` mapping on a sheet or layout.
pub(super) fn page_setup(
    node: &Node,
    path: &str,
    units: Unit,
    is_layout: bool,
) -> Result<PageSetup, Error> {
    let fields = Fields::of(node, path, &PAGE_SETUP_KEYS, "a page setup")?;
    let mut setup = PageSetup::default();

    if let Some(paper_node) = fields.get("paper") {
        setup.paper = Some(super::paper::paper(
            paper_node,
            &fields.path_of("paper"),
            units,
        )?);
    }

    let area_word = match fields.get("area") {
        Some(area_node) => {
            let area_path = fields.path_of("area");
            let word = scalar(area_node, &area_path, "`extents`, `window` or `layout`")?;
            match word {
                "extents" | "window" => Some((word, area_node)),
                "layout" => {
                    if !is_layout {
                        return Err(bad_value(
                            area_node,
                            &area_path,
                            "`area: layout` is only for a paper-space layout; a model-space sheet \
                             plots `extents` or `window`",
                        ));
                    }
                    Some((word, area_node))
                }
                _ => {
                    return Err(bad_value(
                        area_node,
                        &area_path,
                        format!(
                            "`{word}` is not a plot area; choose `extents`, `window` or `layout`"
                        ),
                    ));
                }
            }
        }
        None => None,
    };

    let window_bounds = match fields.get("window") {
        Some(win_node) => Some((
            plot_window(win_node, &fields.path_of("window"), units)?,
            win_node,
        )),
        None => None,
    };

    setup.area = match (area_word, window_bounds) {
        (Some(("window", _)), Some(((min, max), _))) => Some(PlotArea::Window { min, max }),
        (Some(("window", area_node)), None) => {
            return Err(at(
                Error::new(
                    ErrorCode::MissingKey,
                    "`area: window` needs `window: [[x1, y1], [x2, y2]]`, the two opposite \
                     corners of the plotted rectangle",
                ),
                path,
                area_node.at,
            ));
        }
        (None, Some(((min, max), _))) => Some(PlotArea::Window { min, max }),
        (Some(("extents", _)), None) => Some(PlotArea::Extents),
        (Some(("layout", _)), None) => Some(PlotArea::Layout),
        (Some((other, _)), Some((_, win_node))) => {
            return Err(bad_value(
                win_node,
                &fields.path_of("window"),
                format!("`window` is only for `area: window`, not `area: {other}`"),
            ));
        }
        (None, None) => None,
        _ => unreachable!(),
    };

    if let Some(scale_node) = fields.get("scale") {
        setup.scale = plot_scale(scale_node, &fields.path_of("scale"))?;
    }

    if let Some(center_node) = fields.get("center") {
        setup.center = boolean(center_node, &fields.path_of("center"))?;
    }

    if let Some(offset_node) = fields.get("offset") {
        let offset_path = fields.path_of("offset");
        let offset = offset_2d(offset_node, &offset_path, units)?;
        if setup.center && (offset[0] != 0.0 || offset[1] != 0.0) {
            return Err(bad_value(
                offset_node,
                &offset_path,
                "`offset` cannot be non-zero when `center: true`",
            ));
        }
        setup.offset = offset;
    }

    if let Some(style_node) = fields.get("plot_style") {
        setup.plot_style = Some(plot_style_file(style_node, &fields.path_of("plot_style"))?);
    }

    if let Some(trans_node) = fields.get("plot_transparency") {
        setup.plot_transparency = boolean(trans_node, &fields.path_of("plot_transparency"))?;
    }

    Ok(setup)
}

/// Parses `window: [[x1, y1], [x2, y2]]` (2D or 3D points, z ignored) into
/// normalized `(min, max)` with positive width and height.
pub(super) fn plot_window(
    node: &Node,
    path: &str,
    units: Unit,
) -> Result<([f64; 2], [f64; 2]), Error> {
    let items = sequence(
        node,
        path,
        "two corners of the plot window, such as `[[0, 0], [100, 50]]`",
    )?;
    if items.len() != 2 {
        return Err(bad_value(
            node,
            path,
            format!(
                "a plot window needs two opposite corners, `[min, max]`; this has {}",
                items.len()
            ),
        ));
    }
    let a = corner_2d(&items[0], &index_path(path, 0), units)?;
    let b = corner_2d(&items[1], &index_path(path, 1), units)?;
    let min = [a[0].min(b[0]), a[1].min(b[1])];
    let max = [a[0].max(b[0]), a[1].max(b[1])];
    if max[0] <= min[0] || max[1] <= min[1] {
        return Err(bad_value(
            node,
            path,
            "a plot window's two corners must differ in both X and Y so the window has positive \
             width and height",
        ));
    }
    Ok((min, max))
}

fn corner_2d(node: &Node, path: &str, units: Unit) -> Result<[f64; 2], Error> {
    let items = sequence(node, path, "a 2D point `[x, y]`")?;
    if items.len() != 2 && items.len() != 3 {
        return Err(bad_value(
            node,
            path,
            format!(
                "expected `[x, y]` (or `[x, y, 0]`), got {} items",
                items.len()
            ),
        ));
    }
    let x = length(&items[0], &index_path(path, 0), units)?;
    let y = length(&items[1], &index_path(path, 1), units)?;
    Ok([x, y])
}

fn offset_2d(node: &Node, path: &str, units: Unit) -> Result<[f64; 2], Error> {
    let items = sequence(node, path, "a plot offset `[x, y]`")?;
    if items.len() != 2 {
        return Err(bad_value(
            node,
            path,
            format!("a plot offset is `[x, y]` (2 lengths), got {}", items.len()),
        ));
    }
    let x = length(&items[0], &index_path(path, 0), units)?;
    let y = length(&items[1], &index_path(path, 1), units)?;
    Ok([x, y])
}

/// Parses `scale`: `"fit"`, a ratio `"paper:drawing"` such as `"1:2"` or
/// `"2:1"`, or a positive plain number such as `0.5` or `1`.
pub(super) fn plot_scale(node: &Node, path: &str) -> Result<PlotScale, Error> {
    let text = scalar(
        node,
        path,
        "`fit`, a ratio such as `1:2` or `1:1`, or a positive number such as `0.5`",
    )?;
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("fit") {
        return Ok(PlotScale::Fit);
    }
    if let Some((num, den)) = trimmed.split_once(':') {
        let n: f64 = num.trim().parse().map_err(|_| {
            bad_value(
                node,
                path,
                format!("`{text}` is not a valid plot scale ratio such as `1:2`"),
            )
        })?;
        let d: f64 = den.trim().parse().map_err(|_| {
            bad_value(
                node,
                path,
                format!("`{text}` is not a valid plot scale ratio such as `1:2`"),
            )
        })?;
        if !(n.is_finite() && d.is_finite() && n > 0.0 && d > 0.0) {
            return Err(bad_value(
                node,
                path,
                format!("plot scale `{text}` must have positive numerator and denominator"),
            ));
        }
        return Ok(PlotScale::Ratio {
            numerator: n,
            denominator: d,
        });
    }
    let factor: f64 = trimmed.parse().map_err(|_| {
        bad_value(
            node,
            path,
            format!(
                "`{text}` is not a plot scale; give `fit`, a ratio such as `1:2`, or a positive \
                 number such as `0.5`"
            ),
        )
    })?;
    if !(factor.is_finite() && factor > 0.0) {
        return Err(bad_value(
            node,
            path,
            format!("plot scale `{text}` must be more than 0"),
        ));
    }
    Ok(PlotScale::Ratio {
        numerator: factor,
        denominator: 1.0,
    })
}

/// JSON Schema property for a CTB or STB plot style file name.
pub(crate) fn plot_style_file_schema(description: &str) -> Value {
    json!({
        "description": description,
        "type": "string",
        "pattern": PLOT_STYLE_FILE_PATTERN,
    })
}

/// JSON Schema definitions for named plot styles, transparency and page setup.
pub(crate) fn add_page_setup_defs(defs: &mut serde_json::Map<String, Value>) {
    let style_name_pattern = "^[^\\s\\x00-\\x1f]([^\\x00-\\x1f]*[^\\s\\x00-\\x1f])?$";
    defs.insert(
        "layer_plot_style".into(),
        json!({
            "description": "The named plot style this layer plots with in an STB drawing, such as \
                            `Normal` or `Heavy`. Leave out in a colour-dependent (CTB) drawing.",
            "type": "string",
            "minLength": 1,
            "maxLength": LONGEST_STYLE_NAME,
            "pattern": style_name_pattern,
        }),
    );
    defs.insert(
        "object_plot_style".into(),
        json!({
            "description": "The object's own named plot style in an STB drawing: `ByLayer`, \
                            `ByBlock`, or a named plot style such as `Heavy`. Without it, `ByLayer`.",
            "type": "string",
            "minLength": 1,
            "maxLength": LONGEST_STYLE_NAME,
            "pattern": style_name_pattern,
        }),
    );
    defs.insert(
        "layer_transparency".into(),
        json!({
            "description": format!(
                "Layer transparency percentage, a whole number from 0 (opaque) to \
                 {MAX_TRANSPARENCY} (90% transparent). Without it, 0."
            ),
            "type": "integer",
            "minimum": 0,
            "maximum": MAX_TRANSPARENCY,
        }),
    );
    defs.insert(
        "object_transparency".into(),
        json!({
            "description": format!(
                "The object's own transparency: `ByLayer`, `ByBlock`, or a whole number from 0 \
                 (opaque) to {MAX_TRANSPARENCY}. Without it, `ByLayer`."
            ),
            "anyOf": [
                { "enum": [BY_LAYER, BY_BLOCK] },
                { "type": "integer", "minimum": 0, "maximum": MAX_TRANSPARENCY },
            ],
        }),
    );
    defs.insert(
        "plot_scale".into(),
        json!({
            "description": "Plot scale: `fit` to fit the printable paper, a ratio `paper:drawing` \
                            such as `1:2` or `2:1`, or a positive number such as `0.5`. Without it, \
                            `1:1`.",
            "anyOf": [
                { "type": "number", "exclusiveMinimum": 0 },
                {
                    "type": "string",
                    "pattern": format!(
                        "^([Ff][Ii][Tt]|{POSITIVE_NUMBER}:{POSITIVE_NUMBER}|{POSITIVE_NUMBER})$"
                    ),
                },
            ],
        }),
    );
    let corner = json!({
        "type": "array",
        "minItems": 2,
        "maxItems": 3,
        "items": { "$ref": "#/$defs/length" },
    });
    defs.insert(
        "plot_window".into(),
        json!({
            "description": "Two opposite corners `[[x1, y1], [x2, y2]]` of the plotted rectangle, \
                            required when `area` is `window`.",
            "type": "array",
            "minItems": 2,
            "maxItems": 2,
            "items": corner,
        }),
    );
    defs.insert(
        "page_setup".into(),
        json!({
            "description": "A sheet's or layout's page setup (`PAGESETUP`): paper, plot area \
                            (`extents`, `window` or `layout`), plot scale (`fit` or a ratio such \
                            as `1:2`), centring, offset, plot style table (CTB or STB) and whether \
                            transparency plots.",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "paper": super::paper::paper_property(),
                "area": {
                    "description": "What area to plot: `extents`, `window` or `layout` (paper-space \
                                    layouts only).",
                    "enum": ["extents", "window", "layout"],
                },
                "window": { "$ref": "#/$defs/plot_window" },
                "scale": { "$ref": "#/$defs/plot_scale" },
                "center": {
                    "description": "Centre the plotted area on the printable paper. Without it, false.",
                    "type": "boolean",
                },
                "offset": {
                    "description": "Plot origin offset `[x, y]` in drawing units when `center` is \
                                    false. Without it, `[0, 0]`.",
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 2,
                    "items": { "$ref": "#/$defs/length" },
                },
                "plot_style": plot_style_file_schema(
                    "The CTB or STB plot style file name, such as `monochrome.ctb` or `plant.stb`, \
                     found in the plot style folders."
                ),
                "plot_transparency": {
                    "description": "Whether object and layer transparency plots. Without it, true.",
                    "type": "boolean",
                },
            },
        }),
    );
    let _ = TEXT_TYPES;
}
