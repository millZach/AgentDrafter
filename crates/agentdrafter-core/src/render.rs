//! Render: drawing a sheet to a PNG, through the drawing pipeline that the
//! PDF and the viewer also use (ADR-0011).
//!
//! The sheet becomes a display list ([`crate::draw`]), the display list
//! becomes pixels on the CPU ([`crate::raster`]), and the PNG carries a
//! stamp of every file read to draw it: the record, and the block library
//! a sheet description names ([`crate::stamp`]). Nothing here needs a
//! window or a GPU.
//!
//! A render covers the paper the sheet declares, as a plot does, what lies
//! off it cut at its edge; a sheet that declares none is covered to its
//! extents with a margin. Either is drawn at a plot scale or an image size
//! the caller gives. Lineweights are widths on paper, so they draw at
//! the render's dots per inch whatever its scale, as AutoCAD's plot fitted
//! to paper keeps them.
//!
//! A render draws each object in its own colour unless it is asked to draw
//! through a plot style, to show what prints: a CTB found as a plot finds
//! it, or the built-in monochrome style a plot draws with when none is
//! named. The pens are then the plot's, applied by the rasteriser through
//! the same [`PlotStyle`] the PDF is written through, and a layer that does
//! not plot is left out, as it is on paper.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use crate::draw::HatchCutShort;
use crate::draw::{self, Bounds, PageFrom};
use crate::error::{Error, ErrorCode};
use crate::model::Sheet;
use crate::output::{
    object_counts, read_record, refuse_to_replace, write_whole, OutputKind, Record,
};
use crate::plot::{self, PlotStyleUsed};
use crate::plot_style::PlotStyle;
use crate::raster::{self, View};
use crate::settings::Settings;
use crate::stamp::Stamp;
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
    /// with the same name and a `.png` extension. A sheet drawn in several
    /// layouts writes one PNG for each, named after this one with `-` and
    /// the layout's name before the extension, such as
    /// `panel-Details.png`.
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
    /// The CTB plot style to draw through, to show what prints, by its file
    /// name, such as `monochrome.ctb`, found in the plot style folders as a
    /// plot finds it. Not given with `monochrome`. Without either, each
    /// object draws in its own colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_style: Option<String>,
    /// Draw through the built-in monochrome style, exactly as a plot draws
    /// with no plot style named: every pen black at its object's own
    /// lineweight. Not given with `plot_style`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub monochrome: bool,
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
            plot_style: None,
            monochrome: false,
        }
    }
}

/// What a render drew its pens through.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawnThrough {
    /// Each object in its own colour and lineweight, or its layer's: the
    /// render's default, `"colour"`.
    Colour,
    /// The built-in monochrome style, as a plot with no plot style named
    /// draws with: `"monochrome"`.
    Monochrome,
    /// A CTB plot style, with the file it was read from and the settings it
    /// sets that are not followed yet, as a plot reports it: `{"plot_style":
    /// {"name": ..., "file": ..., "not_followed": [...]}}`.
    PlotStyle(PlotStyleUsed),
}

/// What a render wrote.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RenderReport {
    /// The record that was rendered.
    pub source: String,
    /// The layout the PNG draws, on a sheet drawn in several, each of which
    /// is rendered to a PNG of its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
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
    /// What the image covers: the paper the sheet declares, with no margin,
    /// or, when it declares none, its extents with the margin round them.
    pub page: PageFrom,
    /// What the pens were drawn through: each object's own colour, the
    /// built-in monochrome style, or a CTB plot style.
    pub drawn_through: DrawnThrough,
    /// How many objects of each type were drawn, such as `{"line": 1}`.
    pub objects: BTreeMap<String, usize>,
    /// How many objects of a DWG were not drawn because the core has no
    /// model for them yet. Always 0 for a sheet description.
    pub objects_not_drawn: usize,
    /// The objects of a DWG not drawn because the core has no model for
    /// them yet, by kind, such as `ACAD_PROXY_ENTITY`, with the handle of
    /// each, as `read` names them. Left out when every object is drawn.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_drawn: Vec<crate::model::RecordsOfKind>,
    /// The size of the PNG in bytes.
    pub bytes: usize,
    /// The text styles whose fonts are not in the font folders, and the
    /// look-alike drawn in their place. Empty when every font was found.
    pub font_substitutions: Vec<FontSubstitution>,
    /// The hatches not drawn in full, their pattern needing more lines or
    /// dashes than one hatch draws. Empty when every hatch was drawn whole.
    pub hatches_cut_short: Vec<HatchCutShort>,
    /// External references that could not be found or loaded. Left out when
    /// every reference resolved cleanly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_xrefs: Vec<crate::model::UnresolvedXref>,
    /// Raster images or PDF underlays that could not be found or decoded. Left
    /// out when every attachment resolved cleanly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_media: Vec<crate::model::UnresolvedMedia>,
    /// The stamp written into the PNG.
    pub stamp: Stamp,
    /// On a sheet drawn in several layouts, what the render wrote for each
    /// layout after the first, in the order of their tabs: this report is
    /// the first's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub other_layouts: Vec<RenderReport>,
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
    let through = through(request, settings)?;
    let source = request.source.as_path();
    let mut record = read_record(source)?;
    if let Some(dir) = source.parent() {
        crate::text::fonts::resolve_sheet_shapes(&mut record.sheet, settings, &[dir]);
    } else {
        crate::text::fonts::resolve_sheet_shapes(&mut record.sheet, settings, &[]);
    }
    let output = request
        .output
        .clone()
        .unwrap_or_else(|| source.with_extension("png"));
    let pages = record.sheet.pages();
    // A sheet drawn in several layouts renders each to a PNG of its own.
    let several = pages.len() > 1;
    let outputs: Vec<PathBuf> = if several {
        pages
            .iter()
            .map(|page| {
                let name = page.layout.as_ref().map_or("", |l| l.name.as_str());
                layout_output(&output, name)
            })
            .collect()
    } else {
        vec![output]
    };
    for output in &outputs {
        refuse_to_replace(source, output, request.overwrite, OutputKind::Png)?;
    }
    let mut reports = pages
        .iter()
        .zip(&outputs)
        .map(|(page, output)| {
            let mut report = render_page(
                request,
                &record,
                page,
                output,
                settings,
                through.as_ref(),
                most_dashes,
            )?;
            if several {
                report.layout = page.layout.as_ref().map(|l| l.name.clone());
            }
            Ok(report)
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let mut first = reports.remove(0);
    first.other_layouts = reports;
    Ok(first)
}

/// The plot style a render draws through, with what the reply says of it:
/// none for each object's own colour.
fn through(
    request: &RenderRequest,
    settings: &Settings,
) -> Result<Option<(PlotStyle, DrawnThrough)>, Error> {
    if request.plot_style.is_none() && !request.monochrome {
        return Ok(None);
    }
    // Found, read and refused as a plot finds, reads and refuses it, so a
    // name that plots draws, and one that a plot refuses is refused alike.
    let (style, used) = plot::plot_style(request.plot_style.as_deref(), settings)?;
    let drawn = if request.monochrome {
        DrawnThrough::Monochrome
    } else {
        DrawnThrough::PlotStyle(used)
    };
    Ok(Some((style, drawn)))
}

/// Where a render of `output` writes the PNG of the layout named `layout`,
/// on a sheet drawn in several: beside it, its name followed by `-` and
/// the layout's, such as `panel-Details.png`. A layout's name holds none of
/// the characters a file name cannot.
pub(crate) fn layout_output(output: &Path, layout: &str) -> PathBuf {
    // Written into the path as given, so that its folders keep the
    // separator they were given with.
    let text = output.to_string_lossy();
    let extension = output.extension().map_or(0, |e| e.len() + 1);
    let (stem, extension) = text.split_at(text.len() - extension);
    PathBuf::from(format!("{stem}-{layout}{extension}"))
}

/// Renders the sheet `record` holds, as `sheet`, one of its layouts, prints
/// it, to a PNG at `output`, written whole or not at all, through the plot
/// style `through` holds, or with none in each object's own colour.
fn render_page(
    request: &RenderRequest,
    record: &Record,
    sheet: &Sheet,
    output: &Path,
    settings: &Settings,
    through: Option<&(PlotStyle, DrawnThrough)>,
    most_dashes: usize,
) -> Result<RenderReport, Error> {
    let source = request.source.as_path();
    let output = output.to_path_buf();
    let fonts = SheetFonts::find(sheet, settings);
    let pixels_per_millimetre = request.dpi / 25.4;
    // The paper is the page, as a plot's is: no margin round it.
    let margin = if sheet.paper.is_some() {
        0.0
    } else {
        request.margin * pixels_per_millimetre
    };
    let millimetres_per_unit = sheet.units.convert(1.0, Unit::Mm);
    // Through a plot style the render shows what prints, so a layer that
    // does not plot is left out, as a plot leaves it out.
    let purpose = if through.is_some() {
        draw::Purpose::Plot
    } else {
        draw::Purpose::Screen
    };
    let area = |list: &draw::DisplayList| {
        draw::page_of(sheet, list).map_or(
            Bounds {
                min: [0.0, 0.0],
                max: [0.0, 0.0],
            },
            |(page, _)| page,
        )
    };
    // A coarse list first, curves to within a drawing unit, for the scale;
    // then the list drawn to a quarter of a pixel, finer than the eye sees,
    // whose extents the image covers. Through a plot style the list is drawn
    // to the plot's own tolerance instead, so each curve is as long as in
    // the PDF and a linetype lays out as many dashes round it.
    let coarse = draw::draw_sheet_for(sheet, &fonts, 1.0, most_dashes, purpose);
    let (rough, _) = fit(
        request,
        area(&coarse),
        pixels_per_millimetre,
        millimetres_per_unit,
        margin,
    )?;
    let tolerance = if through.is_some() {
        plot::CHORD_TOLERANCE_MM / millimetres_per_unit
    } else {
        0.25 / rough
    };
    let list = draw::draw_sheet_for(sheet, &fonts, tolerance, most_dashes, purpose);
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

    let style = through.map(|(style, _)| style);
    let plot_transparency = if through.is_some() {
        sheet
            .effective_page_setup()
            .is_none_or(|ps| ps.plot_transparency)
    } else {
        true
    };
    let mut pixmap =
        raster::rasterize(&list, &view, style, plot_transparency).ok_or_else(|| {
            Error::new(
                ErrorCode::WriteFailed,
                format!(
                    "could not make an image of {} by {} pixels",
                    view.width, view.height
                ),
            )
        })?;
    // What lies off the paper is cut away, where the image is larger than
    // the paper it fits. Through a plot style, ink is cut at the plot's
    // page, its extents on a sheet with no paper, as the PDF's page cuts it,
    // and the margin round it stays white. A page with no width or no
    // height, such as the extents of one straight line, has nothing to cut
    // at, and a plot refuses it; the render draws it uncut.
    let rendered_page_from = draw::page_of(sheet, &list).map_or(PageFrom::Extents, |(_, f)| f);
    if sheet.paper.is_some() || sheet.effective_page_setup().is_some() || through.is_some() {
        if let Some((paper, _)) = draw::page_of(sheet, &list) {
            if paper.width() > 0.0 && paper.height() > 0.0 {
                raster::blank_outside(&mut pixmap, &view, paper);
            }
        }
    }
    let stamp = Stamp::new(&record.read(), &output);
    // The stamp holds no plot style, as a PDF's holds none; the PNG names
    // it as the PDF's `Creator` does, in its `Software` text chunk.
    let drawn_through = through.map_or(DrawnThrough::Colour, |(_, drawn)| drawn.clone());
    let software = match &drawn_through {
        DrawnThrough::Colour => None,
        DrawnThrough::Monochrome => Some(plot::BUILT_IN_PLOT_STYLE),
        DrawnThrough::PlotStyle(used) => Some(used.name.as_str()),
    }
    .map(|name| format!("agentdrafter render, plot style {name}"));
    let png = raster::png(
        &pixmap,
        &stamp.to_json(),
        software.as_deref(),
        pixels_per_millimetre,
    )
    .map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not encode the PNG: {e}"),
        )
        .in_file(&output)
    })?;
    write_whole(&output, &png, OutputKind::Png)?;

    Ok(RenderReport {
        source: source.display().to_string(),
        layout: None,
        png: output.display().to_string(),
        width: view.width,
        height: view.height,
        dpi: request.dpi,
        scale: pixels_per_unit / pixels_per_millimetre / millimetres_per_unit,
        pixels_per_unit,
        top_left: view.top_left,
        units: record.units,
        extents,
        page: rendered_page_from,
        drawn_through,
        objects: object_counts(sheet),
        objects_not_drawn: record.objects_not_drawn,
        not_drawn: record.not_drawn.clone(),
        bytes: png.len(),
        font_substitutions: fonts.substitutions(),
        hatches_cut_short: list.hatches_cut_short.clone(),
        unresolved_xrefs: record.unresolved_xrefs.clone(),
        unresolved_media: record.unresolved_media.clone(),
        stamp,
        other_layouts: Vec::new(),
    })
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
    if request.monochrome && request.plot_style.is_some() {
        return Err(Error::usage(
            "draw through a plot style (plot_style) or monochrome, not both: monochrome is the \
             built-in style a plot draws with when no plot style is named",
        )
        .at_field("monochrome"));
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

#[cfg(test)]
mod tests {
    use std::fs;

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
