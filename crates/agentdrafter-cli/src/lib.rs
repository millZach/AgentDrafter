//! `agentdrafter`: the command line over AgentDrafter's core.
//!
//! This is a thin layer (ADR-0007). It turns arguments into a command's
//! input, hands it to the core's registry of commands, and prints the
//! core's reply as one line of JSON on standard output, on success and on
//! failure. Drawing logic here is a bug.
//!
//! `agentdrafter serve` and `agentdrafter mcp` start the HTTP API and the
//! MCP server, which take their commands from the same registry
//! (`http.rs`, `mcp.rs`).
//!
//! The command is a library, [`main`], so that the binary
//! (`src/bin/agentdrafter.rs`) and the Python package's command, which runs
//! it inside the Python library's extension (`crates/agentdrafter-py`), are
//! the same code.
//!
//! Exit codes: 0 when the reply is `ok`, 1 when the core reports an error,
//! such as a sheet of a drawing set that failed, 2 when the arguments are
//! wrong.

mod assistant;
pub mod http;
mod mcp;
mod page;

pub use http::{start_local, LocalServer};

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use agentdrafter_core::{commands, find_command, Error, ErrorCode, Interface, Reply, Settings};
use agentdrafter_core::{
    AcceptRequest, CheckRequest, ConvertRequest, DisplayListRequest, DrawingFormat, DwgVersion,
    ImportBlocksRequest, JournalRequest, MeasureTextRequest, ReadRequest, RenderRequest,
    StaleRequest, UndoRequest, Unit, FONTS_VARIABLE, PLOT_STYLES_VARIABLE, SYSTEM_FONTS_VARIABLE,
};
use agentdrafter_core::{
    End, Matching, Measure, ObjectRef, Query, QueryRequest, Selection, SelectionBox, SnapKind,
    SnapRef,
};
use serde::Serialize;
use serde_json::{json, Map, Value};

const BUILD_USAGE: &str =
    "agentdrafter build <sheet-description | drawing-set-folder> [--output <dwg-or-dxf>] [--format <dwg | dxf | dxf_binary>] [--dwg-version <version>] [--overwrite] [--clean | --no-clean] [--fonts <folder>]... [--no-system-fonts]";
const SCHEMA_USAGE: &str =
    "agentdrafter schema [sheet-description | block-library | template | set | rows | change-set]";
const OPERATIONS_USAGE: &str = "agentdrafter operations";
const CHANGE_USAGE: &str = "agentdrafter change <drawing-set-folder | original.dwg> <change-set.json | -> [--output <revision.dwg>] [--allow-loss] [--expected-original-sha256 <sha256>] [--dry-run] [--fonts <folder>]... [--no-system-fonts] [--plot-style <name>] [--plot-styles <folder>]... [--no-pdf]";
const JOURNAL_USAGE: &str = "agentdrafter journal <drawing-set-folder | original.dwg>";
const UNDO_USAGE: &str =
    "agentdrafter undo <drawing-set-folder | original.dwg> <change-set> [--dry-run]";
const ACCEPT_USAGE: &str = "agentdrafter accept <drawing-set-folder | original.dwg> [--despite-failed-checks] [--newest-seen <entry>] [--dry-run] [--fonts <folder>]... [--no-system-fonts]";
const PROPOSE_USAGE: &str = "agentdrafter propose <drawing-set-folder> <change-set.json | -> [--by <who>] [--fonts <folder>]... [--no-system-fonts]";
const PROPOSALS_USAGE: &str = "agentdrafter proposals <drawing-set-folder>";
const GUIDE_USAGE: &str = "agentdrafter guide <drawing-set-folder> <guide.json | -> [--by <who>] [--fonts <folder>]... [--no-system-fonts]";
const GUIDES_USAGE: &str =
    "agentdrafter guides <drawing-set-folder> [--fonts <folder>]... [--no-system-fonts]";
const KEEP_USAGE: &str = "agentdrafter keep <drawing-set-folder> <proposal> [--by <who>] [--fonts <folder>]... [--no-system-fonts]";
const DISCARD_USAGE: &str = "agentdrafter discard <drawing-set-folder> <proposal> [--by <who>]";
const LOOK_AT_USAGE: &str =
    "agentdrafter look-at <drawing-set-folder> <sheet> [--layout <layout>] [--selected <id>]...";
const LOOKING_AT_USAGE: &str = "agentdrafter looking-at <drawing-set-folder>";
const MEASURE_USAGE: &str = "agentdrafter measure-text <contents> --font <file> --height <height> [--width-factor <factor>] [--fonts <folder>]... [--no-system-fonts]";
const RENDER_USAGE: &str = "agentdrafter render <sheet-description-or-dwg> [--output <png>] [--overwrite] [--dpi <dpi>] [--scale <scale> | --width <pixels> --height <pixels>] [--margin <mm>] [--plot-style <ctb-or-stb> | --monochrome] [--plot-styles <folder>]... [--fonts <folder>]... [--no-system-fonts]";
const DISPLAY_LIST_USAGE: &str = "agentdrafter display-list <sheet-description | dwg | drawing-set-folder> [--sheet <sheet>] [--layout <layout>] [--tolerance <length>] [--entry <journal-entry>] [--snaps] [--origins] [--values] [--change-set <change-set.json | -> [--all-or-none]] [--read-past-damage] [--fonts <folder>]... [--no-system-fonts]";
const PLOT_USAGE: &str = "agentdrafter plot <sheet-description | dwg | drawing-set-folder> [--output <pdf>] [--overwrite] [--clean | --no-clean] [--plot-style <ctb-or-stb>] [--plot-styles <folder>]... [--fonts <folder>]... [--no-system-fonts]";
const STALE_USAGE: &str =
    "agentdrafter stale <png-or-pdf> [--source <sheet-description-dwg-or-template>]";
const CHECK_USAGE: &str = "agentdrafter check <drawing-set-folder | sheet-description | dwg> [--sheet <sheet>]... [--fonts <folder>]... [--no-system-fonts]";
const IMPORT_BLOCKS_USAGE: &str = "agentdrafter import-blocks <dwg | dxf> --library <block-library> [--block <name>]... [--replace] [--units <unit>] [--dry-run]";
const READ_USAGE: &str = "agentdrafter read <dwg> [--mirror <path> | --no-mirror]";
const CONVERT_USAGE: &str = "agentdrafter convert <dwg | dxf | folder | drawing-set-folder> --version <version> [--output <path-or-folder>] [--format <dwg | dxf | dxf_binary>] [--overwrite] [--allow-loss] [--dry-run]";
const QUERY_USAGE: &str = "agentdrafter query <find | blocks | attributes | measure | extents | layers | snap | geometry | layout | groups | annotation-scales> <drawing-set-folder | sheet-description | dwg> [--sheet <sheet>] [find, attributes and geometry: --id <id> --name <name> --block <block> --tag <tag> --value <value> --text <text> --group <group> --space <space> --contains --window <x1,y1,x2,y2> --crossing <x1,y1,x2,y2> --touching <id> | --touching-name <name>] [measure: --from <id> | --from-name <name>, --to <id> | --to-name <name>, --from-point <point> --to-point <point>] [snap: --id <id> | --name <name>, --snap <snap> --which <which>, --with <id> | --with-name <name>, --from <id> | --from-name <name> --from-snap <snap> --from-which <which> --from-with <id> | --from-with-name <name>] [--fonts <folder>...] [--no-system-fonts]";

/// Runs the `agentdrafter` command on `args`, the arguments after its own
/// name, as the binary does: it prints the reply on standard output, or
/// serves until it is stopped, and gives the exit code.
pub fn main(args: Vec<OsString>) -> ExitCode {
    let mut args = args.into_iter().peekable();
    // The servers run until they are stopped, and print as their protocols
    // say; every other command prints one line.
    match args.peek().and_then(|a| a.to_str()) {
        Some("serve") => return http::serve(args.skip(1).collect::<Vec<_>>().into_iter()),
        Some("mcp") => return mcp::serve(args.skip(1).collect::<Vec<_>>().into_iter()),
        _ => {}
    }
    let (line, code) = run(args.collect());
    println!("{line}");
    code
}

/// The arguments after a command's name.
type Args = std::vec::IntoIter<OsString>;

/// Turns a command's arguments into its input, as the HTTP API and the MCP
/// server take it.
type Parse = fn(Args) -> Result<Value, Error>;

/// The command line's form of a registry command: its usage and how its
/// arguments become its input. Every command of the registry has one, which
/// a test holds it to.
fn cli_form(command: &str) -> Option<(&'static str, Parse)> {
    Some(match command {
        "build" => (BUILD_USAGE, parse_build),
        "schema" => (SCHEMA_USAGE, parse_schema),
        "measure-text" => (MEASURE_USAGE, parse_measure),
        "render" => (RENDER_USAGE, parse_render),
        "display-list" => (DISPLAY_LIST_USAGE, parse_display_list),
        "plot" => (PLOT_USAGE, parse_plot),
        "stale" => (STALE_USAGE, parse_stale),
        "query" => (QUERY_USAGE, parse_query),
        "operations" => (OPERATIONS_USAGE, parse_operations),
        "change" => (CHANGE_USAGE, parse_change),
        "journal" => (JOURNAL_USAGE, parse_journal),
        "undo" => (UNDO_USAGE, parse_undo),
        "accept" => (ACCEPT_USAGE, parse_accept),
        "propose" => (PROPOSE_USAGE, parse_propose),
        "proposals" => (PROPOSALS_USAGE, parse_proposals),
        "keep" => (KEEP_USAGE, parse_keep),
        "discard" => (DISCARD_USAGE, parse_discard),
        "guide" => (GUIDE_USAGE, parse_guide),
        "guides" => (GUIDES_USAGE, parse_guides),
        "look-at" => (LOOK_AT_USAGE, parse_look_at),
        "looking-at" => (LOOKING_AT_USAGE, parse_looking_at),
        "check" => (CHECK_USAGE, parse_check),
        "import-blocks" => (IMPORT_BLOCKS_USAGE, parse_import_blocks),
        "read" => (READ_USAGE, parse_read),
        "convert" => (CONVERT_USAGE, parse_convert),
        _ => return None,
    })
}

/// Every command's usage, the servers' last.
fn usages() -> String {
    commands()
        .iter()
        .filter_map(|command| cli_form(command.name()).map(|(usage, _)| usage))
        .chain([http::SERVE_USAGE, mcp::MCP_USAGE])
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Runs one command and returns the JSON line to print and the exit code.
fn run(args: Vec<OsString>) -> (String, ExitCode) {
    let mut args = args.into_iter();
    let Some(name) = args.next() else {
        return reply::<()>(Err(Error::usage(format!(
            "no command given; usage: {}",
            usages()
        ))));
    };
    match name.to_str() {
        Some("help" | "--help" | "-h") => return reply(Ok(help())),
        Some("--version" | "-V") => {
            return reply(Ok(json!({ "version": env!("CARGO_PKG_VERSION") })))
        }
        _ => {}
    }
    let Some((command, (_, parse))) = name
        .to_str()
        .and_then(find_command)
        .and_then(|command| Some((command, cli_form(command.name())?)))
    else {
        return reply::<()>(Err(Error::usage(format!(
            "unknown command `{}`; usage: {}",
            name.to_string_lossy(),
            usages()
        ))));
    };
    match parse(args) {
        Ok(input) => send(command.run(input, &Settings::from_env(), Interface::Cli)),
        Err(error) => reply::<()>(Err(error)),
    }
}

/// The font options a command takes: the folders `--fonts` names, and
/// whether `--no-system-fonts` leaves the system's font folders out.
#[derive(Default)]
struct FontOptions {
    folders: Vec<PathBuf>,
    no_system: bool,
}

impl FontOptions {
    fn push(&mut self, folder: PathBuf) {
        self.folders.push(folder);
    }
}

/// The input a core request gives as plain data, with the font folders
/// `--fonts` names and `--no-system-fonts`. A path that is not valid text
/// cannot be sent as JSON, through any interface.
fn input_of(request: impl Serialize, fonts: FontOptions) -> Result<Value, Error> {
    let not_text = |e: serde_json::Error| {
        Error::usage(format!(
            "every interface takes paths and values as text (UTF-8), and one given is not: {e}"
        ))
    };
    let mut input = serde_json::to_value(request).map_err(not_text)?;
    if !fonts.folders.is_empty() {
        input["fonts"] = serde_json::to_value(fonts.folders).map_err(not_text)?;
    }
    if fonts.no_system {
        input["no_system_fonts"] = Value::Bool(true);
    }
    Ok(input)
}

/// A number an option gave, which is finite. JSON has no way to write NaN
/// or an infinity, so one would reach the core as no number at all, and
/// pass the checks a number is held to; it is refused here instead.
fn float(flag: &str, number: f64) -> Result<f64, Error> {
    if number.is_finite() {
        return Ok(number);
    }
    Err(Error::new(
        ErrorCode::BadNumber,
        format!("{flag} takes a finite number, not `{number}`"),
    ))
}

fn parse_schema(mut args: Args) -> Result<Value, Error> {
    let kind = args.next();
    if let Some(extra) = args.next() {
        return Err(Error::usage(format!(
            "schema takes at most one argument, found `{}` as well; usage: {SCHEMA_USAGE}",
            extra.to_string_lossy()
        )));
    }
    match kind {
        None => Ok(json!({})),
        Some(kind) => match kind.into_string() {
            Ok(kind) => Ok(json!({ "kind": kind })),
            Err(kind) => Err(Error::usage(format!(
                "schema prints the schema of a sheet description, a block library, a template, a drawing set's settings, its rows or a change set, not `{}`; usage: {SCHEMA_USAGE}",
                kind.to_string_lossy()
            ))),
        },
    }
}

fn parse_operations(mut args: Args) -> Result<Value, Error> {
    match args.next() {
        None => Ok(json!({})),
        Some(extra) => Err(Error::usage(format!(
            "operations takes no argument, found `{}`; usage: {OPERATIONS_USAGE}",
            extra.to_string_lossy()
        ))),
    }
}

fn parse_render(mut args: Args) -> Result<Value, Error> {
    let mut source: Option<PathBuf> = None;
    let mut request = RenderRequest::new(PathBuf::new());
    let mut fonts = FontOptions::default();
    let mut plot_styles: Vec<PathBuf> = Vec::new();
    let value = |flag: &str, value: Option<OsString>| -> Result<OsString, Error> {
        value.ok_or_else(|| Error::usage(format!("{flag} needs a value; usage: {RENDER_USAGE}")))
    };
    fn number<T: std::str::FromStr>(flag: &str, text: OsString) -> Result<T, Error> {
        let text = text.to_string_lossy();
        text.parse().map_err(|_| {
            Error::usage(format!(
                "{flag} takes a number, found `{text}`; usage: {RENDER_USAGE}"
            ))
        })
    }
    let finite = |flag: &str, text: OsString| float(flag, number(flag, text)?);
    let once = |flag: &str| Error::usage(format!("{flag} is given twice"));
    let (mut dpi, mut margin) = (None, None);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--output" | "-o") => {
                let path = value("--output", args.next())?;
                if request.output.replace(path.into()).is_some() {
                    return Err(once("--output"));
                }
            }
            Some("--overwrite") => request.overwrite = true,
            Some("--monochrome") => request.monochrome = true,
            Some("--plot-style") => {
                let name = value("--plot-style", args.next())?
                    .into_string()
                    .map_err(|_| Error::usage("the plot style's name is not valid text"))?;
                if request.plot_style.replace(name).is_some() {
                    return Err(once("--plot-style"));
                }
            }
            Some("--plot-styles") => plot_styles.push(value("--plot-styles", args.next())?.into()),
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => fonts.push(value("--fonts", args.next())?.into()),
            Some("--dpi") => {
                if dpi
                    .replace(finite("--dpi", value("--dpi", args.next())?)?)
                    .is_some()
                {
                    return Err(once("--dpi"));
                }
            }
            Some("--margin") => {
                let mm = finite("--margin", value("--margin", args.next())?)?;
                if margin.replace(mm).is_some() {
                    return Err(once("--margin"));
                }
            }
            Some("--scale") => {
                let scale = finite("--scale", value("--scale", args.next())?)?;
                if request.scale.replace(scale).is_some() {
                    return Err(once("--scale"));
                }
            }
            Some("--width") => {
                let pixels = number("--width", value("--width", args.next())?)?;
                if request.width.replace(pixels).is_some() {
                    return Err(once("--width"));
                }
            }
            Some("--height") => {
                let pixels = number("--height", value("--height", args.next())?)?;
                if request.height.replace(pixels).is_some() {
                    return Err(once("--height"));
                }
            }
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(Error::usage(format!(
                    "unknown option `{flag}`; usage: {RENDER_USAGE}"
                )))
            }
            _ => {
                if source.replace(arg.into()).is_some() {
                    return Err(Error::usage(format!(
                        "render takes one sheet description or DWG; usage: {RENDER_USAGE}"
                    )));
                }
            }
        }
    }
    request.source = source.ok_or_else(|| {
        Error::usage(format!(
            "render needs a sheet description or DWG; usage: {RENDER_USAGE}"
        ))
    })?;
    if let Some(dpi) = dpi {
        request.dpi = dpi;
    }
    if let Some(margin) = margin {
        request.margin = margin;
    }
    input_of(
        RenderArgs {
            request,
            plot_styles,
        },
        fonts,
    )
}

/// What the command line's `render` gives the core: the request, and the
/// plot style folders `--plot-styles` names, which go to the settings.
#[derive(Serialize)]
struct RenderArgs {
    #[serde(flatten)]
    request: RenderRequest,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    plot_styles: Vec<PathBuf>,
}

fn parse_display_list(mut args: Args) -> Result<Value, Error> {
    let mut request = DisplayListRequest::new(PathBuf::new());
    let mut source: Option<PathBuf> = None;
    let mut fonts = FontOptions::default();
    let value = |flag: &str, value: Option<OsString>| -> Result<OsString, Error> {
        value.ok_or_else(|| {
            Error::usage(format!("{flag} needs a value; usage: {DISPLAY_LIST_USAGE}"))
        })
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--sheet") => {
                let sheet = value("--sheet", args.next())?
                    .into_string()
                    .map_err(|_| Error::usage("the sheet's name is not valid text"))?;
                if request.sheet.replace(sheet).is_some() {
                    return Err(Error::usage("--sheet is given twice"));
                }
            }
            Some("--layout") => {
                let layout = value("--layout", args.next())?
                    .into_string()
                    .map_err(|_| Error::usage("the layout's name is not valid text"))?;
                if request.layout.replace(layout).is_some() {
                    return Err(Error::usage("--layout is given twice"));
                }
            }
            Some("--tolerance") => {
                let text = value("--tolerance", args.next())?;
                let text = text.to_string_lossy();
                let length: f64 = text.parse().map_err(|_| {
                    Error::usage(format!(
                        "--tolerance takes a number, found `{text}`; usage: {DISPLAY_LIST_USAGE}"
                    ))
                })?;
                if request
                    .tolerance
                    .replace(float("--tolerance", length)?)
                    .is_some()
                {
                    return Err(Error::usage("--tolerance is given twice"));
                }
            }
            Some("--entry") => {
                let text = value("--entry", args.next())?;
                let entry = entry_number(&text).ok_or_else(|| {
                    Error::usage(format!(
                        "--entry takes a journal entry by its number, such as 3, not `{}`; usage: \
                         {DISPLAY_LIST_USAGE}",
                        text.to_string_lossy()
                    ))
                })?;
                if request.entry.replace(entry).is_some() {
                    return Err(Error::usage("--entry is given twice"));
                }
            }
            Some("--snaps") => {
                if request.snaps {
                    return Err(Error::usage("--snaps is given twice"));
                }
                request.snaps = true;
            }
            Some("--origins") => {
                if request.origins {
                    return Err(Error::usage("--origins is given twice"));
                }
                request.origins = true;
            }
            Some("--values") => {
                if request.values {
                    return Err(Error::usage("--values is given twice"));
                }
                request.values = true;
            }
            Some("--change-set") => {
                let file = value("--change-set", args.next())?;
                if request
                    .change_set
                    .replace(read_change_set(&file)?)
                    .is_some()
                {
                    return Err(Error::usage("--change-set is given twice"));
                }
            }
            Some("--all-or-none") => {
                if request.all_or_none {
                    return Err(Error::usage("--all-or-none is given twice"));
                }
                request.all_or_none = true;
            }
            Some("--read-past-damage") => {
                if request.read_past_damage {
                    return Err(Error::usage("--read-past-damage is given twice"));
                }
                request.read_past_damage = true;
            }
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => fonts.push(value("--fonts", args.next())?.into()),
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(Error::usage(format!(
                    "unknown option `{flag}`; usage: {DISPLAY_LIST_USAGE}"
                )))
            }
            _ => {
                if source.replace(arg.into()).is_some() {
                    return Err(Error::usage(format!(
                        "display-list takes one sheet description, DWG or drawing set's folder; \
                         usage: {DISPLAY_LIST_USAGE}"
                    )));
                }
            }
        }
    }
    request.source = source.ok_or_else(|| {
        Error::usage(format!(
            "display-list needs a sheet description, a DWG or a drawing set's folder; usage: \
             {DISPLAY_LIST_USAGE}"
        ))
    })?;
    input_of(request, fonts)
}

/// The input of `build` and `plot`: one sheet description, DWG, DXF or drawing
/// set's folder, which the core tells apart.
#[derive(Serialize)]
struct SourceArgs {
    source: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<DrawingFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dwg_version: Option<DwgVersion>,
    overwrite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    clean: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plot_style: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    plot_styles: Vec<PathBuf>,
}

/// Takes `--clean` or `--no-clean` into `clean`: a drawing set's run moves
/// its leftover outputs into `superseded/`, or only lists them, whatever
/// its settings say. Each is given once, and not both.
fn set_clean(clean: &mut Option<bool>, flag: &str) -> Result<(), Error> {
    if clean.replace(flag == "--clean").is_some() {
        return Err(Error::usage(
            "--clean or --no-clean is given more than once; give one, or neither to do as the \
             set's settings say",
        ));
    }
    Ok(())
}

fn parse_plot(mut args: Args) -> Result<Value, Error> {
    let mut source: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut overwrite = false;
    let mut clean: Option<bool> = None;
    let mut plot_style: Option<String> = None;
    let mut fonts = FontOptions::default();
    let mut plot_styles = Vec::new();
    let value = |flag: &str, value: Option<OsString>| -> Result<OsString, Error> {
        value.ok_or_else(|| Error::usage(format!("{flag} needs a value; usage: {PLOT_USAGE}")))
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--output" | "-o") => {
                let path = value("--output", args.next())?;
                if output.replace(path.into()).is_some() {
                    return Err(Error::usage("--output is given twice"));
                }
            }
            Some("--overwrite") => overwrite = true,
            Some(flag @ ("--clean" | "--no-clean")) => set_clean(&mut clean, flag)?,
            Some("--plot-style") => {
                let name = value("--plot-style", args.next())?
                    .into_string()
                    .map_err(|_| Error::usage("the plot style's name is not valid text"))?;
                if plot_style.replace(name).is_some() {
                    return Err(Error::usage("--plot-style is given twice"));
                }
            }
            Some("--plot-styles") => plot_styles.push(value("--plot-styles", args.next())?.into()),
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => fonts.push(value("--fonts", args.next())?.into()),
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(Error::usage(format!(
                    "unknown option `{flag}`; usage: {PLOT_USAGE}"
                )))
            }
            _ => {
                if source.replace(arg.into()).is_some() {
                    return Err(Error::usage(format!(
                        "plot takes one sheet description, DWG or drawing set; usage: {PLOT_USAGE}"
                    )));
                }
            }
        }
    }
    let source = source.ok_or_else(|| {
        Error::usage(format!(
            "plot needs a sheet description, a DWG or a drawing set's folder; usage: {PLOT_USAGE}"
        ))
    })?;
    let request = SourceArgs {
        source,
        output,
        format: None,
        dwg_version: None,
        overwrite,
        clean,
        plot_style,
        plot_styles,
    };
    input_of(request, fonts)
}

fn parse_stale(mut args: Args) -> Result<Value, Error> {
    let mut output: Option<PathBuf> = None;
    let mut source: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--source") => {
                let path = args.next().ok_or_else(|| {
                    Error::usage(format!("--source needs a path; usage: {STALE_USAGE}"))
                })?;
                if source.replace(path.into()).is_some() {
                    return Err(Error::usage("--source is given twice"));
                }
            }
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(Error::usage(format!(
                    "unknown option `{flag}`; usage: {STALE_USAGE}"
                )))
            }
            _ => {
                if output.replace(arg.into()).is_some() {
                    return Err(Error::usage(format!(
                        "stale takes one output; usage: {STALE_USAGE}"
                    )));
                }
            }
        }
    }
    let output = output.ok_or_else(|| {
        Error::usage(format!(
            "stale needs the output to check; usage: {STALE_USAGE}"
        ))
    })?;
    input_of(StaleRequest { output, source }, FontOptions::default())
}

/// Reads `change`'s arguments: the drawing set's folder or an original `.dwg`,
/// the change set's JSON file or `-` for standard input, and the options.
fn parse_change(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {CHANGE_USAGE}"));
    let value = |flag: &str, val: Option<OsString>| -> Result<OsString, Error> {
        match val {
            Some(v)
                if v.to_str()
                    .is_some_and(|text| text.starts_with('-') && text.len() > 1) =>
            {
                Err(usage(format!(
                    "{flag} needs a value, not option `{}`",
                    v.to_string_lossy()
                )))
            }
            Some(v) => Ok(v),
            None => Err(usage(format!("{flag} needs a value"))),
        }
    };
    let mut given: Vec<OsString> = Vec::new();
    let mut dry_run = false;
    let mut output: Option<PathBuf> = None;
    let mut allow_loss = false;
    let mut expected_original_sha256: Option<String> = None;
    let mut plot_style: Option<String> = None;
    let mut plot_styles: Vec<PathBuf> = Vec::new();
    let mut no_pdf = false;
    let mut fonts = FontOptions::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--dry-run") => dry_run = true,
            Some("--allow-loss") => allow_loss = true,
            Some("--no-pdf") => no_pdf = true,
            Some("--expected-original-sha256") => {
                let hash = value("--expected-original-sha256", args.next())?
                    .into_string()
                    .map_err(|_| {
                        usage("--expected-original-sha256 must be valid UTF-8 text".to_owned())
                    })?;
                if expected_original_sha256.replace(hash).is_some() {
                    return Err(usage(
                        "--expected-original-sha256 is given twice".to_owned(),
                    ));
                }
            }
            Some("--output" | "-o") => {
                let path = value("--output", args.next())?;
                if output.replace(PathBuf::from(path)).is_some() {
                    return Err(usage("--output is given twice".to_owned()));
                }
            }
            Some("--plot-style") => {
                let name = value("--plot-style", args.next())?
                    .into_string()
                    .map_err(|_| usage("the plot style's name is not valid text".to_owned()))?;
                if plot_style.replace(name).is_some() {
                    return Err(usage("--plot-style is given twice".to_owned()));
                }
            }
            Some("--plot-styles") => {
                let folder = value("--plot-styles", args.next())?;
                plot_styles.push(PathBuf::from(folder));
            }
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => {
                let folder = value("--fonts", args.next())?;
                fonts.push(PathBuf::from(folder));
            }
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("change takes no option `{flag}`")))
            }
            _ => given.push(arg),
        }
    }
    let [set, change_set] = <[OsString; 2]>::try_from(given).map_err(|given| {
        usage(format!(
            "change takes a drawing set's folder (or original `.dwg`) and a change set, found {} \
             arguments",
            given.len()
        ))
    })?;
    let request = agentdrafter_core::ReviseRequest {
        set: PathBuf::from(set),
        change_set: read_change_set(&change_set)?,
        dry_run,
        output,
        allow_loss,
        expected_original_sha256,
        plot_style,
        no_pdf,
    };
    let mut input = input_of(request, fonts)?;
    if !plot_styles.is_empty() {
        input["plot_styles"] = serde_json::to_value(plot_styles).map_err(|e| {
            Error::new(
                ErrorCode::Usage,
                format!("could not encode plot styles: {e}"),
            )
        })?;
    }
    Ok(input)
}

/// Reads a change set from the file `given` names, or from standard input
/// for `-`, as `change` and `display-list --change-set` take it.
fn read_change_set(given: &OsString) -> Result<Value, Error> {
    read_json(given, "change set")
}

/// Reads JSON, `what`, from the file `given` names, or from standard input
/// for `-`.
fn read_json(given: &OsString, what: &str) -> Result<Value, Error> {
    let text = if given == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the {what} from standard input: {e}"),
            )
        })?;
        text
    } else {
        let path = PathBuf::from(given);
        std::fs::read_to_string(&path).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the {what} `{}`: {e}", path.display()),
            )
        })?
    };
    serde_json::from_str(&text)
        .map_err(|e| Error::new(ErrorCode::WrongType, format!("the {what} is not JSON: {e}")))
}

/// Reads `journal`'s argument: the set's folder.
fn parse_journal(args: Args) -> Result<Value, Error> {
    let given: Vec<OsString> = args.collect();
    if let Some(flag) = given
        .iter()
        .filter_map(|a| a.to_str())
        .find(|a| a.starts_with('-') && a.len() > 1)
    {
        return Err(Error::usage(format!(
            "journal takes no option `{flag}`; usage: {JOURNAL_USAGE}"
        )));
    }
    let [set] = <[OsString; 1]>::try_from(given).map_err(|given| {
        Error::usage(format!(
            "journal takes a drawing set's folder or an original `.dwg`, found {} arguments; \
             usage: {JOURNAL_USAGE}",
            given.len()
        ))
    })?;
    let request = JournalRequest {
        set: PathBuf::from(set),
    };
    input_of(request, FontOptions::default())
}

/// Reads `undo`'s arguments: the set's folder, the change set's entry in
/// the journal, and the options.
fn parse_undo(args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {UNDO_USAGE}"));
    let mut given: Vec<OsString> = Vec::new();
    let mut dry_run = false;
    for arg in args {
        match arg.to_str() {
            Some("--dry-run") => dry_run = true,
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("undo takes no option `{flag}`")))
            }
            _ => given.push(arg),
        }
    }
    let [set, change_set] = <[OsString; 2]>::try_from(given).map_err(|given| {
        usage(format!(
            "undo takes a drawing set's folder (or original `.dwg`) and a change set, found {} \
             arguments",
            given.len()
        ))
    })?;
    let change_set = change_set
        .to_str()
        .and_then(|n| n.parse::<u64>().ok())
        .ok_or_else(|| {
            usage(format!(
                "undo takes the change set by its entry's number in the journal, such as 3, \
                 not `{}`",
                change_set.to_string_lossy()
            ))
        })?;
    let request = UndoRequest {
        set: PathBuf::from(set),
        change_set,
        dry_run,
    };
    input_of(request, FontOptions::default())
}

/// A journal entry's number, as an argument gives it.
fn entry_number(text: &OsString) -> Option<u64> {
    text.to_str().and_then(|n| n.parse::<u64>().ok())
}

fn parse_accept(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {ACCEPT_USAGE}"));
    let mut set: Option<OsString> = None;
    let mut request = AcceptRequest {
        set: PathBuf::new(),
        despite_failed_checks: false,
        newest_seen: None,
        dry_run: false,
    };
    let mut fonts = FontOptions::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--dry-run") => request.dry_run = true,
            Some("--despite-failed-checks") => request.despite_failed_checks = true,
            Some("--newest-seen") => {
                let text = args
                    .next()
                    .ok_or_else(|| usage("--newest-seen needs a journal entry's number".into()))?;
                let entry = entry_number(&text).ok_or_else(|| {
                    usage(format!(
                        "--newest-seen takes the journal's newest entry the caller has seen, by \
                         its number, such as 7, not `{}`",
                        text.to_string_lossy()
                    ))
                })?;
                if request.newest_seen.replace(entry).is_some() {
                    return Err(usage("--newest-seen is given twice".into()));
                }
            }
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => fonts.push(PathBuf::from(
                args.next()
                    .ok_or_else(|| usage("--fonts needs a folder".into()))?,
            )),
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("accept takes no option `{flag}`")))
            }
            _ => {
                if set.replace(arg).is_some() {
                    return Err(usage(
                        "accept takes one drawing set's folder or original `.dwg`".into(),
                    ));
                }
            }
        }
    }
    request.set = PathBuf::from(set.ok_or_else(|| {
        usage("accept needs a drawing set's folder or an original `.dwg`".into())
    })?);
    input_of(request, fonts)
}

/// What a proposal's command takes after the set's folder: a change set,
/// or a proposal's number.
#[derive(Clone, Copy, PartialEq)]
enum Second {
    ChangeSet,
    Proposal,
    Guide,
}

/// Reads the arguments of `propose`, `keep` and `discard`: the set's
/// folder, then the change set or the proposal's number, `--by`, and,
/// where `fonts` says the command measures text, the font options.
fn parse_proposing(
    mut args: Args,
    command: &str,
    usage_text: &str,
    second: Second,
    fonts_taken: bool,
) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {usage_text}"));
    let mut given: Vec<OsString> = Vec::new();
    let mut by: Option<String> = None;
    let mut fonts = FontOptions::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--by") => {
                let who = args
                    .next()
                    .ok_or_else(|| usage("--by needs who it is, such as Assistant".into()))?;
                let who = who
                    .into_string()
                    .map_err(|_| usage("--by takes text (UTF-8)".into()))?;
                if by.replace(who).is_some() {
                    return Err(usage("--by is given twice".into()));
                }
            }
            Some("--no-system-fonts") if fonts_taken => fonts.no_system = true,
            Some("--fonts") if fonts_taken => fonts.push(PathBuf::from(
                args.next()
                    .ok_or_else(|| usage("--fonts needs a folder".into()))?,
            )),
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("{command} takes no option `{flag}`")))
            }
            _ => given.push(arg),
        }
    }
    let what = match second {
        Second::ChangeSet => "a change set",
        Second::Proposal => "a proposal's number",
        Second::Guide => "a guide",
    };
    let [set, then] = <[OsString; 2]>::try_from(given).map_err(|given| {
        usage(format!(
            "{command} takes a drawing set's folder and {what}, found {} arguments",
            given.len()
        ))
    })?;
    let set = set
        .into_string()
        .map_err(|_| usage("every interface takes paths as text (UTF-8)".into()))?;
    let mut input = json!({ "set": set });
    match second {
        Second::ChangeSet => input["change_set"] = read_change_set(&then)?,
        Second::Guide => input["guide"] = read_json(&then, "guide")?,
        Second::Proposal => {
            let number = entry_number(&then).ok_or_else(|| {
                usage(format!(
                    "{command} takes the proposal by its number, such as 2, not `{}`",
                    then.to_string_lossy()
                ))
            })?;
            input["proposal"] = json!(number);
        }
    }
    if let Some(by) = by {
        input["by"] = Value::String(by);
    }
    input_of(input, fonts)
}

fn parse_propose(args: Args) -> Result<Value, Error> {
    parse_proposing(args, "propose", PROPOSE_USAGE, Second::ChangeSet, true)
}

fn parse_keep(args: Args) -> Result<Value, Error> {
    parse_proposing(args, "keep", KEEP_USAGE, Second::Proposal, true)
}

fn parse_discard(args: Args) -> Result<Value, Error> {
    parse_proposing(args, "discard", DISCARD_USAGE, Second::Proposal, false)
}

fn parse_guide(args: Args) -> Result<Value, Error> {
    parse_proposing(args, "guide", GUIDE_USAGE, Second::Guide, true)
}

/// Reads `guides`' arguments: the set's folder, and the font options.
fn parse_guides(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {GUIDES_USAGE}"));
    let mut given: Vec<OsString> = Vec::new();
    let mut fonts = FontOptions::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => fonts.push(PathBuf::from(
                args.next()
                    .ok_or_else(|| usage("--fonts needs a folder".into()))?,
            )),
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("guides takes no option `{flag}`")))
            }
            _ => given.push(arg),
        }
    }
    let [set] = <[OsString; 1]>::try_from(given).map_err(|given| {
        usage(format!(
            "guides takes a drawing set's folder, found {} arguments",
            given.len()
        ))
    })?;
    let request = agentdrafter_core::GuidesRequest {
        set: PathBuf::from(set),
    };
    input_of(request, fonts)
}

/// Reads `proposals`' argument: the set's folder.
fn parse_proposals(args: Args) -> Result<Value, Error> {
    let given: Vec<OsString> = args.collect();
    if let Some(flag) = given
        .iter()
        .filter_map(|a| a.to_str())
        .find(|a| a.starts_with('-') && a.len() > 1)
    {
        return Err(Error::usage(format!(
            "proposals takes no option `{flag}`; usage: {PROPOSALS_USAGE}"
        )));
    }
    let [set] = <[OsString; 1]>::try_from(given).map_err(|given| {
        Error::usage(format!(
            "proposals takes a drawing set's folder, found {} arguments; usage: \
             {PROPOSALS_USAGE}",
            given.len()
        ))
    })?;
    let request = agentdrafter_core::ProposalsRequest {
        set: PathBuf::from(set),
    };
    input_of(request, FontOptions::default())
}

/// Reads `look-at`'s arguments: the set's folder, the sheet, `--layout`
/// and each `--selected` object's ID.
fn parse_look_at(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {LOOK_AT_USAGE}"));
    let text = |arg: OsString, what: &str| {
        arg.into_string()
            .map_err(|_| usage(format!("{what} is text (UTF-8)")))
    };
    let mut given: Vec<String> = Vec::new();
    let mut layout: Option<String> = None;
    let mut selected: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--layout") => {
                let named = args
                    .next()
                    .ok_or_else(|| usage("--layout needs the layout's name".into()))?;
                if layout.replace(text(named, "--layout")?).is_some() {
                    return Err(usage("--layout is given twice".into()));
                }
            }
            Some("--selected") => {
                let id = args
                    .next()
                    .ok_or_else(|| usage("--selected needs an object's ID".into()))?;
                selected.push(text(id, "--selected")?);
            }
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("look-at takes no option `{flag}`")))
            }
            _ => given.push(text(arg, "every argument")?),
        }
    }
    let [set, sheet] = <[String; 2]>::try_from(given).map_err(|given| {
        usage(format!(
            "look-at takes a drawing set's folder and a sheet, found {} arguments",
            given.len()
        ))
    })?;
    let mut input = json!({ "set": set, "sheet": sheet, "selected": selected });
    if let Some(layout) = layout {
        input["layout"] = Value::String(layout);
    }
    Ok(input)
}

/// Reads `looking-at`'s argument: the set's folder.
fn parse_looking_at(args: Args) -> Result<Value, Error> {
    let given: Vec<OsString> = args.collect();
    if let Some(flag) = given
        .iter()
        .filter_map(|a| a.to_str())
        .find(|a| a.starts_with('-') && a.len() > 1)
    {
        return Err(Error::usage(format!(
            "looking-at takes no option `{flag}`; usage: {LOOKING_AT_USAGE}"
        )));
    }
    let [set] = <[OsString; 1]>::try_from(given).map_err(|given| {
        Error::usage(format!(
            "looking-at takes a drawing set's folder, found {} arguments; usage:              {LOOKING_AT_USAGE}",
            given.len()
        ))
    })?;
    let set = set
        .into_string()
        .map_err(|_| Error::usage("every interface takes paths as text (UTF-8)".to_owned()))?;
    Ok(json!({ "set": set }))
}

fn parse_query(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {QUERY_USAGE}"));
    let kind = match args.next().as_ref().map(|a| a.to_string_lossy()) {
        Some(kind) => kind.into_owned(),
        None => return Err(usage("query needs what to ask".to_owned())),
    };
    let selecting = matches!(kind.as_str(), "find" | "attributes" | "geometry");
    if !selecting
        && !matches!(
            kind.as_str(),
            "blocks"
                | "measure"
                | "extents"
                | "layers"
                | "snap"
                | "layout"
                | "groups"
                | "annotation-scales"
                | "annotation_scales"
        )
    {
        return Err(usage(format!(
            "query asks find, blocks, attributes, measure, extents, layers, snap, geometry, layout, \
             groups or annotation-scales, not `{kind}`"
        )));
    }
    let mut source: Option<PathBuf> = None;
    let mut sheet: Option<String> = None;
    let mut selection = Selection::default();
    let (mut from, mut to) = (End::default(), End::default());
    let (mut snap, mut with, mut snap_from) =
        (SnapRef::default(), ObjectRef::default(), SnapRef::default());
    // The other object of a `from` that is itself an intersection.
    let mut from_with = ObjectRef::default();
    let (mut snap_kind, mut from_kind): (Option<String>, Option<String>) = (None, None);
    let mut fonts = FontOptions::default();
    let value = |flag: &str, value: Option<OsString>| -> Result<String, Error> {
        value
            .and_then(|v| v.into_string().ok())
            .ok_or_else(|| usage(format!("{flag} needs a value")))
    };
    let once = |slot: &mut Option<String>, flag: &str, given: String| match slot.replace(given) {
        Some(_) => Err(Error::usage(format!("{flag} is given twice"))),
        None => Ok(()),
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--sheet") => once(&mut sheet, "--sheet", value("--sheet", args.next())?)?,
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => fonts.push(value("--fonts", args.next())?.into()),
            Some("--contains") if selecting => selection.matching = Matching::Contains,
            Some(flag @ ("--window" | "--crossing")) if selecting => {
                let given = value(flag, args.next())?;
                let numbers: Vec<f64> = given
                    .split(',')
                    .map(|n| n.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .ok()
                    .filter(|n: &Vec<f64>| n.len() == 4 && n.iter().all(|n| n.is_finite()))
                    .ok_or_else(|| {
                        usage(format!(
                            "{flag} is a box's two opposite corners, x1,y1,x2,y2, four numbers in \
                             the declared unit, not `{given}`"
                        ))
                    })?;
                let corners = SelectionBox([[numbers[0], numbers[1]], [numbers[2], numbers[3]]]);
                let slot = if flag == "--window" {
                    &mut selection.window
                } else {
                    &mut selection.crossing
                };
                if slot.replace(corners).is_some() {
                    return Err(Error::usage(format!("{flag} is given twice")));
                }
            }
            Some(flag @ ("--touching" | "--touching-name")) if selecting => {
                let touching = selection.touching.get_or_insert_with(ObjectRef::default);
                let slot = if flag == "--touching" {
                    &mut touching.id
                } else {
                    &mut touching.name
                };
                once(slot, flag, value(flag, args.next())?)?;
            }
            Some(
                flag @ ("--id" | "--name" | "--block" | "--tag" | "--value" | "--text" | "--group"
                | "--space"),
            ) if selecting => {
                let slot = match flag {
                    "--id" => &mut selection.id,
                    "--name" => &mut selection.name,
                    "--block" => &mut selection.block,
                    "--tag" => &mut selection.tag,
                    "--value" => &mut selection.value,
                    "--group" => &mut selection.group,
                    "--space" => &mut selection.space,
                    _ => &mut selection.text,
                };
                once(slot, flag, value(flag, args.next())?)?;
            }
            Some(
                flag @ ("--from" | "--from-name" | "--from-point" | "--to" | "--to-name"
                | "--to-point"),
            ) if kind == "measure" => {
                let slot = match flag {
                    "--from" => &mut from.id,
                    "--from-name" => &mut from.name,
                    "--from-point" => &mut from.point,
                    "--to" => &mut to.id,
                    "--to-name" => &mut to.name,
                    _ => &mut to.point,
                };
                once(slot, flag, value(flag, args.next())?)?;
            }
            Some(
                flag @ ("--id" | "--name" | "--snap" | "--which" | "--with" | "--with-name"
                | "--from" | "--from-name" | "--from-snap" | "--from-which" | "--from-with"
                | "--from-with-name"),
            ) if kind == "snap" => {
                let slot = match flag {
                    "--id" => &mut snap.id,
                    "--name" => &mut snap.name,
                    "--snap" => &mut snap_kind,
                    "--which" => &mut snap.which,
                    "--with" => &mut with.id,
                    "--with-name" => &mut with.name,
                    "--from" => &mut snap_from.id,
                    "--from-name" => &mut snap_from.name,
                    "--from-snap" => &mut from_kind,
                    "--from-with" => &mut from_with.id,
                    "--from-with-name" => &mut from_with.name,
                    _ => &mut snap_from.which,
                };
                once(slot, flag, value(flag, args.next())?)?;
            }
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("`query {kind}` takes no option `{flag}`")))
            }
            _ => {
                if source.replace(arg.into()).is_some() {
                    return Err(usage(
                        "query takes one drawing set's folder, sheet description or DWG".to_owned(),
                    ));
                }
            }
        }
    }
    let source = source.ok_or_else(|| {
        usage("query needs a drawing set's folder, a sheet description or a DWG".to_owned())
    })?;
    let snap_of = |name: Option<String>, flag: &str| -> Result<Option<SnapKind>, Error> {
        name.map(|name| {
            SnapKind::ALL
                .into_iter()
                .find(|k| k.name() == name)
                .ok_or_else(|| {
                    let kinds: Vec<&str> = SnapKind::ALL.iter().map(|k| k.name()).collect();
                    usage(format!(
                        "{flag} is one of {}, not `{name}`",
                        kinds.join(", ")
                    ))
                })
        })
        .transpose()
    };
    snap.snap = snap_of(snap_kind, "--snap")?;
    snap_from.snap = snap_of(from_kind, "--from-snap")?;
    if with != ObjectRef::default() {
        snap.with = Some(with);
    }
    if from_with != ObjectRef::default() {
        snap_from.with = Some(from_with);
    }
    if snap_from != SnapRef::default() {
        snap.from = Some(Box::new(snap_from));
    }
    let query = match kind.as_str() {
        "find" => Query::Find(selection),
        "geometry" => Query::Geometry(selection),
        "snap" => Query::Snap(snap),
        "attributes" => Query::Attributes(selection),
        "blocks" => Query::Blocks,
        "extents" => Query::Extents,
        "layers" => Query::Layers,
        "layout" => Query::Layout,
        "groups" => Query::Groups,
        "annotation-scales" | "annotation_scales" => Query::AnnotationScales,
        _ => Query::Measure(Measure { from, to }),
    };
    let request = QueryRequest {
        source,
        sheet,
        query,
    };
    input_of(request, fonts)
}

fn parse_check(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {CHECK_USAGE}"));
    let mut source: Option<PathBuf> = None;
    let mut sheets = Vec::new();
    let mut fonts = FontOptions::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--no-system-fonts") => fonts.no_system = true,
            Some(flag @ ("--sheet" | "--fonts")) => {
                let Some(value) = args.next() else {
                    return Err(usage(format!("{flag} needs a value")));
                };
                if flag == "--sheet" {
                    sheets.push(value.to_string_lossy().into_owned());
                } else {
                    fonts.push(PathBuf::from(value));
                }
            }
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("check takes no option `{flag}`")))
            }
            _ => {
                if source.replace(arg.into()).is_some() {
                    return Err(usage(
                        "check takes one drawing set's folder, sheet description or DWG".to_owned(),
                    ));
                }
            }
        }
    }
    let source = source.ok_or_else(|| {
        usage("check needs a drawing set's folder, a sheet description or a DWG".to_owned())
    })?;
    input_of(CheckRequest { source, sheets }, fonts)
}

fn parse_import_blocks(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {IMPORT_BLOCKS_USAGE}"));
    let mut drawing: Option<PathBuf> = None;
    let mut library: Option<PathBuf> = None;
    let mut blocks = Vec::new();
    let mut replace = false;
    let mut units: Option<Unit> = None;
    let mut dry_run = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some(flag @ ("--library" | "--block" | "--units")) => {
                let Some(value) = args.next() else {
                    return Err(usage(format!("{flag} needs a value")));
                };
                match flag {
                    "--library" => {
                        if library.replace(PathBuf::from(value)).is_some() {
                            return Err(usage("--library is given twice".to_owned()));
                        }
                    }
                    "--block" => blocks.push(value.to_string_lossy().into_owned()),
                    _ => {
                        let text = value.to_string_lossy();
                        let Some(unit) = Unit::from_symbol(&text) else {
                            return Err(usage(format!(
                                "--units takes one of {}, not `{text}`",
                                Unit::ALL.map(Unit::symbol).join(", ")
                            )));
                        };
                        if units.replace(unit).is_some() {
                            return Err(usage("--units is given twice".to_owned()));
                        }
                    }
                }
            }
            Some("--replace") => replace = true,
            Some("--dry-run") => dry_run = true,
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("import-blocks takes no option `{flag}`")))
            }
            _ => {
                if drawing.replace(arg.into()).is_some() {
                    return Err(usage("import-blocks takes one DWG or DXF".to_owned()));
                }
            }
        }
    }
    let drawing = drawing.ok_or_else(|| usage("import-blocks needs a DWG or a DXF".to_owned()))?;
    let library = library.ok_or_else(|| {
        usage("import-blocks needs the block library to write, under --library".to_owned())
    })?;
    input_of(
        ImportBlocksRequest {
            drawing,
            library,
            blocks,
            replace,
            units,
            dry_run,
        },
        FontOptions::default(),
    )
}

/// Reads `read`'s arguments: the original, and where to write its text
/// mirror, or that none is written.
fn parse_read(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {READ_USAGE}"));
    let value = |flag: &str, value: Option<OsString>| -> Result<OsString, Error> {
        match value {
            Some(path)
                if path
                    .to_str()
                    .is_some_and(|text| text.starts_with('-') && text.len() > 1) =>
            {
                Err(usage(format!(
                    "{flag} needs a path, not option `{}`",
                    path.to_string_lossy()
                )))
            }
            Some(path) => Ok(path),
            None => Err(usage(format!("{flag} needs a path"))),
        }
    };
    let mut source: Option<PathBuf> = None;
    let mut mirror: Option<PathBuf> = None;
    let mut no_mirror = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--mirror") => {
                let path = value("--mirror", args.next())?;
                if mirror.replace(path.into()).is_some() {
                    return Err(usage("--mirror is given twice".to_owned()));
                }
            }
            Some("--no-mirror") => no_mirror = true,
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("read takes no option `{flag}`")))
            }
            _ => {
                if source.replace(arg.into()).is_some() {
                    return Err(usage("read takes one DWG".to_owned()));
                }
            }
        }
    }
    let source = source.ok_or_else(|| usage("read needs the DWG to read".to_owned()))?;
    input_of(
        ReadRequest {
            source,
            mirror,
            no_mirror,
        },
        FontOptions::default(),
    )
}

fn parse_convert(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {CONVERT_USAGE}"));
    let mut source: Option<PathBuf> = None;
    let mut version: Option<DwgVersion> = None;
    let mut output: Option<PathBuf> = None;
    let mut format: Option<DrawingFormat> = None;
    let mut overwrite = false;
    let mut allow_loss = false;
    let mut dry_run = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some(flag @ ("--version" | "--to")) => {
                let raw = args
                    .next()
                    .ok_or_else(|| usage(format!("{flag} needs a DWG version")))?
                    .into_string()
                    .map_err(|_| usage("the DWG version is not valid text".to_owned()))?;
                let parsed = DwgVersion::parse(&raw).ok_or_else(|| {
                    usage(format!(
                        "unknown DWG version `{raw}`; choose one of {}",
                        DwgVersion::list_choices()
                    ))
                })?;
                if version.replace(parsed).is_some() {
                    return Err(usage("--version is given twice".to_owned()));
                }
            }
            Some("--output" | "-o") => {
                let path = args
                    .next()
                    .ok_or_else(|| usage("--output needs a path".to_owned()))?;
                if output.replace(path.into()).is_some() {
                    return Err(usage("--output is given twice".to_owned()));
                }
            }
            Some("--format") => {
                let raw = args
                    .next()
                    .ok_or_else(|| usage("--format needs a value".to_owned()))?
                    .into_string()
                    .map_err(|_| usage("the format is not valid text".to_owned()))?;
                let parsed = DrawingFormat::from_name(&raw).ok_or_else(|| {
                    usage(format!(
                        "unknown --format `{raw}`; choose `dwg`, `dxf` or `dxf_binary`"
                    ))
                })?;
                if format.replace(parsed).is_some() {
                    return Err(usage("--format is given twice".to_owned()));
                }
            }
            Some("--overwrite") => overwrite = true,
            Some("--allow-loss") => allow_loss = true,
            Some("--dry-run") => dry_run = true,
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("convert takes no option `{flag}`")))
            }
            _ => {
                if source.replace(arg.into()).is_some() {
                    return Err(usage(
                        "convert takes one drawing, folder or drawing set".to_owned(),
                    ));
                }
            }
        }
    }
    let source = source.ok_or_else(|| {
        usage("convert needs the drawing, folder or drawing set to convert".to_owned())
    })?;
    let version = version.ok_or_else(|| {
        usage(
            "convert needs the target DWG version under --version (such as 2010 or AC1024)"
                .to_owned(),
        )
    })?;
    input_of(
        ConvertRequest {
            source,
            version,
            output,
            format,
            overwrite,
            allow_loss,
            dry_run,
        },
        FontOptions::default(),
    )
}

fn parse_build(mut args: Args) -> Result<Value, Error> {
    let mut description: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut format: Option<DrawingFormat> = None;
    let mut dwg_version: Option<DwgVersion> = None;
    let mut overwrite = false;
    let mut clean: Option<bool> = None;
    let mut fonts = FontOptions::default();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => fonts.push(
                args.next()
                    .ok_or_else(|| {
                        Error::usage(format!("--fonts needs a folder; usage: {BUILD_USAGE}"))
                    })?
                    .into(),
            ),
            Some("--output" | "-o") => {
                let path = args.next().ok_or_else(|| {
                    Error::usage(format!("--output needs a path; usage: {BUILD_USAGE}"))
                })?;
                if output.replace(path.into()).is_some() {
                    return Err(Error::usage("--output is given twice"));
                }
            }
            Some("--format") => {
                let raw = args
                    .next()
                    .ok_or_else(|| {
                        Error::usage(format!("--format needs a value; usage: {BUILD_USAGE}"))
                    })?
                    .into_string()
                    .map_err(|_| Error::usage("the format is not valid text"))?;
                let parsed = DrawingFormat::from_name(&raw).ok_or_else(|| {
                    Error::usage(format!(
                        "unknown --format `{raw}`; choose `dwg`, `dxf` or `dxf_binary`; usage: \
                         {BUILD_USAGE}"
                    ))
                })?;
                if format.replace(parsed).is_some() {
                    return Err(Error::usage("--format is given twice"));
                }
            }
            Some("--dwg-version") => {
                let raw = args
                    .next()
                    .ok_or_else(|| {
                        Error::usage(format!("--dwg-version needs a value; usage: {BUILD_USAGE}"))
                    })?
                    .into_string()
                    .map_err(|_| Error::usage("the DWG version is not valid text"))?;
                let parsed = DwgVersion::parse(&raw).ok_or_else(|| {
                    Error::usage(format!(
                        "unknown --dwg-version `{raw}`; choose one of {}; usage: {BUILD_USAGE}",
                        DwgVersion::list_choices()
                    ))
                })?;
                if dwg_version.replace(parsed).is_some() {
                    return Err(Error::usage("--dwg-version is given twice"));
                }
            }
            Some("--overwrite") => overwrite = true,
            Some(flag @ ("--clean" | "--no-clean")) => set_clean(&mut clean, flag)?,
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(Error::usage(format!(
                    "unknown option `{flag}`; usage: {BUILD_USAGE}"
                )))
            }
            _ => {
                if description.replace(arg.into()).is_some() {
                    return Err(Error::usage(format!(
                        "build takes one sheet description or drawing set; usage: {BUILD_USAGE}"
                    )));
                }
            }
        }
    }
    let source = description.ok_or_else(|| {
        Error::usage(format!(
            "build needs a sheet description or a drawing set's folder; usage: {BUILD_USAGE}"
        ))
    })?;
    let request = SourceArgs {
        source,
        output,
        format,
        dwg_version,
        overwrite,
        clean,
        plot_style: None,
        plot_styles: Vec::new(),
    };
    input_of(request, fonts)
}

fn parse_measure(mut args: Args) -> Result<Value, Error> {
    let mut contents: Option<String> = None;
    let mut font: Option<String> = None;
    let mut height: Option<f64> = None;
    let mut width_factor: Option<f64> = None;
    let mut fonts = FontOptions::default();
    let value = |flag: &str, value: Option<OsString>| -> Result<String, Error> {
        value
            .and_then(|v| v.into_string().ok())
            .ok_or_else(|| Error::usage(format!("{flag} needs a value; usage: {MEASURE_USAGE}")))
    };
    let number = |flag: &str, text: String| -> Result<f64, Error> {
        let number = text.parse().map_err(|_| {
            Error::usage(format!(
                "{flag} takes a number, found `{text}`; usage: {MEASURE_USAGE}"
            ))
        })?;
        float(flag, number)
    };
    let once = |flag: &str| Error::usage(format!("{flag} is given twice"));
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--font") => {
                if font.replace(value("--font", args.next())?).is_some() {
                    return Err(once("--font"));
                }
            }
            Some("--height") => {
                let h = number("--height", value("--height", args.next())?)?;
                if height.replace(h).is_some() {
                    return Err(once("--height"));
                }
            }
            Some("--width-factor") => {
                let f = number("--width-factor", value("--width-factor", args.next())?)?;
                if width_factor.replace(f).is_some() {
                    return Err(once("--width-factor"));
                }
            }
            Some("--no-system-fonts") => fonts.no_system = true,
            Some("--fonts") => fonts.push(value("--fonts", args.next())?.into()),
            Some(flag) if flag.starts_with("--") => {
                return Err(Error::usage(format!(
                    "unknown option `{flag}`; usage: {MEASURE_USAGE}"
                )))
            }
            _ => {
                let text = arg
                    .into_string()
                    .map_err(|_| Error::usage("the contents are not valid text".to_owned()))?;
                if contents.replace(text).is_some() {
                    return Err(Error::usage(format!(
                        "measure-text takes one line of contents; quote it if it has spaces; usage: {MEASURE_USAGE}"
                    )));
                }
            }
        }
    }
    let missing =
        |what: &str| Error::usage(format!("measure-text needs {what}; usage: {MEASURE_USAGE}"));
    let request = MeasureTextRequest {
        contents: contents.ok_or_else(|| missing("the contents to measure"))?,
        font: font.ok_or_else(|| missing("--font"))?,
        height: height.ok_or_else(|| missing("--height"))?,
        width_factor: width_factor.unwrap_or(1.0),
    };
    input_of(request, fonts)
}

/// The command line's help: each command's usage and what it does, from
/// the registry, and the servers'.
fn help() -> Value {
    let mut listed = Map::new();
    for command in commands() {
        let usage = cli_form(command.name()).map(|(usage, _)| usage);
        listed.insert(
            command.name().to_owned(),
            json!({ "usage": usage, "does": command.does() }),
        );
    }
    listed.insert(
        "serve".to_owned(),
        json!({ "usage": http::SERVE_USAGE, "does": http::serve_does() }),
    );
    listed.insert(
        "mcp".to_owned(),
        json!({ "usage": mcp::MCP_USAGE, "does": mcp::MCP_DOES }),
    );
    json!({
        "name": "agentdrafter",
        "version": env!("CARGO_PKG_VERSION"),
        "commands": listed,
        "options": "Each command's options give its input, which the HTTP API and the MCP server \
                    take as a JSON object of the same names, - written _: --dry-run is dry_run, \
                    --plot-style plot_style, and an option given more than once, such as \
                    --fonts, a list. The input's schema is listed by `agentdrafter serve` at GET \
                    /commands and by `agentdrafter mcp` as each tool's input schema.",
        "fonts": format!(
            "Fonts are read from the folders --fonts names, then from those the {FONTS_VARIABLE} \
             environment variable lists (separated as the system separates paths), then, for a \
             TrueType font, from the system's own font folders, unless --no-system-fonts or \
             {SYSTEM_FONTS_VARIABLE}=0 leaves them out. A font in none of them is replaced by a \
             look-alike: the reply says which, and warns of it under `warnings` at its top."
        ),
        "plot_styles": format!(
            "CTB and STB plot styles are read from the folders --plot-styles names, then from those the \
             {PLOT_STYLES_VARIABLE} environment variable lists, such as AutoCAD's Plot Styles \
             folder. There is no default folder."
        ),
        "output": "One line of JSON on standard output: {\"ok\": true, \"result\": ...} or \
                   {\"ok\": false, \"error\": {\"code\", \"message\", \"field\", \"line\", \
                   \"column\", \"file\"}}.",
        "exit_codes": {
            "0": "ok",
            "1": "the core reported an error, or a sheet of a drawing set failed",
            "2": "wrong arguments",
        },
    })
}

fn reply<T: Serialize>(outcome: Result<T, Error>) -> (String, ExitCode) {
    send(Reply::from(outcome))
}

fn send<T: Serialize>(reply: Reply<T>) -> (String, ExitCode) {
    let code = match &reply.error {
        _ if reply.ok => ExitCode::SUCCESS,
        Some(error) if error.code == ErrorCode::Usage => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    };
    // A reply holds only strings, numbers, lists and string-keyed maps, which
    // always serialize.
    let line = serde_json::to_string(&reply).expect("a reply serializes to JSON");
    (line, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_of_the_registry_has_a_command_line_form() {
        assert!(!commands().is_empty());
        for command in commands() {
            let (usage, _) = cli_form(command.name()).unwrap_or_else(|| {
                panic!(
                    "`{}` is in the registry but has no command-line form",
                    command.name()
                )
            });
            assert!(
                usage.starts_with(&format!("agentdrafter {} ", command.name()))
                    || usage == format!("agentdrafter {}", command.name()),
                "{usage}"
            );
            let help = help();
            assert_eq!(help["commands"][command.name()]["usage"], usage);
            assert_eq!(help["commands"][command.name()]["does"], command.does());
        }
    }
}
