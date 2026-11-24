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

use crate::description::{
    read_rows_files_in, read_set_settings, rows_of, template_at, template_path, ReadFiles,
};
use crate::draw::{self, Bounds, HatchCutShort, PageFrom};
use crate::error::{Error, ErrorCode};
use crate::leftovers::{leftovers, scan_error, Leftover, SetOutputs};
use crate::model::Sheet;
use crate::output::{
    object_counts, read_record, refuse_to_replace, write_whole, OutputKind, Record,
};
use crate::pdf::{self, About, Page, POINTS_PER_MILLIMETRE};
use crate::plot_style::{NotFollowed, PlotStyle};
use crate::reply::Reply;
use crate::set::{set_in_mark, Notice, SET_SETTINGS_FILE};
use crate::settings::Settings;
use crate::stamp::{SourceKind, Stamp};
use crate::text::fonts::{file_in, FontSubstitution};
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// What the reply calls the plot style a plot draws with when none is
/// named.
pub const BUILT_IN_PLOT_STYLE: &str = "monochrome (built in)";

/// How far a curve's chords may stray from it, in millimetres of paper:
/// finer than any plotter draws. A render through a plot style draws its
/// curves to it too, so a linetype repeats as often round them as in the
/// PDF.
pub(crate) const CHORD_TOLERANCE_MM: f64 = 0.005;

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
    /// Move the set's leftover outputs into `superseded/` in its output
    /// folder (true), or only list them (false). Without it, as the set's
    /// settings say under `clean`, as for a build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clean: Option<bool>,
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
    /// What the page is: the paper the sheet declares, or, when it declares
    /// none, its extents.
    pub page: PageFrom,
    /// The plot scale: paper length over drawing length. Always 1.
    pub scale: f64,
    /// The drawing's unit; none when a DWG has none the core knows, and it
    /// is plotted as millimetres.
    pub units: Option<Unit>,
    /// The box round everything drawn, in drawing units; none when a sheet
    /// that declares its paper draws nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extents: Option<Bounds>,
    /// The plot style the PDF was drawn through.
    pub plot_style: PlotStyleUsed,
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
    /// External references that could not be found or loaded. Left out when
    /// every reference resolved cleanly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_xrefs: Vec<crate::model::UnresolvedXref>,
    /// The stamp written into the PDF.
    pub stamp: Stamp,
    /// On a sheet drawn in several layouts, each layout's page, in the
    /// order of their tabs, which is the PDF's order: the first's is the
    /// one `sheet_size`, `page` and `extents` give.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layouts: Vec<PlottedLayout>,
}

/// One page of a plot of a sheet drawn in several layouts: one layout.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlottedLayout {
    /// The layout's name.
    pub layout: String,
    /// The page's size, the layout's paper.
    pub sheet_size: SheetSize,
    /// What the page covers.
    pub page: PageFrom,
    /// The box round everything the page draws, in drawing units.
    pub extents: Option<Bounds>,
    /// How many lines of searchable text the page holds.
    pub text_lines: usize,
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
    /// The set's inconsistent mark, when a change set landed on some of its
    /// sheets and failed on others: the PDFs plot the sheets as they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inconsistent: Option<crate::change::Inconsistent>,
    /// The DWGs and PDFs AgentDrafter made for the set in its output folder
    /// that no row writes any more, each moved into `superseded/` when the
    /// run cleaned, and any no row writes that could not be read to tell,
    /// marked `unreadable`. Empty when there are none; left out when the
    /// output folder could not be listed, which `leftovers_error` says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leftovers: Option<Vec<Leftover>>,
    /// Why the output folder could not be listed to look for leftovers,
    /// when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leftovers_error: Option<Error>,
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
    let explicit_style = request
        .plot_style
        .as_deref()
        .map(|name| plot_style(Some(name), settings))
        .transpose()?;
    let source = request.source.as_path();
    let record = read_record(source)?;
    let style = match explicit_style {
        Some(style) => style,
        None => plot_style_for_sheet(&record.sheet, settings)?,
    };
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
        Ok(report) => {
            match scan_error(report.leftovers_error.as_ref(), report.leftovers.as_deref()) {
                Some(error) => Reply::partly(report, error),
                None => Reply::from(Ok(report)),
            }
        }
        Err(error) => Reply::from(Err(error)),
    }
}

fn plot_sheets(request: &PlotSetRequest, settings: &Settings) -> Result<PlotSetReport, Error> {
    let folder = request.set.as_path();
    let set = read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    let rows_files = read_rows_files_in(&PlotFiles, folder, &set)?;
    let rows = rows_of(&rows_files)?;
    let chosen_style = request.plot_style.as_deref().or(set.plot_style.as_deref());
    let set_style = chosen_style
        .map(|name| plot_style(Some(name), settings))
        .transpose()?;
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
            let template = template_path(folder, row);
            let pdf = output.join(format!("{}.pdf", row.sheet));
            let mut notices = Vec::new();
            let plotted = read_text(&template, "the template").and_then(|(bytes, text)| {
                let (read, library) = template_at(&text, &template, row, set.units, &mut notices)?;
                let mut sheet = read.sheet;
                let sheet_style = match &set_style {
                    Some(style) => std::borrow::Cow::Borrowed(style),
                    None => std::borrow::Cow::Owned(plot_style_for_sheet(&sheet, settings)?),
                };
                let rows_file = &rows_files[row.file_index];
                let mut files = vec![
                    (SourceKind::Template, template.clone(), bytes),
                    (
                        SourceKind::Rows,
                        rows_file.path.clone(),
                        rows_file.text.clone().into_bytes(),
                    ),
                ];
                if let Some(library) = library {
                    files.push((SourceKind::BlockLibrary, library.path, library.bytes));
                }
                let resolved_xrefs = crate::xref::resolve_sheet_xrefs(&mut sheet, &template);
                files.extend(resolved_xrefs.files);
                let record = Record {
                    units: Some(sheet.units),
                    sheet,
                    objects_not_drawn: 0,
                    not_drawn: Vec::new(),
                    ids: Vec::new(),
                    files,
                    unresolved_xrefs: resolved_xrefs.unresolved,
                };
                plot_record(
                    &record,
                    &template,
                    &pdf,
                    request.overwrite,
                    &row.sheet,
                    sheet_style.as_ref(),
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
    let marked = set_in_mark(folder, &set.output);
    let scan = leftovers(
        &SetOutputs {
            folder,
            output: &output,
            marked: marked.as_deref(),
            rows_files: &rows_files,
            sheets: rows.iter().map(|row| row.sheet.as_str()).collect(),
        },
        request.clean.unwrap_or(set.clean),
    );
    Ok(PlotSetReport {
        set: folder.display().to_string(),
        units: set.units,
        output: output.display().to_string(),
        plotted,
        failed: sheets.len() - plotted,
        sheets,
        inconsistent: crate::change::inconsistent(folder),
        leftovers: scan.leftovers,
        leftovers_error: scan.error,
    })
}

/// The bytes of the file at `path` and its text. `what` names it in the
/// error, such as "the template".
/// The files as a set's plot reads its rows: as the disk holds them, with
/// a file that is not UTF-8 text refused in the words [`read_text`] refuses
/// the plot's other files in.
struct PlotFiles;

impl ReadFiles for PlotFiles {
    fn text(&self, path: &Path) -> std::io::Result<String> {
        String::from_utf8(fs::read(path)?)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, NOT_UTF8))
    }
}

/// Why the plot refuses a file it reads as text that is not UTF-8.
const NOT_UTF8: &str = "it is not UTF-8 text";

fn read_text(path: &Path, what: &str) -> Result<(Vec<u8>, String), Error> {
    let bytes = fs::read(path).map_err(|e| {
        Error::new(ErrorCode::ReadFailed, format!("could not read {what}: {e}")).in_file(path)
    })?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read {what}: {NOT_UTF8}"),
        )
        .in_file(path)
    })?;
    Ok((bytes, text))
}

/// Resolves the plot style from `sheet`'s effective page setup (when no
/// explicit `--plot-style` or `set.yaml` `plot_style` was supplied). When
/// the style came from a DWG's stored `Layout.plot_style_sheet` and is not
/// found in the configured plot style folders, falls back to built-in
/// monochrome so DWGs with AutoCAD's default `monochrome.ctb` still plot
/// on machines without AutoCAD's `Plot Styles` folder.
pub(crate) fn plot_style_for_sheet(
    sheet: &Sheet,
    settings: &Settings,
) -> Result<(PlotStyle, PlotStyleUsed), Error> {
    let Some(ps) = sheet.effective_page_setup() else {
        return plot_style(None, settings);
    };
    let Some(name) = ps.plot_style.as_deref() else {
        return plot_style(None, settings);
    };
    if ps.plot_style_from_dwg {
        if !crate::description::page_setup::is_plot_style_file(name) {
            return plot_style(None, settings);
        }
        let found = settings
            .plot_style_folders
            .iter()
            .any(|folder| file_in(folder, name).is_some());
        if !found {
            return plot_style(None, settings);
        }
    }
    plot_style(Some(name), settings)
}

/// The plot style `name` names (a CTB or STB file), read from the plot
/// style folders, or the built-in monochrome style when there is no name.
/// A render through a plot style finds it here too.
pub(crate) fn plot_style(
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
    // Lower-cased whole, so a name of any characters is checked without
    // cutting one in two.
    let plain = crate::description::page_setup::is_plot_style_file(name);
    if !plain {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "a plot style is named by its CTB or STB file's name, such as `monochrome.ctb` \
                 or `plant.stb`, and found in the plot style folders; `{name}` is not such a name"
            ),
        )
        .at_field("plot_style"));
    }
    let expect_stb = name.to_ascii_lowercase().ends_with(".stb");
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
    let style = PlotStyle::parse(&bytes, Some(expect_stb)).map_err(|why| {
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

/// Plots a DWG already in memory (such as a revision DWG just serialized) to
/// PDF bytes stamped with `source` (the revision DWG's path), returning both
/// the PDF bytes and its [`PlotReport`] without writing any file.
pub(crate) fn plot_dwg_bytes(
    source: &Path,
    bytes: &[u8],
    output: &Path,
    style: &(PlotStyle, PlotStyleUsed),
    settings: &Settings,
) -> Result<(Vec<u8>, PlotReport), Error> {
    let drawing = crate::dwg::read_bytes(bytes, source)?;
    let mut sheet = Sheet {
        units: drawing.units.unwrap_or(Unit::Mm),
        block_library: None,
        linetypes: drawing.linetypes,
        layers: drawing.layers,
        text_styles: drawing.text_styles,
        table_styles: drawing.table_styles,
        mleader_styles: drawing.mleader_styles,
        dimension_styles: drawing.dimension_styles,
        blocks: drawing.blocks,
        xrefs: drawing.xrefs,
        resolved_xrefs: Vec::new(),
        objects: drawing.objects,
        paper: drawing.paper,
        page_setup: drawing.page_setup,
        ltscale: drawing.ltscale,
        layout: drawing.layout,
        other_layouts: drawing.other_layouts,
        wipeout_frame: drawing.wipeout_frame,
        custom_properties: drawing.custom_properties,
    };
    let resolved_xrefs = crate::xref::resolve_sheet_xrefs(&mut sheet, source);
    let mut files = vec![(SourceKind::of(source), source.to_path_buf(), bytes.to_vec())];
    files.extend(resolved_xrefs.files);
    let record = Record {
        sheet,
        units: drawing.units,
        objects_not_drawn: drawing.objects_not_read,
        not_drawn: drawing.not_read,
        ids: Vec::new(),
        files,
        unresolved_xrefs: resolved_xrefs.unresolved,
    };
    let title = source
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    render_pdf_record(&record, source, output, &title, style, settings)
}

/// Plots the sheet `record` holds, read from `source`, to a PDF at
/// `output`, written whole or not at all.
fn plot_record(
    record: &Record,
    source: &Path,
    output: &Path,
    overwrite: bool,
    title: &str,
    style: &(PlotStyle, PlotStyleUsed),
    settings: &Settings,
) -> Result<PlotReport, Error> {
    refuse_to_replace(source, output, overwrite, OutputKind::Pdf)?;
    let (bytes, report) = render_pdf_record(record, source, output, title, style, settings)?;
    write_whole(output, &bytes, OutputKind::Pdf)?;
    Ok(report)
}

fn render_pdf_record(
    record: &Record,
    source: &Path,
    output: &Path,
    title: &str,
    (style, used): &(PlotStyle, PlotStyleUsed),
    settings: &Settings,
) -> Result<(Vec<u8>, PlotReport), Error> {
    let sheet = &record.sheet;
    let fonts = SheetFonts::find(sheet, settings);
    let millimetres_per_unit = sheet.units.convert(1.0, Unit::Mm);
    // A sheet drawn in several layouts plots a page for each, in the order
    // of their tabs, each at its own paper.
    let pages = sheet.pages();
    let mut drawn = Vec::with_capacity(pages.len());
    for on in &pages {
        // On paper: a layer that does not plot is left out (Zach,
        // 2026-11-13).
        let list = draw::draw_sheet_for(
            on,
            &fonts,
            CHORD_TOLERANCE_MM / millimetres_per_unit,
            draw::MOST_HATCH_DASHES,
            draw::Purpose::Plot,
        );
        let page = page(on, &list)?;
        drawn.push((list, page));
    }
    let stamp = Stamp::new(&record.read(), output);
    let producer = format!("agentdrafter {}", env!("CARGO_PKG_VERSION"));
    let creator = format!("agentdrafter plot, plot style {}", used.name);
    let sheets: Vec<(&draw::DisplayList, Page)> =
        drawn.iter().map(|(list, page)| (list, page.0)).collect();
    let bytes = pdf::pdf_pages(
        &sheets,
        style,
        &About {
            title,
            producer: &producer,
            creator: &creator,
            stamp: &stamp.to_json(),
        },
    );
    let layouts: Vec<PlottedLayout> = if pages.len() > 1 {
        pages
            .iter()
            .zip(&drawn)
            .map(|(on, (list, page))| PlottedLayout {
                layout: on
                    .layout
                    .as_ref()
                    .map_or_else(String::new, |l| l.name.clone()),
                sheet_size: page.1,
                page: page.3,
                extents: list.extents(),
                text_lines: list.text.len(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let (list, page) = drawn.swap_remove(0);
    let report = PlotReport {
        source: source.display().to_string(),
        pdf: output.display().to_string(),
        sheet_size: page.1,
        page: page.3,
        scale: page.4,
        units: record.units,
        extents: list.extents(),
        plot_style: used.clone(),
        objects: object_counts(sheet),
        objects_not_drawn: record.objects_not_drawn,
        not_drawn: record.not_drawn.clone(),
        text_lines: list.text.len(),
        bytes: bytes.len(),
        font_substitutions: fonts.substitutions(),
        hatches_cut_short: drawn.iter().fold(
            list.hatches_cut_short.clone(),
            |mut all, (other, _)| {
                all.extend(other.hatches_cut_short.iter().cloned());
                all
            },
        ),
        unresolved_xrefs: record.unresolved_xrefs.clone(),
        stamp,
        layouts,
    };
    Ok((bytes, report))
}

/// The page `sheet`, drawn as `list`, plots on: obeying `sheet`'s effective
/// [`crate::model::PageSetup`] (paper, plot area `extents`/`window`/`layout`,
/// scale `fit` or ratio, centring, offset, margins and transparency plotting)
/// when present, or else the declared paper or extents at 1:1.
fn page(
    sheet: &Sheet,
    list: &draw::DisplayList,
) -> Result<(Page, SheetSize, Bounds, PageFrom, f64), Error> {
    let units = sheet.units;
    let Some((source_bounds, from)) = draw::page_of(sheet, list) else {
        return Err(Error::new(
            ErrorCode::BadValue,
            "the sheet draws nothing, so it has no size to plot: a sheet is as large as what it \
             draws, such as its template's trim line",
        ));
    };
    let mm = |points: f64| (points / POINTS_PER_MILLIMETRE * 1000.0).round() / 1000.0;
    let base_points_per_unit = units.convert(1.0, Unit::In) * 72.0;

    let Some(ps) = sheet.effective_page_setup() else {
        // Default 1:1 path with no page_setup: identical to legacy behaviour.
        let width = source_bounds.width() * base_points_per_unit;
        let height = source_bounds.height() * base_points_per_unit;
        check_page_bounds(width, height, mm)?;
        let size = SheetSize {
            width_mm: mm(width),
            height_mm: mm(height),
            width_points: f64::from(width as f32),
            height_points: f64::from(height as f32),
        };
        let page = Page {
            origin: source_bounds.min,
            points_per_unit: base_points_per_unit,
            width,
            height,
            clip_window: None,
            plot_transparency: true,
        };
        return Ok((page, size, source_bounds, from, 1.0));
    };

    let effective_paper = ps.paper.as_ref().or(sheet.paper.as_ref());
    let [left_mm, bottom_mm, right_mm, top_mm] = ps.margins_mm;
    let mm_per_unit = units.convert(1.0, Unit::Mm);
    let src_w_mm = source_bounds.width() * mm_per_unit;
    let src_h_mm = source_bounds.height() * mm_per_unit;

    let (paper_w_pt, paper_h_pt, paper_w_mm, paper_h_mm) = match effective_paper {
        Some(paper) => {
            let w_pt = paper.width * base_points_per_unit;
            let h_pt = paper.height * base_points_per_unit;
            (
                w_pt,
                h_pt,
                paper.width * mm_per_unit,
                paper.height * mm_per_unit,
            )
        }
        None => {
            let scale = match ps.scale {
                crate::model::PlotScale::Fit => {
                    return Err(Error::new(
                        ErrorCode::BadValue,
                        "plot scale `fit` needs a paper size to fit to; declare `paper` on the \
                         sheet or inside `page_setup`",
                    ));
                }
                crate::model::PlotScale::Ratio {
                    numerator,
                    denominator,
                } => numerator / denominator,
            };
            let w_pt = source_bounds.width() * base_points_per_unit * scale;
            let h_pt = source_bounds.height() * base_points_per_unit * scale;
            (w_pt, h_pt, src_w_mm * scale, src_h_mm * scale)
        }
    };

    let printable_w_mm = paper_w_mm - left_mm - right_mm;
    let printable_h_mm = paper_h_mm - bottom_mm - top_mm;
    if !(printable_w_mm > 0.0 && printable_h_mm > 0.0) {
        return Err(Error::new(
            ErrorCode::BadValue,
            "the page setup's margins leave no printable area on the paper",
        ));
    }

    let scale = match ps.scale {
        crate::model::PlotScale::Fit => {
            if !(src_w_mm > 0.0 || src_h_mm > 0.0) {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    "cannot fit a zero-size drawing area to the paper",
                ));
            }
            let sx = if src_w_mm > 0.0 {
                printable_w_mm / src_w_mm
            } else {
                f64::INFINITY
            };
            let sy = if src_h_mm > 0.0 {
                printable_h_mm / src_h_mm
            } else {
                f64::INFINITY
            };
            sx.min(sy)
        }
        crate::model::PlotScale::Ratio {
            numerator,
            denominator,
        } => numerator / denominator,
    };

    check_page_bounds(paper_w_pt, paper_h_pt, mm)?;
    let points_per_unit = base_points_per_unit * scale;
    let mm_per_unit_on_paper = mm_per_unit * scale;
    let scaled_w_mm = src_w_mm * scale;
    let scaled_h_mm = src_h_mm * scale;

    let (place_x_mm, place_y_mm) = if ps.center {
        (
            left_mm + (printable_w_mm - scaled_w_mm) / 2.0,
            bottom_mm + (printable_h_mm - scaled_h_mm) / 2.0,
        )
    } else {
        (
            left_mm + ps.offset[0] * mm_per_unit,
            bottom_mm + ps.offset[1] * mm_per_unit,
        )
    };

    let origin = [
        source_bounds.min[0] - place_x_mm / mm_per_unit_on_paper,
        source_bounds.min[1] - place_y_mm / mm_per_unit_on_paper,
    ];
    let has_margins = left_mm > 0.0 || bottom_mm > 0.0 || right_mm > 0.0 || top_mm > 0.0;
    let clip_window = if matches!(ps.area, Some(crate::model::PlotArea::Window { .. })) {
        Some(source_bounds)
    } else if has_margins {
        Some(Bounds {
            min: [
                origin[0] + left_mm / mm_per_unit_on_paper,
                origin[1] + bottom_mm / mm_per_unit_on_paper,
            ],
            max: [
                origin[0] + (paper_w_mm - right_mm) / mm_per_unit_on_paper,
                origin[1] + (paper_h_mm - top_mm) / mm_per_unit_on_paper,
            ],
        })
    } else {
        None
    };

    let size = SheetSize {
        width_mm: mm(paper_w_pt),
        height_mm: mm(paper_h_pt),
        width_points: f64::from(paper_w_pt as f32),
        height_points: f64::from(paper_h_pt as f32),
    };
    let page = Page {
        origin,
        points_per_unit,
        width: paper_w_pt,
        height: paper_h_pt,
        clip_window,
        plot_transparency: ps.plot_transparency,
    };
    Ok((page, size, source_bounds, from, scale))
}

fn check_page_bounds(width: f64, height: f64, mm: impl Fn(f64) -> f64) -> Result<(), Error> {
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
    Ok(())
}

#[cfg(test)]
mod tests;
