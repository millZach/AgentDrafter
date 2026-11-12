//! A sheet's display list, as plain data: what the viewer draws, through
//! the one drawing pipeline the PNG and the PDF draw through (ADR-0011).
//!
//! The engine stays on the machine that holds the drawings: it reads the
//! record and lays the sheet out, text in the fonts of that machine's font
//! folders, and sends the display list to the page, where the same Rust
//! code, compiled to WebAssembly, turns it into pixels. Nothing here writes
//! a file.
//!
//! The reply also says what each of the sheet's objects is, by its place:
//! its ID, name, type and layer, and an insert's block and attributes, so
//! the page can show the object the reviewer picks from what the core's
//! pick finds in the display list. A drawing set can be drawn as it was at
//! an entry of its journal, rebuilt in memory from the undo states.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use std::sync::Arc;

use crate::change::FilesAsOf;
use crate::description::{read_sheet_record, SavedSet};
use crate::draw::{self, Bounds, DisplayList, HatchCutShort};
use crate::error::{Error, ErrorCode};
use crate::model::{Attribute, Object, Paper, Sheet};
use crate::output::read_record;
use crate::settings::Settings;
use crate::text::fonts::FontSubstitution;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// How finely a display list's curves are drawn unless asked otherwise:
/// within this share of the sheet's larger side, a quarter of a pixel when
/// the sheet spans 25,000 pixels across, as the viewer's closest zoom does.
pub const DISPLAY_LIST_TOLERANCE: f64 = 1e-5;

/// A request for a sheet's display list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayListRequest {
    /// The record to draw: a sheet description, a DWG when the file ends
    /// `.dwg`, or a drawing set's folder.
    pub source: PathBuf,
    /// Which of a drawing set's sheets to draw, by name, in any case.
    /// Without it, the set's first sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    /// How far, in drawing units, a curve's chords may stray from it.
    /// Without it, [`DISPLAY_LIST_TOLERANCE`] of the sheet's larger side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<f64>,
    /// Draw a drawing set as it was just after this entry of its journal,
    /// rebuilt in memory from the undo states. Without it, as it is now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<u64>,
}

impl DisplayListRequest {
    /// A request for the first sheet of `source`, at the default tolerance.
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            sheet: None,
            tolerance: None,
            entry: None,
        }
    }
}

/// What one of the sheet's objects is, for the reviewer who picks it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetObject {
    /// Its ID, such as `DEMO-001/border`; none for an object of a DWG,
    /// whose IDs the core does not read yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Its name, if it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its type, such as `insert`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The layer it sits on.
    pub layer: String,
    /// The block an insert places.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
    /// An insert's attributes, each its tag and value, in the order its
    /// block defines them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<Attribute>,
}

/// A sheet's display list, with what the viewer needs to show it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DisplayListReport {
    /// The record drawn: the sheet description, the DWG or the drawing
    /// set's folder.
    pub source: String,
    /// What the record is called, for a heading: the drawing set's folder's
    /// own name, such as `demo-plant` for `.` when the folder is the
    /// set's, or the sheet description's or DWG's file name.
    pub name: String,
    /// The sheet drawn.
    pub sheet: String,
    /// Every sheet of the drawing set, in its order; the one sheet of a
    /// sheet description or a DWG.
    pub sheets: Vec<String>,
    /// The drawing's unit; none when a DWG has none the core knows, and it
    /// is drawn as millimetres.
    pub units: Option<Unit>,
    /// How many millimetres of paper one drawing unit spans at 1:1, so a
    /// lineweight, in millimetres, can be drawn at a zoom given in drawing
    /// units.
    pub millimetres_per_unit: f64,
    /// The paper the sheet is drawn on, in drawing units with its lower
    /// left corner at the origin, when the record declares one.
    pub paper: Option<Paper>,
    /// The box round everything drawn, in drawing units; none when nothing
    /// draws.
    pub extents: Option<Bounds>,
    /// How far, in drawing units, the display list's chords stray from the
    /// curves at most.
    pub tolerance: f64,
    /// How many objects of a DWG were not drawn because the core has no
    /// model for them yet. Always 0 for a sheet description.
    pub objects_not_drawn: usize,
    /// The text styles whose fonts are not in the font folders, and the
    /// look-alike drawn in their place. Empty when every font was found.
    pub font_substitutions: Vec<FontSubstitution>,
    /// The hatches not drawn in full. Empty when every hatch was drawn
    /// whole.
    pub hatches_cut_short: Vec<HatchCutShort>,
    /// The journal's entry the set is drawn as it was just after, when it
    /// was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<u64>,
    /// Each of the sheet's objects, by its place, which each item of the
    /// display list names as its `object`.
    pub objects: Vec<SheetObject>,
    /// What the sheet draws.
    pub display_list: DisplayList,
}

/// Draws a sheet to its display list, reading fonts from the font folders
/// the environment names ([`Settings::from_env`]).
pub fn display_list(request: &DisplayListRequest) -> Result<DisplayListReport, Error> {
    display_list_with(request, &Settings::from_env())
}

/// Draws a sheet to its display list, as a render draws it before it
/// becomes pixels, reading fonts from the font folders `settings` names. A
/// drawing set's sheet is its template filled in from its row, as a plot
/// reads it. Nothing is written.
pub fn display_list_with(
    request: &DisplayListRequest,
    settings: &Settings,
) -> Result<DisplayListReport, Error> {
    if let Some(tolerance) = request.tolerance {
        if !(tolerance.is_finite() && tolerance > 0.0) {
            return Err(Error::new(
                ErrorCode::BadValue,
                "a tolerance is a length in drawing units, more than 0",
            )
            .at_field("tolerance"));
        }
    }
    let source = request.source.as_path();
    let drawn = read(source, request.sheet.as_deref(), request.entry)?;
    let sheet = &drawn.sheet;
    let fonts = SheetFonts::find(sheet, settings);
    let tolerance = match request.tolerance {
        Some(tolerance) => tolerance,
        None => {
            // A coarse list first, curves to within a drawing unit, for the
            // sheet's size.
            let coarse = draw::draw_sheet(sheet, &fonts, 1.0, draw::MOST_HATCH_DASHES);
            let size = coarse.extents().map_or(0.0, |b| b.width().max(b.height()));
            if size > 0.0 {
                size * DISPLAY_LIST_TOLERANCE
            } else {
                DISPLAY_LIST_TOLERANCE
            }
        }
    };
    let list = draw::draw_sheet(sheet, &fonts, tolerance, draw::MOST_HATCH_DASHES);
    // `.` and `..` have no name of their own: the folder's own is found.
    let own = |path: &Path| path.file_name().map(|n| n.to_string_lossy().into_owned());
    let name = own(source)
        .or_else(|| source.canonicalize().ok().and_then(|path| own(&path)))
        .unwrap_or_else(|| source.display().to_string());
    Ok(DisplayListReport {
        source: source.display().to_string(),
        name,
        sheet: drawn.name,
        sheets: drawn.names,
        units: drawn.units,
        millimetres_per_unit: sheet.units.convert(1.0, Unit::Mm),
        paper: sheet.paper.clone(),
        extents: list.extents(),
        tolerance,
        objects_not_drawn: drawn.objects_not_drawn,
        font_substitutions: fonts.substitutions(),
        hatches_cut_short: list.hatches_cut_short.clone(),
        entry: request.entry,
        objects: drawn.objects,
        display_list: list,
    })
}

/// The sheet to draw, read from its record.
struct Drawn {
    name: String,
    names: Vec<String>,
    sheet: Sheet,
    units: Option<Unit>,
    objects_not_drawn: usize,
    objects: Vec<SheetObject>,
}

/// What each of `sheet`'s objects is, with its ID and name when `ids`
/// gives them, by the same places.
fn objects_of(sheet: &Sheet, ids: &[(String, Option<String>)]) -> Vec<SheetObject> {
    sheet
        .objects
        .iter()
        .enumerate()
        .map(|(i, object)| {
            let (block, attributes) = match object {
                Object::Insert(insert) => (Some(insert.block.clone()), insert.attributes.clone()),
                _ => (None, Vec::new()),
            };
            SheetObject {
                id: ids.get(i).map(|(id, _)| id.clone()),
                name: ids.get(i).and_then(|(_, name)| name.clone()),
                kind: object.type_name().to_owned(),
                layer: object.layer().to_owned(),
                block,
                attributes,
            }
        })
        .collect()
}

/// Reads the sheet `sheet` names from `source`: a drawing set's folder, as
/// it was just after its journal's entry `entry` when one is given, or one
/// sheet description or DWG, whose one sheet is named by its file.
fn read(source: &Path, sheet: Option<&str>, entry: Option<u64>) -> Result<Drawn, Error> {
    let named = |name: &str| sheet.is_none_or(|s| s.to_lowercase() == name.to_lowercase());
    if source.is_dir() {
        let set = match entry {
            Some(entry) => SavedSet::read_from(source, Arc::new(FilesAsOf::read(source, entry)?))?,
            None => SavedSet::read(source)?,
        };
        let names: Vec<String> = set.names().into_iter().map(str::to_owned).collect();
        let Some(row) = set.rows.iter().find(|row| named(&row.sheet)) else {
            return Err(no_such_sheet(sheet, &names));
        };
        let record = set.sheet(row)?;
        let ids: Vec<(String, Option<String>)> = (0..record.objects.len())
            .map(|i| (record.id(i), record.objects[i].name.clone()))
            .collect();
        return Ok(Drawn {
            name: record.name,
            names,
            units: Some(record.sheet.units),
            objects: objects_of(&record.sheet, &ids),
            sheet: record.sheet,
            objects_not_drawn: 0,
        });
    }
    if entry.is_some() {
        return Err(Error::new(
            ErrorCode::BadValue,
            "a sheet is drawn as it was at an entry of the journal only from a drawing set's \
             folder, whose journal it is",
        )
        .at_field("entry"));
    }
    let name = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !named(&name) {
        return Err(no_such_sheet(sheet, std::slice::from_ref(&name)));
    }
    let record = read_record(source)?;
    let ids: Vec<(String, Option<String>)> = if record.units.is_some() && !is_dwg(source) {
        let described = read_sheet_record(source)?;
        (0..described.objects.len())
            .map(|i| (described.id(i), described.objects[i].name.clone()))
            .collect()
    } else {
        Vec::new()
    };
    Ok(Drawn {
        names: vec![name.clone()],
        name,
        objects: objects_of(&record.sheet, &ids),
        sheet: record.sheet,
        units: record.units,
        objects_not_drawn: record.objects_not_drawn,
    })
}

/// Whether `source` is read as a DWG: its name ends `.dwg`, in any case.
fn is_dwg(source: &Path) -> bool {
    source
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dwg"))
}

/// The error for a sheet to draw that is not among `names`, the sheets
/// there are.
fn no_such_sheet(sheet: Option<&str>, names: &[String]) -> Error {
    Error::new(
        ErrorCode::UnknownName,
        format!(
            "there is no sheet `{}` to draw; the sheets: {}",
            sheet.unwrap_or_default(),
            names.join(", ")
        ),
    )
    .at_field("sheet")
}
