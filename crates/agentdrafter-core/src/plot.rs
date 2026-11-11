//! Plot: drawing a sheet to a PDF at its sheet size, through a plot style,
//! as AutoCAD plots the same DWG to PDF, for issue.
//!
//! The sheet becomes a display list ([`crate::draw`]), the same one the PNG
//! draws from (ADR-0011), and the display list becomes a PDF
//! ([`crate::pdf`]) through the pens of a plot style
//! ([`crate::plot_style`]).
//!
//! **The sheet size.** A sheet is as large as what it draws: its template
//! draws its trim line at the paper's edge, and the usual case is one DWG
//! per sheet, drawn in model space and plotted to extents at 1:1. So the
//! page is the drawing's extents, the box round everything it draws as
//! AutoCAD's extents are, at 1:1 in the drawing's unit: a sheet whose trim
//! line is drawn at 17 by 11 inches plots on a page of exactly 17 by 11
//! inches, 431.8 by 279.4 mm, 1224 by 792 points. A DWG with no unit is
//! taken to be in millimetres.
//!
//! **The plot style.** A CTB file, found by its name in the plot style
//! folders, sets each pen's colour and lineweight. With none named, the
//! built-in monochrome style plots every pen black at its object's own
//! lineweight, as AutoCAD's `monochrome.ctb` does.
//!
//! Like every output, the PDF is stamped with every file read to draw it
//! and is written whole or not at all.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::description::{read_set_settings, rows_at, template_at};
use crate::draw::{self, Bounds, HatchCutShort};
use crate::error::{Error, ErrorCode};
use crate::output::{
    object_counts, read_record, refuse_to_replace, write_whole, OutputKind, Record,
};
use crate::pdf::{self, About, Page, POINTS_PER_MILLIMETRE};
use crate::plot_style::{NotFollowed, PlotStyle};
use crate::reply::Reply;
use crate::set::{Notice, SET_SETTINGS_FILE};
use crate::settings::Settings;
use crate::stamp::{SourceKind, Stamp};
use crate::text::fonts::{file_in, FontSubstitution};
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// What the reply calls the plot style a plot draws with when none is
/// named.
pub const BUILT_IN_PLOT_STYLE: &str = "monochrome (built in)";

/// How far a curve's chords may stray from it, in millimetres of paper:
/// finer than any plotter draws.
const CHORD_TOLERANCE_MM: f64 = 0.005;

/// The largest page a PDF reader opens: 200 inches a side.
pub const MOST_PAGE_POINTS: f64 = 14_400.0;

/// A request to plot one sheet to a PDF.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlotRequest {
    /// The record to plot: a sheet description, or a DWG when the file ends
    /// `.dwg`.
    pub source: PathBuf,
    /// Where to write the PDF. Without it, the PDF goes beside the source
    /// with the same name and a `.pdf` extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    /// Replace the file at the output path, which is allowed only when it
    /// is a PDF AgentDrafter plotted. Without it, an existing file is never
    /// touched.
    #[serde(default)]
    pub overwrite: bool,
    /// The CTB plot style to plot through, by its file name, such as
    /// `monochrome.ctb`, found in the plot style folders. Without it, the
    /// built-in monochrome style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_style: Option<String>,
}

impl PlotRequest {
    /// A request to plot `source` beside itself through the built-in
    /// monochrome style.
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            output: None,
            overwrite: false,
            plot_style: None,
        }
    }
}

/// A request to plot every sheet of a drawing set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlotSetRequest {
    /// The drawing set's folder, which holds its settings in
    /// [`SET_SETTINGS_FILE`].
    pub set: PathBuf,
    /// Replace each sheet's PDF that is already there, which is allowed
    /// only when it is a PDF AgentDrafter plotted.
    #[serde(default)]
    pub overwrite: bool,
    /// The CTB plot style to plot through, as for one sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_style: Option<String>,
}

/// The size of a plotted sheet: the PDF's page.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SheetSize {
    /// Across, in millimetres, to the micrometre.
    pub width_mm: f64,
    /// Down, in millimetres, to the micrometre.
    pub height_mm: f64,
    /// Across, in points (72 to the inch), as the PDF's page box gives it.
    pub width_points: f64,
    /// Down, in points.
    pub height_points: f64,
}

/// The plot style a plot drew with.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlotStyleUsed {
    /// Its name: the CTB file's, or [`BUILT_IN_PLOT_STYLE`].
    pub name: String,
    /// Where the CTB file was read from; none for the built-in style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The pen settings it sets that the plot does not follow yet, such as
    /// screening, each with the colour indexes whose pens set it. Empty
    /// when every setting was followed.
    pub not_followed: Vec<NotFollowed>,
}

/// What a plot wrote.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlotReport {
    /// The record that was plotted, or for a sheet of a drawing set, its
    /// template.
    pub source: String,
    /// The PDF that was written.
    pub pdf: String,
    /// The sheet's size, which is the PDF's page.
    pub sheet_size: SheetSize,
    /// The plot scale: paper length over drawing length. Always 1.
    pub scale: f64,
    /// The drawing's unit; none when a DWG has none the core knows, and it
    /// is plotted as millimetres.
    pub units: Option<Unit>,
    /// The box round everything drawn, in drawing units: the sheet.
    pub extents: Bounds,
    /// The plot style the PDF was drawn through.
    pub plot_style: PlotStyleUsed,
    /// How many objects of each type were drawn, such as `{"line": 1}`.
    pub objects: BTreeMap<String, usize>,
    /// How many objects of a DWG were not drawn because the core has no
    /// model for them yet. Always 0 for a sheet description.
    pub objects_not_drawn: usize,
    /// How many lines of text the PDF holds as searchable text.
    pub text_lines: usize,
    /// The size of the PDF in bytes.
    pub bytes: usize,
    /// The text styles whose fonts are not in the font folders, and the
    /// look-alike drawn in their place. Empty when every font was found.
    pub font_substitutions: Vec<FontSubstitution>,
    /// The hatches not drawn in full. Empty when every hatch was drawn
    /// whole.
    pub hatches_cut_short: Vec<HatchCutShort>,
    /// The stamp written into the PDF.
    pub stamp: Stamp,
}

/// What a drawing set's plot did.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlotSetReport {
    /// The drawing set's folder.
    pub set: String,
    /// The set's declared unit.
    pub units: Unit,
    /// The folder the set's outputs go to, which its PDFs are written in.
    pub output: String,
    /// How many sheets were plotted.
    pub plotted: usize,
    /// How many sheets failed. Nothing was written for them.
    pub failed: usize,
    /// Each sheet, in the order its rows give them.
    pub sheets: Vec<PlottedSheet>,
}

/// What became of one sheet of a drawing set's plot.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlottedSheet {
    /// The sheet's name, as its row gives it.
    pub sheet: String,
    /// The sheet's template, as its row writes its path.
    pub template: String,
    /// Whether the sheet was plotted.
    pub ok: bool,
    /// What the sheet's plot wrote, when it was plotted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<PlotReport>,
    /// Why the sheet was not plotted, when it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
    /// What reading the sheet noticed that did not stop it, as a build
    /// notices it. Empty when there is nothing to say.
    pub notices: Vec<Notice>,
}

/// Plots a sheet to a PDF, reading fonts and plot styles from the folders
/// the environment names ([`Settings::from_env`]).
pub fn plot(request: &PlotRequest) -> Result<PlotReport, Error> {
    plot_with(request, &Settings::from_env())
}

/// Plots a sheet description or a DWG to a PDF at its sheet size, reading
/// fonts and plot styles from the folders `settings` names. The PDF is
/// written whole or not at all.
pub fn plot_with(request: &PlotRequest, settings: &Settings) -> Result<PlotReport, Error> {
    let style = plot_style(request.plot_style.as_deref(), settings)?;
    let source = request.source.as_path();
    let record = read_record(source)?;
    let output = request
        .output
        .clone()
        .unwrap_or_else(|| source.with_extension("pdf"));
    let title = source
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    plot_record(
        &record,
        source,
        &output,
        request.overwrite,
        &title,
        &style,
        settings,
    )
}

/// Plots every sheet of a drawing set, reading fonts and plot styles from
/// the folders the environment names. See [`plot_set_with`].
pub fn plot_set(request: &PlotSetRequest) -> Reply<PlotSetReport> {
    plot_set_with(request, &Settings::from_env())
}

/// Plots every sheet of a drawing set to a PDF, each its template filled in
/// from its row, as a build reads it, into `<output>/<sheet>.pdf` in the
/// set's output folder. Each PDF is stamped with the template, the rows
/// file and the block library it was drawn from.
///
/// As for a build, a sheet that fails writes nothing and the others are
/// still plotted: `ok` with the report when every sheet was plotted; not
/// `ok`, with the report and a `sheets_failed` error, when some failed; and
/// not `ok`, with only an error, when the set's settings, its rows or the
/// plot style could not be read and nothing was plotted.
pub fn plot_set_with(request: &PlotSetRequest, settings: &Settings) -> Reply<PlotSetReport> {
    match plot_sheets(request, settings) {
        Ok(report) if report.failed > 0 => {
            let error = Error::new(
                ErrorCode::SheetsFailed,
                format!(
                    "{} of the set's {} sheets failed, and nothing was written for them; {} \
                     plotted. Each sheet's outcome is under `sheets`",
                    report.failed,
                    report.sheets.len(),
                    report.plotted
                ),
            );
            Reply::partly(report, error)
        }
        outcome => Reply::from(outcome),
    }
}

fn plot_sheets(request: &PlotSetRequest, settings: &Settings) -> Result<PlotSetReport, Error> {
    let folder = request.set.as_path();
    let set = read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let rows_file = folder.join(&set.rows);
    let (rows_bytes, rows_text) = read_text(&rows_file, "the drawing set's rows")?;
    let rows = rows_at(&rows_text, &rows_file)?;
    let style = plot_style(request.plot_style.as_deref(), settings)?;
    let output = folder.join(&set.output);
    fs::create_dir_all(&output).map_err(|e| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not make the drawing set's output folder: {e}"),
        )
        .in_file(&output)
    })?;
    let sheets: Vec<PlottedSheet> = rows
        .iter()
        .map(|row| {
            let template = folder.join(&row.template);
            let pdf = output.join(format!("{}.pdf", row.sheet));
            let mut notices = Vec::new();
            let plotted = read_text(&template, "the template").and_then(|(bytes, text)| {
                let (read, library) =
                    template_at(&text, &template, row, &rows_file, set.units, &mut notices)?;
                let sheet = read.sheet;
                let mut files = vec![
                    (SourceKind::Template, template.clone(), bytes),
                    (SourceKind::Rows, rows_file.clone(), rows_bytes.clone()),
                ];
                if let Some(library) = library {
                    files.push((SourceKind::BlockLibrary, library.path, library.bytes));
                }
                let record = Record {
                    units: Some(sheet.units),
                    sheet,
                    objects_not_drawn: 0,
                    files,
                };
                plot_record(
                    &record,
                    &template,
                    &pdf,
                    request.overwrite,
                    &row.sheet,
                    &style,
                    settings,
                )
            });
            let (result, error) = match plotted {
                Ok(report) => (Some(report), None),
                Err(error) => (None, Some(error)),
            };
            PlottedSheet {
                sheet: row.sheet.clone(),
                template: row.template.clone(),
                ok: result.is_some(),
                result,
                error,
                notices,
            }
        })
        .collect();
    let plotted = sheets.iter().filter(|s| s.ok).count();
    Ok(PlotSetReport {
        set: folder.display().to_string(),
        units: set.units,
        output: output.display().to_string(),
        plotted,
        failed: sheets.len() - plotted,
        sheets,
    })
}

/// The bytes of the file at `path` and its text. `what` names it in the
/// error, such as "the template".
fn read_text(path: &Path, what: &str) -> Result<(Vec<u8>, String), Error> {
    let bytes = fs::read(path).map_err(|e| {
        Error::new(ErrorCode::ReadFailed, format!("could not read {what}: {e}")).in_file(path)
    })?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read {what}: it is not UTF-8 text"),
        )
        .in_file(path)
    })?;
    Ok((bytes, text))
}

/// The plot style `name` names, read from the plot style folders, or the
/// built-in monochrome style when there is no name.
fn plot_style(
    name: Option<&str>,
    settings: &Settings,
) -> Result<(PlotStyle, PlotStyleUsed), Error> {
    let Some(name) = name else {
        let used = PlotStyleUsed {
            name: BUILT_IN_PLOT_STYLE.to_owned(),
            file: None,
            not_followed: Vec::new(),
        };
        return Ok((PlotStyle::monochrome(), used));
    };
    let plain = !name.is_empty()
        && !name.contains(['/', '\\', ':'])
        && name.len() > 4
        && name[name.len() - 4..].eq_ignore_ascii_case(".ctb");
    if !plain {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "a plot style is named by its CTB file's name, such as `monochrome.ctb`, and \
                 found in the plot style folders; `{name}` is not such a name"
            ),
        )
        .at_field("plot_style"));
    }
    let folders = &settings.plot_style_folders;
    let Some(path) = folders.iter().find_map(|folder| file_in(folder, name)) else {
        let message = if folders.is_empty() {
            format!(
                "no plot style folder is set, so the plot style `{name}` cannot be found; name \
                 the folder that holds it, such as AutoCAD's `Plot Styles` folder, with \
                 --plot-styles or the AGENTDRAFTER_PLOT_STYLES environment variable"
            )
        } else {
            let listed: Vec<String> = folders.iter().map(|f| f.display().to_string()).collect();
            format!(
                "the plot style `{name}` is in none of the plot style folders: {}",
                listed.join(", ")
            )
        };
        return Err(Error::new(ErrorCode::ReadFailed, message).at_field("plot_style"));
    };
    let bytes = fs::read(&path).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the plot style `{name}`: {e}"),
        )
        .in_file(&path)
    })?;
    let style = PlotStyle::parse_ctb(&bytes).map_err(|why| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the plot style `{name}`: {why}"),
        )
        .in_file(&path)
    })?;
    let used = PlotStyleUsed {
        name: name.to_owned(),
        file: Some(path.display().to_string()),
        not_followed: style.not_followed.clone(),
    };
    Ok((style, used))
}

/// Plots the sheet `record` holds, read from `source`, to a PDF at
/// `output`, written whole or not at all.
fn plot_record(
    record: &Record,
    source: &Path,
    output: &Path,
    overwrite: bool,
    title: &str,
    (style, used): &(PlotStyle, PlotStyleUsed),
    settings: &Settings,
) -> Result<PlotReport, Error> {
    refuse_to_replace(source, output, overwrite, OutputKind::Pdf)?;
    let sheet = &record.sheet;
    let fonts = SheetFonts::find(sheet, settings);
    let millimetres_per_unit = sheet.units.convert(1.0, Unit::Mm);
    let list = draw::draw_sheet(
        sheet,
        &fonts,
        CHORD_TOLERANCE_MM / millimetres_per_unit,
        draw::MOST_HATCH_DASHES,
    );
    let page = page(&list, sheet.units)?;
    let stamp = Stamp::new(&record.read(), output);
    let producer = format!("agentdrafter {}", env!("CARGO_PKG_VERSION"));
    let creator = format!("agentdrafter plot, plot style {}", used.name);
    let bytes = pdf::pdf(
        &list,
        &page.0,
        style,
        &About {
            title,
            producer: &producer,
            creator: &creator,
            stamp: &stamp.to_json(),
        },
    );
    write_whole(output, &bytes, OutputKind::Pdf)?;
    Ok(PlotReport {
        source: source.display().to_string(),
        pdf: output.display().to_string(),
        sheet_size: page.1,
        scale: 1.0,
        units: record.units,
        extents: page.2,
        plot_style: used.clone(),
        objects: object_counts(sheet),
        objects_not_drawn: record.objects_not_drawn,
        text_lines: list.text.len(),
        bytes: bytes.len(),
        font_substitutions: fonts.substitutions(),
        hatches_cut_short: list.hatches_cut_short.clone(),
        stamp,
    })
}

/// The page a display list plots on: its extents at 1:1 in `units`, with
/// the sheet's size and the extents. A sheet that draws nothing, or nothing
/// with both a width and a height, has no page.
fn page(list: &draw::DisplayList, units: Unit) -> Result<(Page, SheetSize, Bounds), Error> {
    let Some(extents) = list.extents() else {
        return Err(Error::new(
            ErrorCode::BadValue,
            "the sheet draws nothing, so it has no size to plot: a sheet is as large as what it \
             draws, such as its template's trim line",
        ));
    };
    // Inches are exact in points, so a sheet drawn in inches plots on a page
    // of exactly its size.
    let points_per_unit = units.convert(1.0, Unit::In) * 72.0;
    let width = extents.width() * points_per_unit;
    let height = extents.height() * points_per_unit;
    let mm = |points: f64| (points / POINTS_PER_MILLIMETRE * 1000.0).round() / 1000.0;
    if !(width > 0.0 && height > 0.0) {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "the sheet's extents are {} by {} mm at 1:1, and a page needs a width and a \
                 height: a sheet is as large as what it draws, such as its template's trim line",
                mm(width),
                mm(height)
            ),
        ));
    }
    if width > MOST_PAGE_POINTS || height > MOST_PAGE_POINTS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "the sheet's extents are {} by {} mm at 1:1, and a PDF page is at most 200 \
                 inches (5080 mm) a side, as PDF readers allow; a sheet is as large as what it \
                 draws, so check for objects drawn far outside its border",
                mm(width),
                mm(height)
            ),
        ));
    }
    let size = SheetSize {
        width_mm: mm(width),
        height_mm: mm(height),
        width_points: f64::from(width as f32),
        height_points: f64::from(height as f32),
    };
    let page = Page {
        origin: extents.min,
        points_per_unit,
        width,
        height,
    };
    Ok((page, size, extents))
}

#[cfg(test)]
mod tests;
