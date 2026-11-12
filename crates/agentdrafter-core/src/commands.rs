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
//! as `--fonts` and `--plot-styles` come before the environment's.
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
use crate::check::{check_with, CheckRequest};
use crate::display_list::{display_list_with, DisplayListRequest};
use crate::error::Error;
use crate::interface::Interface;
use crate::plot::{plot_set_with, plot_with, PlotRequest, PlotSetRequest, BUILT_IN_PLOT_STYLE};
use crate::query::{query_with, QueryRequest};
use crate::render::{render_with, RenderRequest};
use crate::reply::Reply;
use crate::set::{build_set_with, SetBuildRequest, SET_SETTINGS_FILE};
use crate::settings::{Settings, FONTS_VARIABLE, PLOT_STYLES_VARIABLE};
use crate::stamp::{check_stale, StaleRequest};
use crate::text::measure::{measure_text_with, MeasureTextRequest};
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
    /// command refuses and does nothing.
    pub fn run(&self, input: Value, settings: &Settings, via: Interface) -> JsonReply {
        let rolled_back = match set_read(&input).map(|set| recover(&set)) {
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
        name: "check",
        does: check_does,
        input: check_input,
        run: run_check,
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
        return Ok(sent(build_set_with(&request, &settings)));
    }
    let request = BuildRequest {
        description: source,
        output,
        overwrite,
    };
    Ok(done(build_with(&request, &settings)))
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
    Ok(done(measure_text_with(&request, &settings)))
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
     AgentDrafter rendered."
        .to_owned()
}

fn render_input() -> Value {
    input(
        json!({
            "source": text("The record to render: a sheet description, or a DWG when the file ends .dwg."),
            "output": text("Where to write the PNG. Without it, beside the source."),
            "overwrite": flag("Replace a PNG already there, which is allowed only when AgentDrafter rendered it."),
            "dpi": number("How many pixels an inch of paper spans. Without it, 150."),
            "scale": number("The plot scale, such as 1 for 1:1 and 0.02 for 1:50. Not given with `width` or `height`."),
            "width": pixels("How many pixels wide the image is, the drawing fitted inside it."),
            "height": pixels("How many pixels high the image is, the drawing fitted inside it."),
            "margin": number("The margin round the drawing's extents, in millimetres of paper. Without it, 5."),
            "fonts": fonts(),
        }),
        &["source"],
    )
}

fn run_render(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: RenderRequest = parse("render", input)?;
    Ok(done(render_with(&request, &settings)))
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
     rebuilt in memory from the undo states, or the reply says why it cannot be. A loop \
     sheet's display list runs to megabytes: it is for the viewer to draw, not to read."
        .to_owned()
}

fn display_list_input() -> Value {
    input(
        json!({
            "source": text("The record to draw: a sheet description, a DWG when the file ends .dwg, or a drawing set's folder."),
            "sheet": text("Which of the drawing set's sheets to draw, by name. Without it, the set's first."),
            "tolerance": number("How far, in drawing units, a curve's chords may stray from it. Without it, a 100,000th of the sheet's larger side."),
            "entry": {
                "description": "Draw the drawing set as it was just after this entry of its journal, by its number, rebuilt in memory from the undo states; nothing is written. Without it, as it is now.",
                "type": "integer",
                "minimum": 1,
            },
            "fonts": fonts(),
        }),
        &["source"],
    )
}

fn run_display_list(
    mut input: Map<String, Value>,
    base: &Settings,
    _: Interface,
) -> Result<JsonReply, Error> {
    let settings = take_settings(&mut input, base, false)?;
    let request: DisplayListRequest = parse("display-list", input)?;
    Ok(done(display_list_with(&request, &settings)))
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
         under `sheets`, and is not ok when any sheet failed."
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
        return Ok(sent(plot_set_with(&request, &settings)));
    }
    let request = PlotRequest {
        source,
        output,
        overwrite,
        plot_style,
    };
    Ok(done(plot_with(&request, &settings)))
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
     with nothing selected by coordinates; `query` says what, by its `kind`. find lists the \
     objects matching every part given, each with its ID and where it is written; attributes \
     gives each matching insert's attributes as JSON from tag to value; blocks lists the \
     blocks each sheet inserts; measure gives the distance between a point of two objects on \
     one sheet, each named by ID or name, by default an insert's or a text's position, a \
     line's from, a circle's centre; extents gives the box round what each sheet draws. \
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
        "description": "What to ask, by its `kind`: find, blocks, attributes, measure or extents.",
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
        ],
    })
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
    Ok(sent(change_with(&request, &settings)))
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
    Ok(sent(accept_with(&request, &settings, via)))
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
    Ok(done(check_with(&request, &settings)))
}

// What every command shares.

/// The drawing set a command's input reads, if it names one: the folder its
/// `set` names; or the set its `source`, or with none its `output`, is, or
/// is in, the nearest folder above it that holds a set's settings.
fn set_read(input: &Value) -> Option<PathBuf> {
    let named = ["set", "source", "output"]
        .iter()
        .find_map(|key| input.get(key))?
        .as_str()?;
    let path = Path::new(named);
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
/// names, then those of `base`; and, when the command plots, the plot
/// style folders its `plot_styles` names, then those of `base`. Both are
/// taken out of the input.
fn take_settings(
    input: &mut Map<String, Value>,
    base: &Settings,
    plots: bool,
) -> Result<Settings, Error> {
    let mut settings = base.clone();
    settings
        .font_folders
        .splice(0..0, folders(input.remove("fonts"), "fonts")?);
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

/// A command's reply from its core entry point's reply.
fn sent<T: Serialize>(reply: Reply<T>) -> JsonReply {
    Reply {
        ok: reply.ok,
        rolled_back: reply.rolled_back,
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
             variable lists on the machine the command runs on."
        ),
        "type": "array",
        "items": { "type": "string" },
    })
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
