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
//! an entry of its journal, rebuilt in memory from the undo states. With
//! `snaps`, the reply also gives each object's object snaps, so the viewer
//! finds the snap nearest the cursor with no round trip
//! ([`crate::display_snaps`]). With `origins`, it says where each of a
//! drawing set's objects comes from, its sheet's template or its row, and
//! which sheets are built from that template: what an edit to it reaches,
//! as a change set writes it (`docs/change-sets.md`). With `values`, each
//! object gives the values quick properties edit, a text's contents, a
//! circle's or an arc's radius and an insert's attribute values, and on a
//! drawing set's sheet where each is written: by the object itself, by the
//! sheet's row, in how many places, or nowhere. With `change_set`, a drawing
//! set's sheet is drawn as the change set would leave it, applied in memory
//! as `change --dry-run` applies it, nothing written and nothing in the
//! journal: its preview marks what it would add and change, gives what it
//! would change or delete as it draws now, says sheet by sheet whether it
//! would land, and gives the checks the sheets would then fail.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::sync::Arc;

use crate::build::is_same_file;
use crate::change::{preview, ChangeReport, FilesAsOf, HowChanged, ObjectChanged, Previewed};
use crate::check::{check_in, CheckReport, CheckRequest, Finding, Rule};
use crate::description::{
    read_sheet_record, ObjectSources, ReadFiles, SavedSet, SheetSources, Source, ID_SEPARATOR,
};
use crate::display_snaps::{edge_fills, snaps_of, DisplaySnap};
use crate::draw::{self, Bounds, DisplayList, HatchCutShort, Item};
use crate::error::{Error, ErrorCode};
use crate::model::{Attribute, Object, Paper, Sheet};
use crate::output::read_record_with_recovery;
use crate::reply::Reply;
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
    /// Which layout of a sheet drawn in several to draw, by its name, in
    /// any case. Without it, the sheet's first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// Give each object's object snaps with the display list: every point
    /// its own snaps find, and where it crosses another. Without it, none,
    /// and the reply is as it was.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub snaps: bool,
    /// Say where each of a drawing set's objects comes from, its sheet's
    /// template or its row, and which sheets are built from the template.
    /// Without it, nothing, and the reply is as it was.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub origins: bool,
    /// Give the values quick properties edit of each object, and on a
    /// drawing set's sheet where each is written. Without it, none, and the
    /// reply is as it was.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub values: bool,
    /// Draw a drawing set's sheet as this change set would leave it: the
    /// same JSON `change` takes, applied in memory to the set as it is, as
    /// a dry run applies it. Nothing is written and nothing goes in the
    /// journal. The reply then gives its `preview`. Without it, the set as
    /// it is, and the reply is as it was. Given as `null`, it is refused, as
    /// `change` refuses it, not read as left out.
    #[serde(
        default,
        deserialize_with = "given",
        skip_serializing_if = "Option::is_none"
    )]
    pub change_set: Option<Value>,
    /// With `change_set`: apply it as a proposal is kept, on every sheet it
    /// reaches or on none, so a sheet that would take its part while
    /// another refuses is shown held back (`shared_edit_failed`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_or_none: bool,
    /// On a damaged original DWG that `read` read past damage, draw the
    /// recoverable objects read-only instead of refusing with `dwg_read`.
    /// Default false: strict DWG reading stands unless asked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_past_damage: bool,
}

/// A value given, even `null`, as itself: only a key left out is none.
fn given<'de, D: serde::Deserializer<'de>>(value: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(value).map(Some)
}

impl DisplayListRequest {
    /// A request for the first sheet of `source`, at the default tolerance.
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            sheet: None,
            tolerance: None,
            entry: None,
            layout: None,
            snaps: false,
            origins: false,
            values: false,
            change_set: None,
            all_or_none: false,
            read_past_damage: false,
        }
    }
}

/// Why a change set given with an entry is refused: it is drawn where it
/// would land.
pub(crate) const NOT_AT_AN_ENTRY: &str = concat!(
    "a change set is drawn on the drawing set as it is, where it would land, ",
    "not as it was at an entry of its journal"
);

/// A change set drawn before it lands: what it would do to the drawing
/// set, sheet by sheet, and on the sheet drawn what it would add, change
/// and delete, and the checks the sheet would then fail.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Preview {
    /// What the change set would do: its dry run's reply, as `change
    /// --dry-run` gives it, each sheet it reaches with whether it would
    /// take its part, or why not.
    pub change: ChangeReport,
    /// The checker's reply on the sheets it would land on, read from the
    /// records as it would write them, as `check` reads the saved files.
    /// Absent when no sheet would take its part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Reply<CheckReport>>,
    /// Each object it would add, change or delete on each sheet it would
    /// land on, by its ID, as its journal entry would name them.
    pub objects: Vec<ObjectChanged>,
    /// On the sheet drawn, the places among the display list's items of
    /// what it would add: every item of an object it adds.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<usize>,
    /// On the sheet drawn, the places among the display list's items of
    /// what it would change: the items of an object it changes that draw
    /// otherwise than they draw now.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<usize>,
    /// On the sheet drawn, each object it would change or delete, with what
    /// it draws now where it would draw no more: a changed object's items
    /// that would draw otherwise, such as its old place, and every item of
    /// a deleted one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub was: Vec<WasDrawn>,
    /// On the sheet drawn, a box round each check it would fail there, in
    /// drawing units: what the viewer clouds.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clouds: Vec<Cloud>,
    /// Said when the sheet as it is cannot be drawn, such as a line on a
    /// layer the template lacks, which the change set repairs: why, and that
    /// `was` is then empty and every item of what it changes is marked.
    /// Left out when the sheet draws as it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// What one object a change set would change or delete draws now, where it
/// would draw no more.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WasDrawn {
    pub id: String,
    /// `changed` or `deleted`.
    pub how: HowChanged,
    /// The items, as the sheet as it is draws them; each names its object
    /// by its place on the sheet as it is.
    pub items: Vec<Item>,
}

/// A check a change set would fail on the sheet drawn, and the box round
/// what fails it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cloud {
    pub rule: Rule,
    /// In drawing units, on the sheet as the change set would leave it.
    pub bounds: Bounds,
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
    /// With `origins`, on a drawing set's sheet, where it is written: in
    /// the sheet's template, so an edit to it reaches every sheet built
    /// from the template, or in the sheet's row, so one reaches that sheet
    /// alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Origin>,
    /// With `values`, the values quick properties edit, for a text, an
    /// mtext, a circle, an arc or an insert with attributes; none for any
    /// other object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<ObjectValues>,
}

/// What quick properties edit of an object: a text's or an mtext's
/// contents, a circle's or an arc's radius, and an insert's attribute
/// values, each, on a drawing set's sheet, with where it is written.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ObjectValues {
    /// A text's or an mtext's contents, as they show.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<WrittenValue>,
    /// A circle's or an arc's radius, in drawing units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    /// An insert's attribute values, one for each of its `attributes`, in
    /// their order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttributeValue>,
}

/// A text value of an object, and where it is written.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WrittenValue {
    /// The value, as it shows.
    pub value: String,
    /// On a drawing set's sheet, where it is written; none for a sheet
    /// description or a DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written: Option<Written>,
    /// When the row writes it, the name of the row's value, such as
    /// `WIRE_1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_value: Option<String>,
    /// When the row writes it, how many places on the sheet the row's value
    /// fills, this one among them: every one changes with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fills: Option<usize>,
    /// When the template joins it from a list of parts, plain text and the
    /// row's values: each row value it holds, in the order they first come,
    /// with how many places on the sheet each fills, this one among them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_values: Option<Vec<PartFills>>,
}

/// One row value a list of parts holds, and how many places on the sheet
/// it fills.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PartFills {
    /// The row value's name, such as `PANEL`.
    pub name: String,
    /// How many places on the sheet it fills, this one among them.
    pub fills: usize,
}

/// One attribute value of an insert, and where it is written.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttributeValue {
    /// Its tag.
    pub tag: String,
    /// Which of the tag's values it is, counted from 1, when the block
    /// defines the tag more than once, as `set_attribute` names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<usize>,
    #[serde(flatten)]
    pub value: WrittenValue,
}

/// Where one of a drawing set's sheet's values is written, as a change set
/// writes an edit to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Written {
    /// By the object itself, where its `from` says: an edit reaches every
    /// sheet built from its template, or its sheet alone.
    Own,
    /// By the sheet's row, as a value its template takes from the row: an
    /// edit sets the row's value, on this sheet alone, in every place it
    /// fills.
    RowValue,
    /// By the sheet's row, as the row values its template joins with plain
    /// text in a list of parts: an edit is matched to the parts and sets
    /// each row value they hold, on this sheet alone, in every place each
    /// fills.
    RowValues,
    /// Nowhere: an attribute the insert gives no value for, which shows its
    /// definition's default. An edit sets it on the insert, where its
    /// `from` says.
    Unset,
}

/// Where a drawing set's object is written, as a change set edits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// In the sheet's template: an edit to it changes the template, and
    /// every sheet built from it follows.
    Template,
    /// In the sheet's row, among its own objects: an edit to it reaches
    /// that sheet alone.
    Row,
}

/// A drawing set's sheet's template, and every sheet built from it: the
/// sheets an edit to one of the template's objects reaches.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TemplateReach {
    /// The template's path, as the sheet's row writes it.
    pub path: String,
    /// Every sheet built from it, in the set's order, this one among them.
    pub sheets: Vec<String>,
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
    /// On a sheet drawn in several layouts, the layout drawn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// On a sheet drawn in several layouts, every one of them, in the order
    /// of their tabs; each draws to a display list of its own.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layouts: Vec<String>,
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
    /// The objects of a DWG not drawn because the core has no model for
    /// them yet, by kind, such as `ACAD_PROXY_ENTITY`, with the handle of
    /// each, as `read` names them. Left out when every object is drawn.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub not_drawn: Vec<crate::model::RecordsOfKind>,
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
    /// Raster images or PDF underlays that could not be found or decoded. Left
    /// out when every attachment resolved cleanly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_media: Vec<crate::model::UnresolvedMedia>,
    /// The journal's entry the set is drawn as it was just after, when it
    /// was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<u64>,
    /// Each of the sheet's objects, by its place, which each item of the
    /// display list names as its `object`.
    pub objects: Vec<SheetObject>,
    /// With `snaps`, every point the object snaps find on the objects the
    /// page draws itself, each with its object by its place, the snap,
    /// `which` of the object's points, and for an intersection the other
    /// object by its place, as `query snap` names them. Left out without it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snaps: Option<Vec<DisplaySnap>>,
    /// With `snaps`, the places among the display list's items of the fills
    /// the nearest snap finds a point on, at their edges, as it does along
    /// a stroke: those a polyline with width, a solid and a donut draw on
    /// the page itself, and no text's, hatch's or arrowhead's. Left out
    /// without it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<usize>>,
    /// With `origins`, on a drawing set's sheet, its template and every
    /// sheet built from it, which an edit to an object whose `from` is
    /// `template` reaches. Left out without it, and for a sheet description
    /// or a DWG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateReach>,
    /// With `origins`, on a sheet drawn in a layout, the space an object
    /// drawn on the page goes in, as a change set's add names it: `paper`,
    /// or on a sheet drawn in several layouts the name of the one drawn.
    /// Left out without it, and for a sheet drawn in model space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    /// With `change_set`, what it would do; the sheet is drawn as it would
    /// leave it. Left out without it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<Preview>,
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
    // A change set is applied in memory to the set as it is.
    let previewed = match &request.change_set {
        None => None,
        Some(change_set) => {
            let refuse =
                |message: &str| Error::new(ErrorCode::BadValue, message).at_field("change_set");
            if request.entry.is_some() {
                return Err(refuse(NOT_AT_AN_ENTRY));
            }
            if !source.is_dir() {
                return Err(refuse(
                    "a change set is drawn only on a drawing set's sheet: it edits a set's \
                     records, and a sheet description or a DWG is not a set",
                ));
            }
            Some(preview(source, change_set, settings, request.all_or_none)?)
        }
    };
    let files: Option<Arc<dyn ReadFiles + Send + Sync>> = match (request.entry, &previewed) {
        (Some(entry), _) if source.is_dir() => Some(Arc::new(FilesAsOf::read(source, entry)?)),
        (_, Some(p)) => Some(p.files.clone()),
        _ => None,
    };
    let drawn = read(
        source,
        request.sheet.as_deref(),
        request.entry,
        files.clone(),
        request,
    )?;
    // A sheet drawn in several layouts draws the one asked for.
    let pages = drawn.sheet.pages();
    let layouts: Vec<String> = if pages.len() > 1 {
        drawn.sheet.layouts().map(|l| l.name.clone()).collect()
    } else {
        Vec::new()
    };
    let at = match &request.layout {
        None => 0,
        Some(asked) => pages
            .iter()
            .position(|p| {
                p.layout
                    .as_ref()
                    .is_some_and(|l| l.name.eq_ignore_ascii_case(asked))
            })
            .ok_or_else(|| {
                let names: Vec<String> = drawn
                    .sheet
                    .layouts()
                    .map(|l| format!("`{}`", l.name))
                    .collect();
                Error::new(
                    ErrorCode::UnknownName,
                    if names.is_empty() {
                        format!(
                            "sheet `{}` is drawn in no layout, so it has no layout `{asked}`",
                            drawn.name
                        )
                    } else {
                        format!(
                            "sheet `{}` has no layout `{asked}`; its layouts are {}",
                            drawn.name,
                            names.join(", ")
                        )
                    },
                )
                .at_field("layout")
            })?,
    };
    let sheet: &Sheet = &pages[at];
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
    let layout = (!layouts.is_empty())
        .then(|| sheet.layout.as_ref().map(|l| l.name.clone()))
        .flatten();
    // With origins, on a sheet drawn in a layout, the space an object drawn
    // on the page goes in, as an add names it.
    let space = request
        .origins
        .then(|| {
            sheet.layout.as_ref().map(|l| {
                if layouts.is_empty() {
                    "paper".to_owned()
                } else {
                    l.name.clone()
                }
            })
        })
        .flatten();
    let snaps = request.snaps.then(|| snaps_of(sheet, &fonts, &list));
    let edges = request.snaps.then(|| edge_fills(sheet, &list));
    let preview = match previewed {
        None => None,
        Some(p) => {
            let drew = Drew {
                drawn: &drawn,
                sheet,
                list: &list,
                tolerance,
            };
            Some(preview_of(p, files, &drew, source, settings)?)
        }
    };
    Ok(DisplayListReport {
        source: source.display().to_string(),
        name,
        sheet: drawn.name.clone(),
        sheets: drawn.names.clone(),
        layout,
        layouts,
        units: drawn.units,
        millimetres_per_unit: sheet.units.convert(1.0, Unit::Mm),
        paper: sheet.paper.clone(),
        extents: list.extents(),
        tolerance,
        objects_not_drawn: drawn.objects_not_drawn,
        not_drawn: drawn.not_drawn,
        font_substitutions: fonts.substitutions(),
        hatches_cut_short: list.hatches_cut_short.clone(),
        unresolved_xrefs: drawn.unresolved_xrefs,
        unresolved_media: drawn.unresolved_media,
        entry: request.entry,
        objects: drawn.objects.clone(),
        snaps,
        edges,
        template: drawn.template,
        space,
        preview,
        display_list: list,
    })
}

/// A sheet drawn as a change set would leave it: what was read, the page
/// drawn and its display list, and the tolerance it was drawn at.
struct Drew<'a> {
    drawn: &'a Drawn,
    sheet: &'a Sheet,
    list: &'a DisplayList,
    tolerance: f64,
}

/// The preview of the change set `p` on the sheet `drew`, read through
/// `files`, the set as it would leave it. The sheet as it is now is drawn
/// too, its same layout at the same tolerance, so an item an object would
/// draw the same is the same, and only what would draw otherwise is marked.
/// When it cannot be, as for a sheet the change set repairs, the preview is
/// still given, without the old places, and says why.
fn preview_of(
    p: Previewed,
    files: Option<Arc<dyn ReadFiles + Send + Sync>>,
    drew: &Drew,
    source: &Path,
    settings: &Settings,
) -> Result<Preview, Error> {
    let name = drew.drawn.name.as_str();
    let (old, warning) = match drawn_now(name, drew, source, settings) {
        Ok(old) => (Some(old), None),
        Err(e) => (
            None,
            Some(format!(
                "sheet `{name}` cannot be drawn as it is ({}), so what the change set would change \
                  or delete is not shown where it draws now: every item of what it would change is \
                  marked",
                e.message
            )),
        ),
    };
    let on_sheet = |id: &str| {
        id.split_once(ID_SEPARATOR)
            .is_some_and(|(s, _)| s.eq_ignore_ascii_case(name))
    };
    let (mut added, mut changed, mut was) = (Vec::new(), Vec::new(), Vec::new());
    for object in p.objects.iter().filter(|o| on_sheet(&o.id)) {
        let after = items_of(drew.list, place(&drew.drawn.objects, &object.id));
        let before = old.as_ref().map_or_else(Vec::new, |(now, was_list)| {
            items_of(was_list, place(&now.objects, &object.id))
        });
        match object.how {
            HowChanged::Added => added.extend(after.iter().map(|(i, _)| *i)),
            HowChanged::Deleted if !before.is_empty() => was.push(WasDrawn {
                id: object.id.clone(),
                how: HowChanged::Deleted,
                items: before.into_iter().map(|(_, item)| item.clone()).collect(),
            }),
            HowChanged::Deleted => {}
            HowChanged::Changed => {
                // What draws the same, before and after, is not marked.
                let (new, gone) = differ(&after, &before);
                changed.extend(new);
                if !gone.is_empty() {
                    was.push(WasDrawn {
                        id: object.id.clone(),
                        how: HowChanged::Changed,
                        items: gone,
                    });
                }
            }
        }
    }
    added.sort_unstable();
    added.dedup();
    changed.sort_unstable();
    changed.dedup();
    // The checker's reply on the sheets it would land on, from the records
    // as it would write them.
    let landed: Vec<String> = p
        .report
        .sheets
        .iter()
        .filter(|s| s.ok)
        .map(|s| s.sheet.clone())
        .collect();
    let checks = (!landed.is_empty()).then(|| {
        let request = CheckRequest {
            source: source.to_owned(),
            sheets: landed,
        };
        let mut reply = Reply::from(check_in(&request, settings, files));
        // The set's mark as the change set would leave it.
        if let Some(report) = reply.result.as_mut() {
            report.inconsistent.clone_from(&p.report.inconsistent);
        }
        reply
    });
    let layout_drawn = drew.sheet.layout.as_ref().map(|l| l.name.to_lowercase());
    let clouds = checks
        .as_ref()
        .and_then(|c| c.result.as_ref())
        .map(|report| {
            report
                .findings
                .iter()
                .filter(|f| f.sheet.eq_ignore_ascii_case(name))
                .filter(|f| f.layout.as_ref().map(|l| l.to_lowercase()) == layout_drawn)
                .filter_map(|f| {
                    Some(Cloud {
                        rule: f.rule,
                        bounds: cloud_of(f, &drew.drawn.objects, drew.list)?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Preview {
        change: p.report,
        checks,
        objects: p.objects,
        added,
        changed,
        was,
        clouds,
        warning,
    })
}

/// The sheet `drew` draws as it is now, before the change set: what was
/// read and its display list, its same layout at the same tolerance.
fn drawn_now(
    name: &str,
    drew: &Drew,
    source: &Path,
    settings: &Settings,
) -> Result<(Drawn, DisplayList), Error> {
    let now = read(
        source,
        Some(name),
        None,
        None,
        &DisplayListRequest::new(source),
    )?;
    let pages = now.sheet.pages();
    let layout = drew.sheet.layout.as_ref().map(|l| l.name.to_lowercase());
    let page = pages
        .iter()
        .find(|p| p.layout.as_ref().map(|l| l.name.to_lowercase()) == layout)
        .unwrap_or(&pages[0]);
    let list = draw::draw_sheet(
        page,
        &SheetFonts::find(page, settings),
        drew.tolerance,
        draw::MOST_HATCH_DASHES,
    );
    Ok((now, list))
}

/// The place among `objects` of the object whose ID is `id`, in any case.
fn place(objects: &[SheetObject], id: &str) -> Option<usize> {
    objects
        .iter()
        .position(|o| o.id.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(id)))
}

/// The items of `list` that the object at `at` draws, with their places.
fn items_of(list: &DisplayList, at: Option<usize>) -> Vec<(usize, &Item)> {
    let Some(at) = at else {
        return Vec::new();
    };
    list.items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.object == at)
        .collect()
}

/// Of one object's items `after` a change and `before` it: the places of
/// those after that draw otherwise than any before, and those before that
/// draw otherwise than any after. An item is the same whatever its
/// object's place, which a deletion before it moves.
fn differ(after: &[(usize, &Item)], before: &[(usize, &Item)]) -> (Vec<usize>, Vec<Item>) {
    let key = |item: &Item| {
        let mut item = item.clone();
        item.object = 0;
        serde_json::to_string(&item).unwrap_or_default()
    };
    let counts = |items: &[(usize, &Item)]| {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (_, item) in items {
            *counts.entry(key(item)).or_default() += 1;
        }
        counts
    };
    let taken = |counts: &mut HashMap<String, usize>, item: &Item| match counts.get_mut(&key(item))
    {
        Some(n) if *n > 0 => {
            *n -= 1;
            true
        }
        _ => false,
    };
    let mut was = counts(before);
    let new = after
        .iter()
        .filter(|(_, item)| !taken(&mut was, item))
        .map(|(i, _)| *i)
        .collect();
    let mut now = counts(after);
    let gone = before
        .iter()
        .filter(|(_, item)| !taken(&mut now, item))
        .map(|(_, item)| (*item).clone())
        .collect();
    (new, gone)
}

/// The box round what fails the check `finding`, on the sheet drawn to
/// `list`, whose objects are `objects`: what the checker says is drawn, or,
/// for a text overflowing its cell, which the checker measures in its
/// block's own coordinates, what the text draws on the sheet; or, for a
/// sheet that would draw nothing and so is not its paper, the paper, where
/// the page draws it.
fn cloud_of(finding: &Finding, objects: &[SheetObject], list: &DisplayList) -> Option<Bounds> {
    if finding.rule != Rule::TextOverflow {
        if let Some(drawn) = &finding.drawn {
            return Some(Bounds {
                min: drawn.min,
                max: drawn.max,
            });
        }
    }
    if let (Rule::PaperSize, None, Some(paper)) = (finding.rule, &finding.object, &finding.paper) {
        return Some(Bounds {
            min: [0.0, 0.0],
            max: [paper.width, paper.height],
        });
    }
    let object = finding.object.as_ref()?;
    let at = place(objects, &object.id)?;
    // The part of an insert's block that draws the text, when it is one of
    // the block's own objects, as `blocks.TITLE.objects[13]` names it.
    let part = object.in_block.as_ref().and_then(|b| {
        let field = b.field.as_deref()?;
        let (_, tail) = field.rsplit_once(".objects[")?;
        let part = tail.strip_suffix(']')?.parse::<usize>().ok()?;
        (field.matches(".objects[").count() == 1).then_some(part)
    });
    let items: Vec<&Item> = items_of(list, Some(at))
        .into_iter()
        .map(|(_, i)| i)
        .collect();
    let bounds = |chosen: Vec<&&Item>| Bounds::of(chosen.into_iter().flat_map(|i| i.points()));
    let narrowed = match (part, object.table_cell) {
        (_, Some(cell)) => bounds(items.iter().filter(|i| i.cell == Some(cell)).collect()),
        (Some(part), None) => bounds(items.iter().filter(|i| i.part == Some(part)).collect()),
        _ => None,
    };
    narrowed.or_else(|| bounds(items.iter().collect()))
}

/// The sheet to draw, read from its record.
struct Drawn {
    name: String,
    names: Vec<String>,
    sheet: Sheet,
    units: Option<Unit>,
    objects_not_drawn: usize,
    not_drawn: Vec<crate::model::RecordsOfKind>,
    unresolved_xrefs: Vec<crate::model::UnresolvedXref>,
    unresolved_media: Vec<crate::model::UnresolvedMedia>,
    objects: Vec<SheetObject>,
    /// A drawing set's sheet's template and the sheets built from it.
    template: Option<TemplateReach>,
}

/// What each of `sheet`'s objects is, with its ID and name when `ids`
/// gives them, by the same places, and where it is written when `from`
/// says, by the same places. With `values`, the values quick properties
/// edit, and where each is written when `sources` says.
fn objects_of(
    sheet: &Sheet,
    ids: &[(String, Option<String>)],
    from: &[Origin],
    values: bool,
    sources: Option<&SheetSources>,
) -> Vec<SheetObject> {
    sheet
        .objects
        .iter()
        .enumerate()
        .map(|(i, object)| {
            let (block, attributes) = match object {
                Object::Insert(insert) => (Some(insert.block.clone()), insert.attributes.clone()),
                _ => (None, Vec::new()),
            };
            let values = values
                .then(|| {
                    let written = sources.map(|s| (s.objects.get(i), &s.fills));
                    values_of(object, &attributes, written)
                })
                .flatten();
            SheetObject {
                id: ids.get(i).map(|(id, _)| id.clone()),
                name: ids.get(i).and_then(|(_, name)| name.clone()),
                kind: object.type_name().to_owned(),
                layer: object.layer().to_owned(),
                block,
                attributes,
                from: from.get(i).copied(),
                values,
            }
        })
        .collect()
}

/// The values quick properties edit of `object`, whose attributes are
/// `attributes`, and where each is written when `written` says, on a
/// drawing set's sheet: the object's own sources, and how many places each
/// of the row's values fills. None for an object with none.
fn values_of(
    object: &Object,
    attributes: &[Attribute],
    written: Option<(Option<&ObjectSources>, &BTreeMap<String, usize>)>,
) -> Option<ObjectValues> {
    let own = written.map(|(own, fills)| (own.cloned().unwrap_or_default(), fills));
    // A value, and where it is written when the sheet is a set's.
    let value = |value: &str, source: Option<Source>| -> WrittenValue {
        let mut given = WrittenValue {
            value: value.to_owned(),
            written: None,
            row_value: None,
            fills: None,
            row_values: None,
        };
        if let (Some(source), Some((_, fills))) = (source, &own) {
            given.written = Some(match source {
                Source::Own => Written::Own,
                Source::Unset => Written::Unset,
                Source::Row(name) => {
                    given.fills = Some(fills.get(&name).copied().unwrap_or(1));
                    given.row_value = Some(name);
                    Written::RowValue
                }
                Source::Parts(names) => {
                    given.row_values = Some(
                        names
                            .into_iter()
                            .map(|name| PartFills {
                                fills: fills.get(&name).copied().unwrap_or(1),
                                name,
                            })
                            .collect(),
                    );
                    Written::RowValues
                }
            });
        }
        given
    };
    let contents = |text: &str| {
        let source = own
            .as_ref()
            .map(|(own, _)| own.contents.clone().unwrap_or(Source::Own));
        Some(ObjectValues {
            contents: Some(value(text, source)),
            radius: None,
            attributes: Vec::new(),
        })
    };
    let radius = |radius: f64| {
        Some(ObjectValues {
            contents: None,
            radius: Some(radius),
            attributes: Vec::new(),
        })
    };
    match object {
        Object::Text(text) => contents(&text.contents),
        Object::MText(text) => contents(&text.contents),
        Object::Circle(circle) => radius(circle.radius),
        Object::Arc(arc) => radius(arc.radius),
        Object::Insert(_) if !attributes.is_empty() => {
            let values = attributes
                .iter()
                .enumerate()
                .map(|(i, attribute)| {
                    // A tag the block defines more than once names which.
                    let same = |a: &Attribute| a.tag == attribute.tag;
                    let before = attributes[..i].iter().filter(|a| same(a)).count();
                    let repeated = before > 0 || attributes[i + 1..].iter().any(same);
                    let occurrence = repeated.then_some(before);
                    let source = own
                        .as_ref()
                        .map(|(own, _)| own.attribute(&attribute.tag, occurrence));
                    AttributeValue {
                        tag: attribute.tag.clone(),
                        occurrence: occurrence.map(|n| n + 1),
                        value: value(&attribute.value, source),
                    }
                })
                .collect();
            Some(ObjectValues {
                contents: None,
                radius: None,
                attributes: values,
            })
        }
        _ => None,
    }
}

/// Reads the sheet `sheet` names from `source`: a drawing set's folder, as
/// it was just after its journal's entry `entry` when one is given, or one
/// sheet description or DWG, whose one sheet is named by its file; a set
/// is read through `files` when they are given, as it was at the entry or
/// as a change set would leave it. With `origins`, a set's sheet says where
/// each object is written and which sheets its template reaches; without,
/// neither is worked out. With `values`, each object gives the values quick
/// properties edit, and a set's sheet where each is written.
fn read(
    source: &Path,
    sheet: Option<&str>,
    entry: Option<u64>,
    files: Option<Arc<dyn ReadFiles + Send + Sync>>,
    request: &DisplayListRequest,
) -> Result<Drawn, Error> {
    let origins = request.origins;
    let named = |name: &str| sheet.is_none_or(|s| s.to_lowercase() == name.to_lowercase());
    if source.is_dir() {
        let set = match files {
            Some(files) => SavedSet::read_from(source, files)?,
            None => SavedSet::read(source)?,
        };
        let names: Vec<String> = set.names().into_iter().map(str::to_owned).collect();
        let Some(row) = set.rows.iter().find(|row| named(&row.sheet)) else {
            return Err(no_such_sheet(sheet, &names));
        };
        // With values, the sheet and where each of its values is written,
        // from one read of its template.
        let (mut record, sources) = if request.values {
            let (record, sources) = set.sheet_with_sources(row)?;
            (record, Some(sources))
        } else {
            (set.sheet(row)?, None)
        };
        let host = record.file.clone();
        let unresolved_xrefs =
            crate::xref::resolve_sheet_xrefs(&mut record.sheet, &host).unresolved;
        let unresolved_media =
            crate::media::resolve_sheet_media_for_host(&mut record.sheet, &host).unresolved;
        let ids: Vec<(String, Option<String>)> = (0..record.objects.len())
            .map(|i| (record.id(i), record.objects[i].name.clone()))
            .collect();
        let (from, template) = if origins {
            let from: Vec<Origin> = record
                .objects
                .iter()
                .map(|o| {
                    if o.in_row {
                        Origin::Row
                    } else {
                        Origin::Template
                    }
                })
                .collect();
            // The sheets built from the same template: those whose rows
            // name one file, as the file system resolves their paths, as a
            // change set finds the sheets a template edit reaches.
            let template_file = source.join(&row.template);
            let sheets = set
                .rows
                .iter()
                .filter(|other| {
                    other.template == row.template
                        || is_same_file(&template_file, &source.join(&other.template))
                })
                .map(|other| other.sheet.clone())
                .collect();
            let reach = TemplateReach {
                path: row.template.clone(),
                sheets,
            };
            (from, Some(reach))
        } else {
            (Vec::new(), None)
        };
        return Ok(Drawn {
            name: record.name,
            names,
            units: Some(record.sheet.units),
            objects: objects_of(&record.sheet, &ids, &from, request.values, sources.as_ref()),
            sheet: record.sheet,
            objects_not_drawn: 0,
            not_drawn: Vec::new(),
            unresolved_xrefs,
            unresolved_media,
            template,
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
    let name = if crate::original::is_drawing(source) {
        crate::original::sheet_name(source)
    } else {
        source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    if !named(&name) {
        return Err(no_such_sheet(sheet, std::slice::from_ref(&name)));
    }
    let record = read_record_with_recovery(source, request.read_past_damage)?;
    // A DWG's or DXF's objects carry the IDs its read gives them.
    let ids: Vec<(String, Option<String>)> = if crate::original::is_drawing(source) {
        record.ids.clone()
    } else {
        let described = read_sheet_record(source)?;
        (0..described.objects.len())
            .map(|i| (described.id(i), described.objects[i].name.clone()))
            .collect()
    };
    Ok(Drawn {
        names: vec![name.clone()],
        name,
        objects: objects_of(&record.sheet, &ids, &[], request.values, None),
        sheet: record.sheet,
        units: record.units,
        objects_not_drawn: record.objects_not_drawn,
        not_drawn: record.not_drawn,
        unresolved_xrefs: record.unresolved_xrefs,
        unresolved_media: record.unresolved_media,
        template: None,
    })
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
