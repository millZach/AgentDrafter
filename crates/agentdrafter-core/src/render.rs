//! Render: drawing a sheet to a PNG, through the drawing pipeline that the
//! PDF and the viewer also use (ADR-0011).
//!
//! The sheet becomes a display list ([`crate::draw`]), the display list
//! becomes pixels on the CPU ([`crate::raster`]), and the PNG carries a
//! stamp of the record it came from ([`crate::stamp`]). Nothing here needs
//! a window or a GPU.
//!
//! Until drawing sets and their paper sizes arrive (ticket #7), a render
//! covers the drawing's extents with a margin, at a plot scale or an image
//! size the caller gives. Lineweights are widths on paper, so they draw at
//! the render's dots per inch whatever its scale, as AutoCAD's plot fitted
//! to paper keeps them.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::description::parse_sheet_description;
use crate::draw::{self, Bounds};
use crate::dwg;
use crate::error::{Error, ErrorCode};
use crate::model::{Object, Sheet};
use crate::raster::{self, View};
use crate::settings::Settings;
use crate::stamp::{self, SourceKind, Stamp};
use crate::text::fonts::FontSubstitution;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// The dots per inch a render draws at unless asked for others.
pub const DEFAULT_DPI: f64 = 150.0;

/// The margin round the drawing, in millimetres of paper, unless asked for
/// another.
pub const DEFAULT_MARGIN: f64 = 5.0;

/// How many pixels the longer side of the image has when neither a scale
/// nor a size is asked for.
pub const DEFAULT_LONGER_SIDE: u32 = 2000;

/// The most pixels a render may have: 100 million, about 400 MB while it
/// is drawn.
pub const MOST_PIXELS: u64 = 100_000_000;

/// The most pixels either side of an image may have.
pub const MOST_SIDE: u32 = 32_767;

fn default_dpi() -> f64 {
    DEFAULT_DPI
}

fn default_margin() -> f64 {
    DEFAULT_MARGIN
}

/// A request to render a sheet to a PNG.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    /// The record to render: a sheet description, or a DWG when the file
    /// ends `.dwg`.
    pub source: PathBuf,
    /// Where to write the PNG. Without it, the PNG goes beside the source
    /// with the same name and a `.png` extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    /// Replace the file at the output path, which is allowed only when it
    /// is a PNG AgentDrafter rendered. Without it, an existing file is
    /// never touched.
    #[serde(default)]
    pub overwrite: bool,
    /// How many pixels an inch of paper spans: lineweights draw at this.
    #[serde(default = "default_dpi")]
    pub dpi: f64,
    /// The plot scale: how long a length on the drawing is on paper, as a
    /// share of itself, such as 1 for 1:1 and 0.02 for 1:50. The drawing's
    /// unit counts, so 1 m drawn at 0.02 is 20 mm on paper. A drawing with
    /// no unit is taken to be in millimetres. Not given with `width` or
    /// `height`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// How many pixels wide the image is, the drawing fitted inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// How many pixels high the image is, the drawing fitted inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// The margin round the drawing's extents, in millimetres of paper.
    #[serde(default = "default_margin")]
    pub margin: f64,
}

impl RenderRequest {
    /// A request to render `source` beside itself, at the defaults: 150
    /// dots per inch, a 5 mm margin, the longer side 2000 pixels.
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            output: None,
            overwrite: false,
            dpi: DEFAULT_DPI,
            scale: None,
            width: None,
            height: None,
            margin: DEFAULT_MARGIN,
        }
    }
}

/// What a render wrote.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RenderReport {
    /// The record that was rendered.
    pub source: String,
    /// The PNG that was written.
    pub png: String,
    /// The image's width in pixels.
    pub width: u32,
    /// The image's height in pixels.
    pub height: u32,
    /// How many pixels an inch of paper spans.
    pub dpi: f64,
    /// The plot scale the image is at: paper length over drawing length.
    pub scale: f64,
    /// How many pixels one drawing unit spans.
    pub pixels_per_unit: f64,
    /// The drawing point at the image's top left corner, `[left, top]`: the
    /// drawing's point `(x, y)` is at pixel `((x - left) * pixels_per_unit,
    /// (top - y) * pixels_per_unit)`, counted from the top left.
    pub top_left: [f64; 2],
    /// The drawing's unit; none when a DWG has none the core knows.
    pub units: Option<Unit>,
    /// The box round everything drawn, in drawing units; none when nothing
    /// draws.
    pub extents: Option<Bounds>,
    /// How many objects of each type were drawn, such as `{"line": 1}`.
    pub objects: BTreeMap<String, usize>,
    /// How many objects of a DWG were not drawn because the core has no
    /// model for them yet. Always 0 for a sheet description.
    pub objects_not_drawn: usize,
    /// The size of the PNG in bytes.
    pub bytes: usize,
    /// The text styles whose fonts are not in the font folders, and the
    /// look-alike drawn in their place. Empty when every font was found.
    pub font_substitutions: Vec<FontSubstitution>,
    /// The hatches not drawn in full, their pattern needing more lines or
    /// dashes than one hatch draws. Empty when every hatch was drawn whole.
    pub hatches_cut_short: Vec<HatchCutShort>,
    /// The stamp written into the PNG.
    pub stamp: Stamp,
}

/// A hatch whose pattern needed more lines or dashes than one hatch draws:
/// the PNG holds only the first of them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HatchCutShort {
    /// The hatch's place in the sheet's objects, counted from 0.
    pub object: usize,
    /// Its pattern.
    pub pattern: String,
    /// Its layer.
    pub layer: String,
    /// Why it was not drawn in full.
    pub why: String,
}

/// Renders a sheet to a PNG, reading fonts from the font folders the
/// environment names ([`Settings::from_env`]).
pub fn render(request: &RenderRequest) -> Result<RenderReport, Error> {
    render_with(request, &Settings::from_env())
}

/// Renders a sheet to a PNG, reading fonts from the font folders `settings`
/// names. The PNG is written whole or not at all.
pub fn render_with(request: &RenderRequest, settings: &Settings) -> Result<RenderReport, Error> {
    render_within(request, settings, draw::MOST_HATCH_DASHES)
}

/// Renders a sheet to a PNG as [`render_with`] does, each hatch drawing at
/// most `most_dashes` dashes.
fn render_within(
    request: &RenderRequest,
    settings: &Settings,
    most_dashes: usize,
) -> Result<RenderReport, Error> {
    check(request)?;
    let source = request.source.as_path();
    let bytes = fs::read(source).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the {}: {e}", SourceKind::of(source).words()),
        )
        .in_file(source)
    })?;
    let (sheet, units, objects_not_drawn) = match SourceKind::of(source) {
        SourceKind::SheetDescription => {
            let text = String::from_utf8(bytes.clone()).map_err(|_| {
                Error::new(
                    ErrorCode::ReadFailed,
                    "could not read the sheet description: it is not UTF-8 text",
                )
                .in_file(source)
            })?;
            let sheet = parse_sheet_description(&text).map_err(|e| e.in_file(source))?;
            let units = Some(sheet.units);
            (sheet, units, 0)
        }
        SourceKind::Dwg => {
            let drawing = dwg::read(source)?;
            let sheet = Sheet {
                units: drawing.units.unwrap_or(Unit::Mm),
                block_library: None,
                linetypes: drawing.linetypes,
                layers: drawing.layers,
                text_styles: drawing.text_styles,
                blocks: drawing.blocks,
                objects: drawing.objects,
            };
            (sheet, drawing.units, drawing.objects_not_read)
        }
    };

    let output = request
        .output
        .clone()
        .unwrap_or_else(|| source.with_extension("png"));
    refuse_to_replace(request, &output)?;

    let fonts = SheetFonts::find(&sheet, settings);
    let pixels_per_millimetre = request.dpi / 25.4;
    let margin = request.margin * pixels_per_millimetre;
    let millimetres_per_unit = sheet.units.convert(1.0, Unit::Mm);
    let area = |list: &draw::DisplayList| {
        list.extents().unwrap_or(Bounds {
            min: [0.0, 0.0],
            max: [0.0, 0.0],
        })
    };
    // A coarse list first, curves to within a drawing unit, for the scale;
    // then the list drawn to a quarter of a pixel, finer than the eye sees,
    // whose extents the image covers.
    let coarse = draw::draw_sheet(&sheet, &fonts, 1.0, most_dashes);
    let (rough, _) = fit(
        request,
        area(&coarse),
        pixels_per_millimetre,
        millimetres_per_unit,
        margin,
    )?;
    let list = draw::draw_sheet(&sheet, &fonts, 0.25 / rough, most_dashes);
    let extents = list.extents();
    let (pixels_per_unit, size) = fit(
        request,
        area(&list),
        pixels_per_millimetre,
        millimetres_per_unit,
        margin,
    )?;
    let view = View::around(
        area(&list),
        pixels_per_unit,
        pixels_per_millimetre,
        margin,
        size,
    );
    check_size(view.width, view.height)?;

    let pixmap = raster::rasterize(&list, &view).ok_or_else(|| {
        Error::new(
            ErrorCode::WriteFailed,
            format!(
                "could not make an image of {} by {} pixels",
                view.width, view.height
            ),
        )
    })?;
    let stamp = Stamp::new(source, &bytes, &output);
    let png = raster::png(&pixmap, &stamp.to_json(), pixels_per_millimetre).map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not encode the PNG: {e}"),
        )
        .in_file(&output)
    })?;
    write_whole(&output, &png)?;

    let mut objects = BTreeMap::new();
    for object in &sheet.objects {
        *objects.entry(object.type_name().to_owned()).or_insert(0) += 1;
    }
    Ok(RenderReport {
        source: source.display().to_string(),
        png: output.display().to_string(),
        width: view.width,
        height: view.height,
        dpi: request.dpi,
        scale: pixels_per_unit / pixels_per_millimetre / millimetres_per_unit,
        pixels_per_unit,
        top_left: view.top_left,
        units,
        extents,
        objects,
        objects_not_drawn,
        bytes: png.len(),
        font_substitutions: fonts.substitutions(),
        hatches_cut_short: cut_short(&sheet, &list, most_dashes),
        stamp,
    })
}

/// The hatches the display list could not draw in full, and why.
fn cut_short(sheet: &Sheet, list: &draw::DisplayList, most_dashes: usize) -> Vec<HatchCutShort> {
    list.hatches_cut_short
        .iter()
        .filter_map(|&object| match sheet.objects.get(object) {
            Some(Object::Hatch(hatch)) => Some(HatchCutShort {
                object,
                pattern: hatch.pattern.name.clone(),
                layer: hatch.layer.clone(),
                why: format!(
                    "its pattern needs more than the {} lines or {most_dashes} dashes one \
                     hatch draws; only the first are drawn",
                    draw::MOST_HATCH_LINES
                ),
            }),
            _ => None,
        })
        .collect()
}

/// Refuses a request that cannot be rendered as asked, naming its field.
fn check(request: &RenderRequest) -> Result<(), Error> {
    let bad = |field: &str, message: String| {
        Err(Error::new(ErrorCode::BadValue, message).at_field(field))
    };
    if !(request.dpi.is_finite() && request.dpi > 0.0 && request.dpi <= 10_000.0) {
        return bad(
            "dpi",
            "dots per inch are more than 0 and at most 10000".to_owned(),
        );
    }
    if !(request.margin.is_finite() && request.margin >= 0.0) {
        return bad("margin", "a margin is 0 or more millimetres".to_owned());
    }
    if let Some(scale) = request.scale {
        if !(scale.is_finite() && scale > 0.0) {
            return bad(
                "scale",
                "a plot scale is more than 0, such as 1 for 1:1 or 0.02 for 1:50".to_owned(),
            );
        }
        if request.width.is_some() || request.height.is_some() {
            return Err(Error::usage(
                "give a plot scale or an image size (width, height), not both",
            ));
        }
    }
    for (field, value) in [("width", request.width), ("height", request.height)] {
        if let Some(pixels) = value {
            if pixels == 0 || pixels > MOST_SIDE {
                return bad(
                    field,
                    format!("an image is 1 to {MOST_SIDE} pixels {field}"),
                );
            }
        }
    }
    Ok(())
}

/// How many pixels a drawing unit spans, and the image's size when the
/// request fixes it.
fn fit(
    request: &RenderRequest,
    area: Bounds,
    pixels_per_millimetre: f64,
    millimetres_per_unit: f64,
    margin: f64,
) -> Result<(f64, Option<(u32, u32)>), Error> {
    if let Some(scale) = request.scale {
        return Ok((scale * millimetres_per_unit * pixels_per_millimetre, None));
    }
    let (w, h) = (area.width(), area.height());
    let one_to_one = millimetres_per_unit * pixels_per_millimetre;
    let longer = f64::from(DEFAULT_LONGER_SIDE);
    // The pixels across and down the drawing may take, before the margin.
    let (across, down) = match (request.width, request.height) {
        (None, None) => {
            if w >= h {
                (Some(longer), None)
            } else {
                (None, Some(longer))
            }
        }
        (width, height) => (width.map(f64::from), height.map(f64::from)),
    };
    let room = |pixels: f64, field: &str| -> Result<f64, Error> {
        let room = pixels - 2.0 * margin;
        if room > 0.0 {
            Ok(room)
        } else {
            Err(Error::new(
                ErrorCode::BadValue,
                format!(
                    "a margin of {} mm leaves no room for the drawing in {pixels} pixels",
                    request.margin
                ),
            )
            .at_field(field))
        }
    };
    let mut candidates = Vec::new();
    if let Some(pixels) = across {
        let room = room(pixels, "width")?;
        if w > 0.0 {
            candidates.push(room / w);
        }
    }
    if let Some(pixels) = down {
        let room = room(pixels, "height")?;
        if h > 0.0 {
            candidates.push(room / h);
        }
    }
    let pixels_per_unit = candidates.into_iter().fold(f64::INFINITY, f64::min);
    let pixels_per_unit = if pixels_per_unit.is_finite() {
        pixels_per_unit
    } else {
        one_to_one
    };
    let size = match (request.width, request.height) {
        (Some(width), Some(height)) => Some((width, height)),
        (Some(width), None) => Some((
            width,
            ((h * pixels_per_unit + 2.0 * margin).ceil() as u32).max(1),
        )),
        (None, Some(height)) => Some((
            ((w * pixels_per_unit + 2.0 * margin).ceil() as u32).max(1),
            height,
        )),
        (None, None) => None,
    };
    Ok((pixels_per_unit, size))
}

fn check_size(width: u32, height: u32) -> Result<(), Error> {
    if width > MOST_SIDE || height > MOST_SIDE || u64::from(width) * u64::from(height) > MOST_PIXELS
    {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "the PNG would be {width} by {height} pixels; the most is {MOST_SIDE} on a side and \
                 {MOST_PIXELS} in all, so ask for a smaller scale, size or dpi"
            ),
        ));
    }
    Ok(())
}

/// Refuses to write over the file at `output` unless asked to and it is a
/// PNG AgentDrafter rendered, as its stamp shows. Anything else may be
/// someone's own file, and is never overwritten.
fn refuse_to_replace(request: &RenderRequest, output: &Path) -> Result<(), Error> {
    if fs::symlink_metadata(output).is_err() {
        return Ok(());
    }
    if let (Ok(a), Ok(b)) = (fs::canonicalize(&request.source), fs::canonicalize(output)) {
        if a == b {
            return Err(Error::usage(
                "the output path is the source itself; choose another path for the PNG",
            )
            .in_file(output));
        }
    }
    if !request.overwrite {
        return Err(Error::new(
            ErrorCode::OutputExists,
            "a file is already at the output path; ask to overwrite it to replace a previous render",
        )
        .in_file(output));
    }
    match fs::read(output).map_err(|e| e.to_string()).and_then(|b| stamp::stamp_of(&b)) {
        Ok(_) => Ok(()),
        Err(why) => Err(Error::new(
            ErrorCode::NotAnOutput,
            format!(
                "{why}; it may be someone's own file, which is never overwritten, so choose another \
                 output path"
            ),
        )
        .in_file(output)),
    }
}

/// Writes `bytes` to a temporary file beside `path`, then moves it onto
/// `path`, so a reader never sees a half-written PNG.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let Some(name) = path.file_name() else {
        return Err(Error::usage("the output path does not name a file").in_file(path));
    };
    let mut temporary_name = OsString::from(".");
    temporary_name.push(name);
    temporary_name.push(format!(".{}.tmp", std::process::id()));
    let temporary: PathBuf = path.with_file_name(temporary_name);
    let written = File::create(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    written.map_err(|e| {
        let _ = fs::remove_file(&temporary);
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not write the PNG: {e}"),
        )
        .in_file(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hatch that reaches its budget of dashes partway is named in the
    /// reply, as a font substitution is, not drawn short in silence.
    #[test]
    fn a_hatch_cut_short_is_named_in_the_reply() {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-render-{}", std::process::id()))
            .join("cut-short");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("sheet.yaml");
        // Ten lines of five dashes each: fifty dashes.
        fs::write(
            &source,
            "units: mm\nhatch_patterns:\n  DASHES:\n    lines:\n      - {angle: 0, origin: [0, 0.5], delta: [0, 1], dashes: [1, -1]}\nlayers:\n  fill: {color: 4}\nobjects:\n  - line: {from: [0, 0, 0], to: [10, 0, 0]}\n  - hatch: {pattern: DASHES, boundary: [{polyline: {points: [[0, 0, 0], [10, 0, 0], [10, 10, 0], [0, 10, 0]]}}], layer: fill}\n",
        )
        .unwrap();
        let mut request = RenderRequest::new(&source);
        let whole = render_within(&request, &Settings::default(), 50).unwrap();
        assert_eq!(whole.hatches_cut_short, Vec::new());
        request.overwrite = true;
        let cut = render_within(&request, &Settings::default(), 20).unwrap();
        assert_eq!(
            cut.hatches_cut_short.len(),
            1,
            "{:?}",
            cut.hatches_cut_short
        );
        let hatch = &cut.hatches_cut_short[0];
        assert_eq!(
            (hatch.object, hatch.pattern.as_str(), hatch.layer.as_str()),
            (1, "DASHES", "fill")
        );
        assert!(hatch.why.contains("20 dashes"), "{}", hatch.why);
        let _ = fs::remove_dir_all(&dir);
    }
}
