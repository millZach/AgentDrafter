//! The registry of commands: everything an interface can ask the core to
//! do, in one list, so that no interface can offer a command another lacks
//! (ADR-0007).
//!
//! A command has one name through every interface, such as `build` or
//! `change`, and takes its input as plain data: a JSON object, whose JSON
//! Schema the command gives. The command line turns its arguments into that
//! object, the HTTP API takes it as a request's body and the MCP server as
//! a tool's arguments, and each sends back the reply [`Command::run`] gives,
//! as the command line prints it. None of them holds drawing logic: what a
//! command makes of its input, such as whether a folder is a drawing set, is
//! decided here, once.
//!
//! Paths in an input are taken as the command line takes them: relative to
//! the folder the interface runs in, or absolute. Font and plot style
//! folders an input names come before those the interface's settings name,
//! as `--fonts` and `--plot-styles` come before the environment's, and the
//! system's font folders come last, for TrueType fonts, unless the input's
//! `no_system_fonts` or the environment leaves them out. A command that
//! measures or draws text, and in which a look-alike stood in for a font,
//! says so under `warnings` at the top of its reply.
//!
//! Before a command reads a drawing set, the set its input names, it rolls
//! back an entry of the set's journal a commit that never finished left
//! pending, as Zach decided on 2026-11-11, and its reply says so under
//! `rolled_back`; when the rollback cannot complete, the command refuses.
//! Since every interface runs its commands here, this holds for each.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Map, Value};

use crate::build::{build_with, BuildRequest};
use crate::change::{accept_with, recover, AcceptRequest, JournalRequest, UndoRequest};
use crate::change::{change_set_schema, change_with, journal, operations, undo, ChangeRequest};
use crate::change::{
    discard, keep_with, proposals, propose_with, DiscardRequest, KeepRequest, ProposalsRequest,
    ProposeRequest, BY_LONGEST, BY_PATTERN, DRAFTER, PROPOSALS_FOLDER,
};
use crate::check::{check_with, CheckRequest};
use crate::display_list::{display_list_with, DisplayListRequest, NOT_AT_AN_ENTRY};
use crate::error::{Error, ErrorCode};
use crate::import::{import_blocks, ImportBlocksRequest};
use crate::interface::Interface;
use crate::plot::{plot_set_with, plot_with, PlotRequest, PlotSetRequest, BUILT_IN_PLOT_STYLE};
use crate::query::{query_with, QueryRequest};
use crate::render::{render_with, RenderRequest};
use crate::reply::Reply;
use crate::set::{build_set_with, set_of_dwg, SetBuildRequest, SET_SETTINGS_FILE};
use crate::settings::{Settings, FONTS_VARIABLE, PLOT_STYLES_VARIABLE, SYSTEM_FONTS_VARIABLE};
use crate::stamp::{check_stale, SourceKind, StaleRequest};
use crate::text::measure::{measure_text_with, MeasureTextRequest};
use crate::units::Unit;
use crate::warning::Warns;
use crate::{block_library_schema, rows_schema, set_schema, sheet_description_schema};
use crate::{template_schema, JOURNAL_FILE, KEPT_UNDO_STATES};

/// A command's reply, its result already JSON, written in the order the
/// core gives its fields, as every interface sends it.
pub type JsonReply = Reply<Box<RawValue>>;

/// How the core carries a command out on its input, an object, with the
/// machine's settings, for the interface it came through.
type Run = fn(Map<String, Value>, &Settings, Interface) -> Result<JsonReply, Error>;

/// One command of the registry: its name, what it does, the schema of its
/// input, and how the core carries it out.
pub struct Command {
    name: &'static str,
    does: fn() -> String,
    input: fn() -> Value,
    run: Run,
}

impl Command {
    /// The command's name, the same through every interface, such as
    /// `measure-text`.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// What the command does, in words, naming its input's fields.
    pub fn does(&self) -> String {
        (self.does)()
    }

    /// The JSON Schema (draft 2020-12) of the command's input: an object
    /// that takes no key the schema does not name.
    pub fn input_schema(&self) -> Value {
        (self.input)()
    }

    /// Carries the command out on `input`, a JSON object its input schema
    /// describes, with `settings` saying where the machine keeps its fonts
    /// and plot styles, for the interface `via` it came through, which an
    /// accept records in the journal. The reply is the one the command line
    /// prints. An input that does not fit the schema is a `usage` error.
    ///
    /// When the input names a drawing set, or a file in one, an entry of
    /// the set's journal that a commit that never finished left pending is
    /// rolled back first, and the reply says so; when it cannot be, the
    /// command refuses and does nothing. A `display-list` with a
    /// `change_set`, a preview, and `propose` and `proposals`, write nothing
    /// to the set: they roll nothing back, and are refused instead on a set
    /// whose journal holds such an entry.
    pub fn run(&self, input: Value, settings: &Settings, via: Interface) -> JsonReply {
        // A preview, and `propose` and `proposals`, write nothing to the
        // set, so they roll nothing back; each refuses a set whose journal
        // holds an unfinished commit instead.
        let previews = (self.name == "display-list" && input.get("change_set").is_some())
            || matches!(self.name, "propose" | "proposals");
        let rolled_back = match set_read(&input)
            .filter(|_| !previews)
            .map(|set| recover(&set))
        {
            Some(Err(error)) => return Reply::from(Err(error)),
            Some(Ok(rolled_back)) => rolled_back,
            None => None,
        };
        let outcome = match input {
            Value::Object(input) => (self.run)(input, settings, via),
            other => Err(Error::usage(format!(
                "the input to `{}` is a JSON object, not {}",
                self.name,
                json_kind(&other)
            ))),
        };
        outcome
            .unwrap_or_else(|error| Reply::from(Err(error)))
            .after(rolled_back)
    }

    /// The command as plain data, to list it: its name, what it does and
    /// its input's schema.
    pub fn info(&self) -> CommandInfo {
        CommandInfo {
            name: self.name.to_owned(),
            does: self.does(),
            input_schema: self.input_schema(),
        }
    }
}

/// A command, listed as plain data.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CommandInfo {
    pub name: String,
    pub does: String,
    pub input_schema: Value,
}

/// Every command, in the order the command line's help lists them.
pub fn commands() -> &'static [Command] {
    COMMANDS
}

/// The command of this name, if there is one.
pub fn find_command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.name == name)
}

const COMMANDS: &[Command] = &[
    Command {
        name: "build",
        does: build_does,
        input: build_input,
        run: run_build,
    },
    Command {
        name: "schema",
        does: schema_does,
        input: schema_input,
        run: run_schema,
    },
    Command {
        name: "measure-text",
        does: measure_does,
        input: measure_input,
        run: run_measure,
    },
    Command {
        name: "render",
        does: render_does,
        input: render_input,
        run: run_render,
    },
    Command {
        name: "display-list",
        does: display_list_does,
        input: display_list_input,
        run: run_display_list,
    },
    Command {
        name: "plot",
        does: plot_does,
        input: plot_input,
        run: run_plot,
    },
    Command {
        name: "stale",
        does: stale_does,
        input: stale_input,
        run: run_stale,
    },
    Command {
        name: "query",
        does: query_does,
        input: query_input,
        run: run_query,
    },
    Command {
        name: "operations",
        does: operations_does,
        input: operations_input,
        run: run_operations,
    },
    Command {
        name: "change",
        does: change_does,
        input: change_input,
        run: run_change,
    },
    Command {
        name: "journal",
        does: journal_does,
        input: journal_input,
        run: run_journal,
    },
    Command {
        name: "undo",
        does: undo_does,
        input: undo_input,
        run: run_undo,
    },
    Command {
        name: "accept",
        does: accept_does,
        input: accept_input,
        run: run_accept,
    },
    Command {
        name: "propose",
        does: propose_does,
        input: propose_input,
        run: run_propose,
    },
    Command {
        name: "proposals",
        does: proposals_does,
        input: proposals_input,
        run: run_proposals,
    },
    Command {
        name: "keep",
        does: keep_does,
        input: keep_input,
        run: run_keep,
    },
    Command {
        name: "discard",
        does: discard_does,
        input: discard_input,
        run: run_discard,
    },
    Command {
        name: "check",
        does: check_does,
        input: check_input,
        run: run_check,
    },
    Command {
        name: "import-blocks",
        does: import_blocks_does,
        input: import_blocks_input,
        run: run_import_blocks,
    },
];

// build

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildInput {
    source: PathBuf,
    #[serde(default)]
    output: Option<PathBuf>,
    #[serde(default)]
    overwrite: bool,
}

fn build_does() -> String {
    format!(
        "Builds a sheet description into a DWG in the 2018 format (AC1032), with the blocks it \
         inserts from the block library it names. The DWG goes beside the sheet description with \
         a .dwg extension unless `output` names another path. An existing file is never replaced \
         without `overwrite`, and `overwrite` replaces only a DWG AgentDrafter built. The reply \
         lists the blocks the DWG defines, and each text style whose font is not in the font \
         folders, with the look-alike measured in its place. Given a folder, builds the drawing \
         set in it: one DWG per row of its rows, each its template filled in from the row, into \
         the output folder its settings ({SET_SETTINGS_FILE}) name. A sheet that fails writes \
         nothing and the others are still built; the reply gives each sheet's outcome under \
         `sheets`, and is not ok when any sheet failed."
    )
}

fn build_input() -> Value {
    input(
        json!({
            "source": text("The sheet description to build, or a drawing set's folder."),
            "output": text("Where to write the DWG of one sheet description. Without it, beside the sheet description."),
            "overwrite": flag("Replace a DWG already there, which is allowed only when AgentDrafter built it."),
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["source"],
    )
}

fn run_build(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let BuildInput {
        source,
        output,
        overwrite,
    } = parse("build", input)?;
    if is_set(&source, output.as_deref(), "DWGs", "one sheet description")? {
        let request = SetBuildRequest {
            set: source,
            overwrite,
        };
        return Ok(sent_warned(build_set_with(&request, &settings)));
    }
    let request = BuildRequest {
        description: source,
        output,
        overwrite,
    };
    Ok(done_warned(build_with(&request, &settings)))
}

// schema

/// The kinds of file `schema` gives the schema of, the first the default.
const SCHEMA_KINDS: [&str; 6] = [
    "sheet-description",
    "block-library",
    "template",
    "set",
    "rows",
    "change-set",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaInput {
    #[serde(default)]
    kind: Option<String>,
}

fn schema_does() -> String {
    "Gives the sheet description's schema, or with `kind` the block library's, a template's, \
     a drawing set's settings', its rows' or a change set's: a JSON Schema (draft 2020-12) of \
     every key it may hold, with what each means."
        .to_owned()
}

fn schema_input() -> Value {
    input(
        json!({
            "kind": {
                "description": "Which file's schema. Without it, the sheet description's.",
                "enum": SCHEMA_KINDS,
            },
        }),
        &[],
    )
}

fn run_schema(input: Map<String, Value>, _: &Settings, _: Interface) -> Result<JsonReply, Error> {
    let SchemaInput { kind } = parse("schema", input)?;
    let schema = match kind.as_deref().unwrap_or(SCHEMA_KINDS[0]) {
        "sheet-description" => sheet_description_schema(),
        "block-library" => block_library_schema(),
        "template" => template_schema(),
        "set" => set_schema(),
        "rows" => rows_schema(),
        "change-set" => change_set_schema(),
        other => {
            return Err(Error::usage(format!(
                "schema gives the schema of a sheet description, a block library, a template, \
                 a drawing set's settings, its rows or a change set ({}), not `{other}`",
                SCHEMA_KINDS.join(", ")
            )))
        }
    };
    Ok(done(Ok(schema)))
}

// measure-text

fn measure_does() -> String {
    "Measures how wide one line of text, `contents`, draws in a `font` at a `height`, as \
     AutoCAD measures a text object: from the left edge of what it draws to the right edge. \
     Reports the font used, and the look-alike when the font is not in the font folders."
        .to_owned()
}

fn measure_input() -> Value {
    input(
        json!({
            "contents": text("The text to measure, one line."),
            "font": text("The font's file name: an SHX font such as `romans.shx` or a TrueType font such as `arial.ttf`."),
            "height": number("The height of the capitals; the measurement is in the same unit."),
            "width_factor": number("How wide characters are drawn, as a share of their normal width. Without it, 1."),
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["contents", "font", "height"],
    )
}

fn run_measure(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: MeasureTextRequest = parse("measure-text", input)?;
    Ok(done_warned(measure_text_with(&request, &settings)))
}

// render

fn render_does() -> String {
    "Draws a sheet description, or a DWG, to a PNG as AutoCAD plots it: layer colours on white \
     paper, lineweights as widths on paper at `dpi` (150), linetypes, text in its fonts and \
     hatches. The PNG covers the drawing's extents with a `margin` (5 mm of paper), at a plot \
     `scale` (1 for 1:1, 0.02 for 1:50) or fitted to `width` and `height` pixels; without \
     either, its longer side is 2000 pixels. It goes beside the source with a .png extension \
     unless `output` names another path, and is stamped with the source's fingerprint. An \
     existing file is never replaced without `overwrite`, and `overwrite` replaces only a PNG \
     AgentDrafter rendered. A sheet drawn in several layouts renders each to a PNG of its own, \
     named after the PNG with `-` and the layout's name, such as panel-Details.png; the reply is \
     the first layout's, with the others' under `other_layouts`. To show what prints, it draws \
     through the CTB plot style `plot_style` names, found in the plot style folders as plot \
     finds it, or with `monochrome` exactly as plot draws with no plot style named, every pen \
     black at its object's own lineweight; either way a layer that does not plot is left out, \
     and the two are not given together. The reply's `drawn_through` says which it drew \
     through: `colour`, `monochrome`, or the plot style, its file and the settings it does not \
     follow yet, as plot reports it."
        .to_owned()
}

fn render_input() -> Value {
    input(
        json!({
            "source": text("The record to render: a sheet description, or a DWG when the file ends .dwg."),
            "output": text("Where to write the PNG. Without it, beside the source. A sheet drawn in several layouts writes one for each, its name followed by `-` and the layout's."),
            "overwrite": flag("Replace a PNG already there, which is allowed only when AgentDrafter rendered it."),
            "dpi": number("How many pixels an inch of paper spans. Without it, 150."),
            "scale": number("The plot scale, such as 1 for 1:1 and 0.02 for 1:50. Not given with `width` or `height`."),
            "width": pixels("How many pixels wide the image is, the drawing fitted inside it."),
            "height": pixels("How many pixels high the image is, the drawing fitted inside it."),
            "margin": number("The margin round the drawing's extents, in millimetres of paper. Without it, 5."),
            "plot_style": text("The CTB plot style to draw through, to show what prints, by its file name, such as `monochrome.ctb`, found in the plot style folders as plot finds it. Not given with `monochrome`. Without either, each object draws in its own colour."),
            "monochrome": flag("Draw exactly as plot draws with no plot style named: every pen black at its object's own lineweight. Not given with `plot_style`."),
            "plot_styles": plot_styles(),
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["source"],
    )
}

fn run_render(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, true)?;
    let request: RenderRequest = parse("render", input)?;
    Ok(done_warned(render_with(&request, &settings)))
}

// display-list

fn display_list_does() -> String {
    "Draws a sheet to its display list, the shapes the PNG, the PDF and the viewer all draw \
     from, and gives it as JSON without writing a file: each item's colour index and its \
     strokes, with their lineweight in millimetres of paper and their dashes laid out, or its \
     filled contours, in drawing units, with each line of text as the characters it reads. \
     `source` is a sheet description, a DWG or a drawing set's folder, whose sheet `sheet` \
     names (its first without it); the reply lists the set's sheets, the paper, the extents \
     and how many millimetres of paper a drawing unit spans, and what each of the sheet's \
     objects is: its ID, name, type and layer, and an insert's block and attributes. Curves \
     are drawn within `tolerance` drawing units, by default a 100,000th of the sheet's larger \
     side. With `entry`, a drawing set is drawn as it was just after that entry of its journal, \
     rebuilt in memory from the undo states, or the reply says why it cannot be. A sheet drawn \
     in several layouts draws the one `layout` names, its first without it, and the reply lists \
     them all under `layouts`. With `snaps`, the reply also gives `snaps`: every point \
     AutoCAD's object snaps find on the objects the page draws itself, each with its `object` \
     by its place, its `snap`, `which` of the object's points and, for an intersection, the \
     other object by its place under `with`, as `query snap` names them, and `at`. With \
     `origins`, on a drawing set's sheet, each object also gives `from`, `template` or `row`, \
     where a change set writes an edit to it, and the reply gives `template`: the sheet's \
     template, its `path` as the rows write it, and every sheet built from it under `sheets`, \
     which an edit to one of the template's objects reaches, and on a sheet drawn in a layout \
     `space`, the space an object drawn on the page goes in, as an add names it. With \
     `values`, each object also gives `values`, what quick properties edit: a text's or an \
     mtext's `contents`, a circle's or an arc's `radius`, and an insert's `attributes`, each \
     with its `tag` and, for a tag its block defines more than once, its `occurrence`; and on a \
     drawing set's sheet each text value says where it is `written`: `own`, by the object \
     itself; `row_value`, by the sheet's row, naming the `row_value` and how many places on the \
     sheet it `fills`; `row_values`, by the sheet's row as the row values its template joins \
     with plain text in a list of parts, listing each under `row_values` with its `name` and \
     how many places it `fills`; or `unset`, an attribute the insert gives no value for, which \
     shows its default. With `change_set`, the same JSON `change` takes, a drawing set's sheet is \
     drawn as the change set would leave it, applied in memory to the set as it is, as a dry run \
     applies it; nothing is written and nothing goes in the journal. The reply then gives \
     `preview`: under `change`, the dry run's reply, each sheet it reaches with whether it would \
     take its part or why not; under `checks`, the checker's reply on the sheets it would land \
     on, read from the records as it would write them; under `objects`, each object it would \
     add, change or delete, by its ID and `how`; and on the sheet drawn, the places among the \
     display list's items it would add (`added`) and change (`changed`), what each object it \
     would change or delete draws now where it would draw no more (`was`, its `items`), and a \
     box round each check the sheet would fail (`clouds`). A loop sheet's display list runs to \
     megabytes: it is for the viewer to draw, not to read."
        .to_owned()
}

fn display_list_input() -> Value {
    // The change set's own schema, its definitions moved up beside the
    // input's, where its references point, as `change` has it.
    let mut change_set = change_set_schema();
    let defs = change_set
        .as_object_mut()
        .and_then(|schema| {
            schema.remove("$schema");
            schema.remove("$defs")
        })
        .unwrap_or_else(|| json!({}));
    if let Some(schema) = change_set.as_object_mut() {
        schema.insert(
            "description".to_owned(),
            json!("Draw the drawing set's sheet as this change set would leave it, the same JSON change takes, applied in memory to the set as it is; nothing is written and nothing goes in the journal, and the reply gives its preview. Without it, the set as it is."),
        );
    }
    let mut schema = display_list_schema(change_set);
    schema["$defs"] = defs;
    // A change set is drawn on the set as it is, never at an entry; and
    // all_or_none says how it is applied, so it goes with one.
    schema["not"] = json!({"required": ["entry", "change_set"]});
    schema["dependentRequired"] = json!({"all_or_none": ["change_set"]});
    schema
}

fn display_list_schema(change_set: Value) -> Value {
    input(
        json!({
            "source": text("The record to draw: a sheet description, a DWG when the file ends .dwg, or a drawing set's folder."),
            "sheet": text("Which of the drawing set's sheets to draw, by name. Without it, the set's first."),
            "layout": text("Which layout of a sheet drawn in several to draw, by its name, in any case. Without it, the sheet's first."),
            "tolerance": number("How far, in drawing units, a curve's chords may stray from it. Without it, a 100,000th of the sheet's larger side."),
            "entry": {
                "description": "Draw the drawing set as it was just after this entry of its journal, by its number, rebuilt in memory from the undo states; nothing is written. Without it, as it is now.",
                "type": "integer",
                "minimum": 1,
            },
            "snaps": {
                "description": "Give each object's object snaps with the display list: every point its own snaps find (endpoint, midpoint, center, quadrant, insertion and node) and every intersection with another object, on the objects the page draws itself. Without it, none.",
                "type": "boolean",
            },
            "origins": {
                "description": "On a drawing set's sheet, say where each object is written, `from` its sheet's `template` or its `row`, and give `template`: the template's path and every sheet built from it, which an edit to a template's object reaches; and on a sheet drawn in a layout, `space`: the space an object drawn on the page goes in, `paper` or the layout's name. Without it, none of them.",
                "type": "boolean",
            },
            "values": {
                "description": "Give each object's `values`, what quick properties edit: a text's or an mtext's contents, a circle's or an arc's radius, and an insert's attribute values; and on a drawing set's sheet where each text value is `written`: by the object itself (`own`), by the sheet's row (`row_value`, with the value's name and how many places on the sheet it `fills`), by the row values its template joins with plain text in a list of parts (`row_values`, each with its `name` and how many places it `fills`), or nowhere (`unset`, an attribute that shows its default). Without it, none.",
                "type": "boolean",
            },
            "change_set": change_set,
            "all_or_none": {
                "description": "With change_set: apply it as a proposal is kept, on every sheet it reaches or on none, so a sheet that would take its part while another refuses is shown held back. Without it, as change applies it.",
                "type": "boolean",
            },
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["source"],
    )
}

fn run_display_list(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    // Both keys given, even `entry: null`, which reads as left out: a change
    // set is never drawn at an entry, as the schema says.
    if input.contains_key("entry") && input.contains_key("change_set") {
        return Err(Error::new(ErrorCode::BadValue, NOT_AT_AN_ENTRY).at_field("change_set"));
    }
    if input.contains_key("all_or_none") && !input.contains_key("change_set") {
        return Err(Error::usage(
            "`all_or_none` says how a change set is applied: it goes with `change_set`",
        )
        .at_field("all_or_none"));
    }
    let settings = take_settings(&mut input, base, false)?;
    let request: DisplayListRequest = parse("display-list", input)?;
    Ok(done_warned(display_list_with(&request, &settings)))
}

// plot

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlotInput {
    source: PathBuf,
    #[serde(default)]
    output: Option<PathBuf>,
    #[serde(default)]
    overwrite: bool,
    #[serde(default)]
    plot_style: Option<String>,
}

fn plot_does() -> String {
    format!(
        "Plots a sheet description, or a DWG, to a PDF as AutoCAD plots it to PDF: the page is \
         the sheet's size, its extents at 1:1 (a sheet drawn 17 by 11 inches plots on a page of \
         exactly 431.8 by 279.4 mm); each pen's colour and lineweight come from the CTB plot \
         style `plot_style` names, found in the plot style folders, or with none from the \
         {BUILT_IN_PLOT_STYLE} style, every pen black at its object's own lineweight; text draws \
         as AutoCAD draws it and can be searched and copied. It goes beside the source with a \
         .pdf extension unless `output` names another path, and is stamped with every file it \
         was drawn from. An existing file is never replaced without `overwrite`, and \
         `overwrite` replaces only a PDF AgentDrafter plotted. Given a folder, plots every sheet \
         of the drawing set in it, each its template filled in from its row, into the output \
         folder its settings ({SET_SETTINGS_FILE}) name, as <sheet>.pdf. A sheet that fails \
         writes nothing and the others are still plotted; the reply gives each sheet's outcome \
         under `sheets`, and is not ok when any sheet failed. A sheet drawn in several layouts \
         plots a page for each, in the order of their tabs, each on its own paper, and the reply \
         lists them under `layouts`."
    )
}

fn plot_input() -> Value {
    input(
        json!({
            "source": text("The record to plot: a sheet description, a DWG when the file ends .dwg, or a drawing set's folder."),
            "output": text("Where to write the PDF of one sheet description or DWG. Without it, beside the source."),
            "overwrite": flag("Replace a PDF already there, which is allowed only when AgentDrafter plotted it."),
            "plot_style": text("The CTB plot style to plot through, by its file name, such as `monochrome.ctb`, found in the plot style folders. Without it, the built-in monochrome style."),
            "plot_styles": plot_styles(),
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["source"],
    )
}

fn run_plot(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, true)?;
    let PlotInput {
        source,
        output,
        overwrite,
        plot_style,
    } = parse("plot", input)?;
    if is_set(
        &source,
        output.as_deref(),
        "PDFs",
        "one sheet description or DWG",
    )? {
        let request = PlotSetRequest {
            set: source,
            overwrite,
            plot_style,
        };
        return Ok(sent_warned(plot_set_with(&request, &settings)));
    }
    let request = PlotRequest {
        source,
        output,
        overwrite,
        plot_style,
    };
    Ok(done_warned(plot_with(&request, &settings)))
}

// stale

fn stale_does() -> String {
    "Says whether a PNG AgentDrafter rendered or a PDF it plotted, `output`, is stale: whether \
     any file its stamp names (the sheet description, DWG or template, and the files it draws \
     on) has changed since, or is gone. The files are found from the output's folder; `source` \
     says where the first, the record, is now."
        .to_owned()
}

fn stale_input() -> Value {
    input(
        json!({
            "output": text("The output to check: a PNG AgentDrafter rendered or a PDF it plotted."),
            "source": text("Where the record is now, the first file the stamp lists. Without it, where the stamp names it, found from the output's folder."),
        }),
        &["output"],
    )
}

fn run_stale(input: Map<String, Value>, _: &Settings, _: Interface) -> Result<JsonReply, Error> {
    let request: StaleRequest = parse("stale", input)?;
    Ok(done(check_stale(&request)))
}

// query

fn query_does() -> String {
    "Asks a drawing set, or one sheet description, what is on its sheets, from the records, \
     with nothing selected by coordinates but a box; `query` says what, by its `kind`. find \
     lists the objects matching every part given, each with its ID and where it is written, \
     `window` and `crossing` the objects within a box or crossing it, as AutoCAD's selection \
     windows take them, and `touching` those whose geometry meets an object's, with where; \
     attributes \
     gives each matching insert's attributes as JSON from tag to value; blocks lists the \
     blocks each sheet inserts; measure gives the distance between a point of two objects on \
     one sheet, each named by ID or name, by default an insert's or a text's position, a \
     line's from, a circle's centre; extents gives the box round what each sheet draws; \
     snap gives the points AutoCAD's object snaps find on an object named by ID or name, \
     inserts' blocks among them: endpoint, midpoint, center, quadrant, insertion and node, \
     intersection and apparent_intersection `with` another object, and perpendicular, \
     tangent and nearest `from` a snap of another object, each named by `which` to ask for \
     it again; geometry gives each matching object's length, area, extents and angles. \
     Lengths are in the declared unit. `id`, `name`, `block` and `tag` match whole, in any \
     case, and `value` and `text` whole, case and all; with `match` `contains`, each matches \
     what holds it, in any case. A query that matches nothing is ok and says so; `sheet` \
     looks at one sheet only."
        .to_owned()
}

fn query_input() -> Value {
    input(
        json!({
            "source": text("A drawing set's folder, or one sheet description."),
            "sheet": text("Only the sheet of this name, of the set's sheets. Without it, every sheet."),
            "query": query_schema(),
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["source", "query"],
    )
}

/// The schema of what a query asks, one choice per `kind`.
fn query_schema() -> Value {
    let selection = |kind: &str, what: &str| {
        json!({
            "description": what,
            "type": "object",
            "additionalProperties": false,
            "required": ["kind"],
            "properties": {
                "kind": { "const": kind },
                "id": text("The object's ID, such as `DEMO-001/border`."),
                "name": text("The object's name, such as `title-block`."),
                "block": text("The block an insert places."),
                "tag": text("An attribute's tag: an insert carrying that attribute, or, with `value`, the attribute the value is looked for in."),
                "value": text("An attribute's value, in any of an insert's attributes, or in the one `tag` names."),
                "text": text("What a text or an mtext says."),
                "match": {
                    "description": "How the parts match: `exact`, the whole value, or `contains`, a value that holds what is given, in any case. Without it, exact.",
                    "enum": ["exact", "contains"],
                },
                "window": selection_box("Objects wholly within this box, as AutoCAD's selection window takes them: everything they draw, text and inserts' blocks among it, inside it, edges included."),
                "space": {
                    "description": "Objects in this space, on a sheet drawn in a layout: `model`, or a layout's paper space by its name, or `paper` on a sheet drawn in one. A box looks at each object in its own space, so on a sheet drawn in several layouts a box on the paper names its layout here.",
                    "type": "string",
                    "minLength": 1,
                },
                "crossing": selection_box("Objects within this box or crossing it, as AutoCAD's crossing selection takes them: something they draw inside it or across its edge, a line, the outline of a filled area, a hatch's boundary or pattern, each wide segment's band, seams included; a box wholly inside an area crosses nothing, but inside a wide arc's band."),
                "touching": {
                    "description": "Objects whose geometry meets this object's, on each sheet it is on: crossing it, or an end of one lying on the other; each found with where it meets it, `part` its own and `with_part` the object's.",
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "id": text("The object's ID."),
                        "name": text("The object's name."),
                    },
                },
            },
        })
    };
    let end = |what: &str| {
        json!({
            "description": what,
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "id": text("The object's ID."),
                "name": text("The object's name, which only one sheet of those looked at may give."),
                "point": text("Which of the object's points, named as its key in the sheet description: `position`, `from`, `to`, `center`, `points[2]`. Without it, the object's first."),
            },
        })
    };
    let bare = |kind: &str, what: &str| {
        json!({
            "description": what,
            "type": "object",
            "additionalProperties": false,
            "required": ["kind"],
            "properties": { "kind": { "const": kind } },
        })
    };
    json!({
        "description": "What to ask, by its `kind`: find, blocks, attributes, measure, extents, layers, snap, geometry or layout.",
        "oneOf": [
            selection("find", "The objects that match every part given; with none given, every object."),
            bare("blocks", "The blocks each sheet inserts."),
            selection("attributes", "The attributes of each insert that matches every part given."),
            {
                "description": "The distance between a point of one object and a point of another on the same sheet.",
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "from", "to"],
                "properties": {
                    "kind": { "const": "measure" },
                    "from": end("The first end: an object by `id` or `name`, and which of its points."),
                    "to": end("The second end, as the first."),
                },
            },
            bare("extents", "The box round everything each sheet draws."),
            bare(
                "layers",
                "The layers of each sheet, with their colour, linetype, lineweight and states, and \
                 how many of its objects sit on each.",
            ),
            snap_schema(),
            selection(
                "geometry",
                "The length, area, extents and angles of each object that matches every part \
                 given, as AutoCAD's LIST and AREA give them; with none given, every object's.",
            ),
            bare(
                "layout",
                "Each sheet's paper-space layouts, if it is drawn in any, in the order of their \
                 tabs: each one's name, its paper and page setup, how many objects are in its \
                 paper space and in the model space its sheet's layouts share, model space's \
                 extents, and its viewports, each with its window on the paper, its view centre, \
                 its scale, whether it is locked, the boundary it is clipped to, the layers \
                 frozen in it, and the part of model space it shows.",
            ),
        ],
    })
}

/// The schema of a box a selection looks within: two opposite corners.
fn selection_box(what: &str) -> Value {
    json!({
        "description": format!("{what} Two opposite corners, [[x1, y1], [x2, y2]], in the declared unit, in either order; a corner may carry a height, which is not looked at."),
        "type": "array",
        "minItems": 2,
        "maxItems": 2,
        "items": {
            "type": "array",
            "minItems": 2,
            "maxItems": 3,
            "items": { "type": "number" },
        },
    })
}

/// The schema of a snap: an object by ID or name, the snap, and which
/// point, with the other object or the point it is taken from.
fn snap_schema() -> Value {
    let kinds: Vec<&str> = crate::geometry::SnapKind::ALL
        .iter()
        .map(|k| k.name())
        .collect();
    let point = |kind: Option<&str>, what: &str, from: Value| {
        let mut properties = json!({
            "id": text("The object's ID."),
            "name": text("The object's name, which only one sheet of those looked at may give."),
            "snap": {
                "description": "The snap: endpoint, midpoint, center, quadrant, insertion and node of the object by itself; intersection and apparent_intersection with `with`; perpendicular, tangent and nearest from `from`. Without it, every snap of the object by itself.",
                "enum": kinds,
            },
            "which": text("Which of the object's points of the snap, as a reply names it: `points[2]`, `segments[1]`, `90`, `objects[3].to` of an insert's block. Without it, every point of the snap."),
            "with": {
                "description": "The other object of an intersection or an apparent intersection, by `id` or `name`.",
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": text("The object's ID."),
                    "name": text("The object's name."),
                },
            },
            "from": from,
        });
        let mut required = Vec::new();
        if let Some(kind) = kind {
            properties["kind"] = json!({ "const": kind });
            required.push("kind");
        }
        json!({
            "description": what,
            "type": "object",
            "additionalProperties": false,
            "required": required,
            "properties": properties,
        })
    };
    let from = point(
        None,
        "The point a perpendicular, a tangent or a nearest point is taken from: a snap of another object that names one point, written as a snap is.",
        json!({ "type": "object" }),
    );
    point(
        Some("snap"),
        "The points AutoCAD's object snaps find on an object, by `id` or `name`, an insert's in its block's own geometry too, each in the declared unit and named by `which`.",
        from,
    )
}

fn run_query(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: QueryRequest = parse("query", input)?;
    Ok(done(query_with(&request, &settings)))
}

// operations

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoInput {}

fn operations_does() -> String {
    "Lists every operation a change set can hold: its name, its AutoCAD alias, what it does and \
     its JSON Schema. `schema` with `kind` `change-set` gives the schema of a whole change set."
        .to_owned()
}

fn operations_input() -> Value {
    input(json!({}), &[])
}

fn run_operations(
    input: Map<String, Value>,
    _: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let NoInput {} = parse("operations", input)?;
    Ok(done(Ok(operations())))
}

// change

fn change_does() -> String {
    "Applies a change set, `change_set`, such as {\"operations\": [{\"op\": \"move\", \
     \"target\": {\"id\": \"DEMO-001/border\"}, \"by\": [0.1, 0, 0]}]}, to the drawing set in \
     the folder `set`, to its own files at once, and builds again the DWGs of the sheets it \
     touched. Objects are named by ID or name, never by coordinates. An edit to a template's \
     object edits the template, so every sheet built from it follows; an object added to a \
     sheet goes in its row. Each sheet takes its part fully or not at all, a template edit \
     lands only if every sheet of its template does, and a change set that lands on some \
     sheets and fails on others marks the set inconsistent until a later change set names \
     them under fixes. With `dry_run` it reports what it would do and writes nothing. The \
     reply is not ok when a sheet failed. Once it has landed, the reply's checks are the \
     checker's reply on the sheets it landed on, read back from their saved files; a failed \
     check does not stop it."
        .to_owned()
}

fn change_input() -> Value {
    // The change set's own schema, its definitions moved up beside the
    // input's, where its references point.
    let mut change_set = change_set_schema();
    let defs = change_set
        .as_object_mut()
        .and_then(|schema| {
            schema.remove("$schema");
            schema.remove("$defs")
        })
        .unwrap_or_else(|| json!({}));
    let mut schema = input(
        json!({
            "set": text("The drawing set's folder."),
            "change_set": change_set,
            "dry_run": flag("Report what the change set would do, and write nothing."),
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["set", "change_set"],
    );
    schema["$defs"] = defs;
    schema
}

fn run_change(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: ChangeRequest = parse("change", input)?;
    Ok(sent_warned(change_with(&request, &settings)))
}

// journal

fn journal_does() -> String {
    format!(
        "Lists the journal of the drawing set in the folder `set`, kept in its folder \
         ({JOURNAL_FILE}) whether or not it is a git repository: every change set, with its \
         operations, the sheets it landed and failed on and the files it wrote; every failed \
         attempt, which changed nothing; and every undo. It says which change sets can still be \
         undone: the undo states of the {KEPT_UNDO_STATES} newest change sets are kept, and an \
         older one's are pruned."
    )
}

fn journal_input() -> Value {
    input(
        json!({ "set": text("The drawing set's folder.") }),
        &["set"],
    )
}

fn run_journal(input: Map<String, Value>, _: &Settings, _: Interface) -> Result<JsonReply, Error> {
    let request: JournalRequest = parse("journal", input)?;
    Ok(sent(journal(&request)))
}

// undo

fn undo_does() -> String {
    "Undoes a change set of the drawing set in the folder `set`, named by its entry's number in \
     the journal, `change_set`, and every later change set with it, the newest first: every \
     file they wrote, records, DWGs and the inconsistent mark, goes back byte for byte as it \
     was before, and a file they made is taken away. It refuses, changing nothing and naming \
     the files, when a file was changed since by hand, and when a change set's undo state was \
     pruned. With `dry_run` it reports what it would do and writes nothing."
        .to_owned()
}

fn undo_input() -> Value {
    input(
        json!({
            "set": text("The drawing set's folder."),
            "change_set": {
                "description": "The change set to undo, by its entry's number in the journal, such as 3.",
                "type": "integer",
                "minimum": 0,
            },
            "dry_run": flag("Report what the undo would do, and write nothing."),
        }),
        &["set", "change_set"],
    )
}

fn run_undo(input: Map<String, Value>, _: &Settings, _: Interface) -> Result<JsonReply, Error> {
    let request: UndoRequest = parse("undo", input)?;
    Ok(sent(undo(&request)))
}

// accept

fn accept_does() -> String {
    "Accepts the working revision of the drawing set in the folder `set`, as the reviewer does \
     in the viewer: every change set still in place is marked issued, and an issued change set \
     cannot be undone; the journal records the accept, the interface it came through, the \
     change sets it issued and the fingerprint of each file of the set, so a PNG or a PDF made \
     from those files says it came from an issued revision. It is refused while the set is \
     inconsistent. It checks every sheet first, and when checks fail it lists them and issues \
     nothing unless `despite_failed_checks` says to accept anyway. `newest_seen` refuses it \
     when the journal has moved on past the entry the caller last looked at. With `dry_run` \
     it reports what it would do and writes nothing."
        .to_owned()
}

fn accept_input() -> Value {
    input(
        json!({
            "set": text("The drawing set's folder."),
            "despite_failed_checks": flag("Accept even when checks fail, once the caller has seen them: without it, an accept that finds failed checks lists them and issues nothing."),
            "newest_seen": {
                "description": "The journal's newest entry the caller has looked at, by its number, such as 7, or 0 for an empty journal: the accept is refused if the journal holds a newer one, so a change set that landed meanwhile is not issued unseen.",
                "type": "integer",
                "minimum": 0,
            },
            "dry_run": flag("Report what the accept would do, and write nothing."),
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["set"],
    )
}

fn run_accept(
    mut input: Map<String, Value>,
    base: &Settings,
    via: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: AcceptRequest = parse("accept", input)?;
    Ok(sent_warned(accept_with(&request, &settings, via)))
}

// proposals

fn propose_does() -> String {
    format!(
        "Proposes a change set, `change_set`, as `change` takes it, to the drawing set in the \
         folder `set`, where `change` would land it at once: it waits, drawn for the drafter to \
         keep or discard, and nothing changes until it is kept. It is checked as `change` \
         checks it, by a dry run on the set as it is, and refused, unrecorded, when the set \
         would refuse it. It is recorded in the set's folder ({PROPOSALS_FOLDER}), numbered, \
         with who proposed it, `by`, in the caller's words (by default the interface it came \
         through), when, and the journal's newest entry it was made against. The reply gives \
         its number and what keeping it would do now."
    )
}

fn propose_input() -> Value {
    let mut schema = change_input();
    let properties = schema["properties"]
        .as_object_mut()
        .expect("change's input has properties");
    properties.remove("dry_run");
    properties.insert("by".to_owned(), by("proposes it", "Assistant"));
    schema
}

fn run_propose(
    mut input: Map<String, Value>,
    base: &Settings,
    via: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: ProposeRequest = parse("propose", input)?;
    Ok(done_warned(propose_with(&request, &settings, via)))
}

fn proposals_does() -> String {
    "Lists the proposals of the drawing set in the folder `set` that are waiting, the oldest \
     first: each one's number, who proposed it and when, the journal's entry it was made \
     against and the change sets and undos that landed since, its change set, and the sheets \
     it reaches. A proposal kept or discarded is no longer listed; the journal records it."
        .to_owned()
}

fn proposals_input() -> Value {
    input(
        json!({ "set": text("The drawing set's folder.") }),
        &["set"],
    )
}

fn run_proposals(
    input: Map<String, Value>,
    _: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let request: ProposalsRequest = parse("proposals", input)?;
    Ok(done(proposals(&request)))
}

fn keep_does() -> String {
    "Keeps a proposal of the drawing set in the folder `set`, by its number, `proposal`: its \
     change set lands as `change` lands one, on the set as it stands now, each sheet fully or \
     not at all, and the reply is `change`'s. Its journal entry says who proposed it and who \
     kept it, `by`, in the caller's words (by default the interface it came through); it is \
     undone and accepted as any change set, and undoing it does not bring the proposal back. \
     When the set refuses it, the attempt is recorded as `change` records one and the \
     proposal goes on waiting. A proposal kept or discarded already is refused."
        .to_owned()
}

fn keep_input() -> Value {
    input(
        json!({
            "set": text("The drawing set's folder."),
            "proposal": proposal_number("keep"),
            "by": by("keeps it", DRAFTER),
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["set", "proposal"],
    )
}

fn run_keep(
    mut input: Map<String, Value>,
    base: &Settings,
    via: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: KeepRequest = parse("keep", input)?;
    Ok(sent_warned(keep_with(&request, &settings, via)))
}

fn discard_does() -> String {
    "Discards a proposal of the drawing set in the folder `set`, by its number, `proposal`: it \
     ends without landing, and the journal records who discarded it, `by`, in the caller's \
     words (by default the interface it came through). No file of the set changes, so there \
     is nothing to undo, and an undo passes over the discard. A proposal kept or discarded \
     already is refused."
        .to_owned()
}

fn discard_input() -> Value {
    input(
        json!({
            "set": text("The drawing set's folder."),
            "proposal": proposal_number("discard"),
            "by": by("discards it", DRAFTER),
        }),
        &["set", "proposal"],
    )
}

fn run_discard(
    input: Map<String, Value>,
    _: &Settings,
    via: Interface,
) -> Result<JsonReply, Error> {
    let request: DiscardRequest = parse("discard", input)?;
    Ok(done(discard(&request, via)))
}

/// The schema of who proposes, keeps or discards.
fn by(does: &str, example: &str) -> Value {
    json!({
        "description": format!(
            "Who {does}, in the caller's words, such as `{example}`, which the journal and the \
             viewer show; by default the interface it came through, such as `agent (MCP)`."
        ),
        "type": "string",
        "minLength": 1,
        "maxLength": BY_LONGEST,
        "pattern": BY_PATTERN,
    })
}

/// The schema of the proposal a keep or a discard takes.
fn proposal_number(does: &str) -> Value {
    json!({
        "description": format!("The proposal to {does}, by its number, such as 2."),
        "type": "integer",
        "minimum": 1,
    })
}

// check

fn check_does() -> String {
    "Checks a drawing set, or one sheet description, `source`, from its saved files, against \
     the default checks: paper_size, the sheet's extents are its declared paper's size; \
     off_sheet, every object draws on the paper, from [0, 0]; text_overflow, every text and \
     attribute of an insert draws within the cell its block declares for it; and \
     text_on_text, no two texts' drawn boxes overlap. A sheet that declares no paper gets one \
     paper_size finding that says so. Each sheet is drawn through the drawing pipeline, so \
     text is measured as its font draws it. Each finding names the sheet, the rule and the \
     object, by ID and name, and says what is wrong in numbers in the declared unit, within a \
     tolerance of 0.01 mm on paper. A failed check is ok and listed under findings; `sheets` \
     checks only the sheets it names."
        .to_owned()
}

fn check_input() -> Value {
    input(
        json!({
            "source": text("A drawing set's folder, or one sheet description."),
            "sheets": {
                "description": "Only the sheets of these names, in any case. Without them, every sheet.",
                "type": "array",
                "items": { "type": "string" },
            },
            "fonts": fonts(),
            "no_system_fonts": no_system_fonts(),
        }),
        &["source"],
    )
}

fn run_check(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: CheckRequest = parse("check", input)?;
    Ok(done_warned(check_with(&request, &settings)))
}

// What every command shares.

// import-blocks

fn import_blocks_does() -> String {
    format!(
        "Imports the blocks of a DWG or a DXF, `drawing`, into the block library `library`, as \
         AutoCAD's DesignCenter (ADCENTER) does: every named block, or with `blocks` those \
         named, each with every block it nests, with their objects, attribute definitions, \
         layers, linetypes, hatch patterns and text styles. The drawing is only read. The \
         library is made in the drawing's unit when it is not there; one that is keeps every \
         byte it had, takes each new block after its last, keeps its own definition of a layer, \
         linetype, hatch pattern or text style it holds already, and must be in the drawing's \
         unit. A block it holds already is refused unless `replace`, which replaces its whole \
         definition. A drawing that states no unit takes `units`. What the core cannot carry, \
         such as a dimension or an object in a true colour, is left out and counted block by \
         block under not_carried. A drawing set's library, which its templates name, is \
         refused: it takes blocks through a change set's import_blocks operation, so its \
         journal and undo hold them; the set is found by its settings ({SET_SETTINGS_FILE}) in \
         the library's folder or one above. With `dry_run` it reports what it would write and \
         writes nothing."
    )
}

fn import_blocks_input() -> Value {
    input(
        json!({
            "drawing": text("The DWG or DXF to read the blocks of, told apart by its extension."),
            "library": text("The block library to write the blocks into: made when it is not there."),
            "blocks": {
                "description": "Only the blocks of these names, in any case, each with every block it nests. Without them, every named block of the drawing.",
                "type": "array",
                "items": { "type": "string" },
            },
            "replace": flag("Replace the whole definition of a block the library holds already, rather than refuse."),
            "units": {
                "description": "The unit the drawing's blocks are drawn in, for a drawing that states none a block library takes.",
                "enum": Unit::ALL.map(Unit::symbol),
            },
            "dry_run": flag("Report what the import would write, and write nothing."),
        }),
        &["drawing", "library"],
    )
}

fn run_import_blocks(
    input: Map<String, Value>,
    _: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let request: ImportBlocksRequest = parse("import-blocks", input)?;
    Ok(done(import_blocks(&request)))
}

/// The drawing set a command's input reads, if it names one: the folder its
/// `set` names; or the set its `source`, or with none its `output`, is, or
/// is in, the nearest folder above it that holds a set's settings. A DWG
/// its build mark names the set of, when it is still that set's, is that
/// set's wherever its output folder is.
fn set_read(input: &Value) -> Option<PathBuf> {
    let named = ["set", "source", "output"]
        .iter()
        .find_map(|key| input.get(key))?
        .as_str()?;
    let path = Path::new(named);
    if SourceKind::of(path) == SourceKind::Dwg && path.is_file() {
        if let Ok(set) = set_of_dwg(path) {
            return Some(set.folder);
        }
    }
    let set = path
        .ancestors()
        .find(|folder| folder.join(SET_SETTINGS_FILE).is_file())?;
    Some(if set.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        set.to_owned()
    })
}

/// Whether `source` is a drawing set: a folder, whose settings say where
/// its `outputs` go, so an `output` path, which is for `one` source, is
/// refused with it. `build` and `plot` both ask.
fn is_set(source: &Path, output: Option<&Path>, outputs: &str, one: &str) -> Result<bool, Error> {
    if !source.is_dir() {
        return Ok(false);
    }
    if output.is_some() {
        return Err(Error::usage(format!(
            "a drawing set's {outputs} go to the folder its settings ({SET_SETTINGS_FILE}) name \
             under `output`; an output path (--output) is for {one}"
        )));
    }
    Ok(true)
}

/// The settings for one command: the font folders the input's `fonts`
/// names, then those of `base`, then the system's font folders of `base`
/// unless its `no_system_fonts` is true; and, when the command reads plot
/// styles, as `plot` and `render` do, the plot style folders its
/// `plot_styles` names, then those of `base`. All are taken out of the
/// input.
fn take_settings(
    input: &mut Map<String, Value>,
    base: &Settings,
    plots: bool,
) -> Result<Settings, Error> {
    let mut settings = base.clone();
    settings
        .font_folders
        .splice(0..0, folders(input.remove("fonts"), "fonts")?);
    match input.remove("no_system_fonts") {
        None | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => settings = settings.without_system_fonts(),
        Some(_) => {
            return Err(
                Error::usage("`no_system_fonts` is true or false").at_field("no_system_fonts")
            )
        }
    }
    if plots {
        settings
            .plot_style_folders
            .splice(0..0, folders(input.remove("plot_styles"), "plot_styles")?);
    }
    Ok(settings)
}

/// The folders a `fonts` or `plot_styles` value lists.
fn folders(value: Option<Value>, key: &str) -> Result<Vec<PathBuf>, Error> {
    match value {
        None => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value).map_err(|_| {
            Error::usage(format!("`{key}` is a list of folders, each a path")).at_field(key)
        }),
    }
}

/// Reads `input` as the request `command` takes. A field the request does
/// not know, or a value of the wrong shape, is a `usage` error.
fn parse<T: DeserializeOwned>(command: &str, input: Map<String, Value>) -> Result<T, Error> {
    serde_json::from_value(Value::Object(input)).map_err(|e| {
        Error::usage(format!(
            "the input to `{command}` does not fit its schema: {e}"
        ))
    })
}

/// A command's reply from its core entry point's outcome.
fn done<T: Serialize>(outcome: Result<T, Error>) -> JsonReply {
    sent(Reply::from(outcome))
}

/// A command's reply from its core entry point's outcome, with the
/// warnings its result holds at its top.
fn done_warned<T: Serialize + Warns>(outcome: Result<T, Error>) -> JsonReply {
    sent_warned(Reply::from(outcome))
}

/// A command's reply from its core entry point's reply, with the warnings
/// its result holds at its top.
fn sent_warned<T: Serialize + Warns>(reply: Reply<T>) -> JsonReply {
    sent(reply.warned())
}

/// A command's reply from its core entry point's reply.
fn sent<T: Serialize>(reply: Reply<T>) -> JsonReply {
    Reply {
        ok: reply.ok,
        rolled_back: reply.rolled_back,
        warnings: reply.warnings,
        // A reply holds only strings, numbers, lists and string-keyed maps,
        // which always serialize.
        result: reply.result.map(|result| {
            serde_json::value::to_raw_value(&result).expect("a reply serializes to JSON")
        }),
        error: reply.error,
    }
}

/// The schema of a command's input: an object of `properties`, holding
/// those `required` and nothing else.
fn input(properties: Value, required: &[&str]) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
    })
}

fn text(what: &str) -> Value {
    json!({ "description": what, "type": "string" })
}

fn flag(what: &str) -> Value {
    json!({ "description": what, "type": "boolean" })
}

fn number(what: &str) -> Value {
    json!({ "description": what, "type": "number" })
}

fn pixels(what: &str) -> Value {
    json!({ "description": what, "type": "integer", "minimum": 0, "maximum": u32::MAX })
}

fn fonts() -> Value {
    json!({
        "description": format!(
            "Folders to read fonts from, searched before those the {FONTS_VARIABLE} environment \
             variable lists on the machine the command runs on, and before the system's own font \
             folders, which are searched last, for TrueType fonts only."
        ),
        "type": "array",
        "items": { "type": "string" },
    })
}

fn no_system_fonts() -> Value {
    flag(&format!(
        "Leave the system's own font folders out, as {SYSTEM_FONTS_VARIABLE}=0 does, so that \
         TrueType fonts are read only from the folders named and what is measured does not \
         depend on the fonts the machine has installed."
    ))
}

fn plot_styles() -> Value {
    json!({
        "description": format!(
            "Folders to read CTB plot styles from, searched before those the \
             {PLOT_STYLES_VARIABLE} environment variable lists on the machine the command runs on."
        ),
        "type": "array",
        "items": { "type": "string" },
    })
}

/// What kind of JSON value `value` is, in words.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "true or false",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}
