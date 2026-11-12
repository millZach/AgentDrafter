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

mod http;
mod mcp;
mod page;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use agentdrafter_core::{commands, find_command, Error, ErrorCode, Reply, Settings};
use agentdrafter_core::{
    ChangeRequest, CheckRequest, DisplayListRequest, JournalRequest, MeasureTextRequest,
    RenderRequest, StaleRequest, UndoRequest, FONTS_VARIABLE, PLOT_STYLES_VARIABLE,
};
use agentdrafter_core::{End, Matching, Measure, Query, QueryRequest, Selection};
use serde::Serialize;
use serde_json::{json, Map, Value};

const BUILD_USAGE: &str =
    "agentdrafter build <sheet-description | drawing-set-folder> [--output <dwg>] [--overwrite] [--fonts <folder>]...";
const SCHEMA_USAGE: &str =
    "agentdrafter schema [sheet-description | block-library | template | set | rows | change-set]";
const OPERATIONS_USAGE: &str = "agentdrafter operations";
const CHANGE_USAGE: &str = "agentdrafter change <drawing-set-folder> <change-set.json | -> [--dry-run] [--fonts <folder>]...";
const JOURNAL_USAGE: &str = "agentdrafter journal <drawing-set-folder>";
const UNDO_USAGE: &str = "agentdrafter undo <drawing-set-folder> <change-set> [--dry-run]";
const MEASURE_USAGE: &str = "agentdrafter measure-text <contents> --font <file> --height <height> [--width-factor <factor>] [--fonts <folder>]...";
const RENDER_USAGE: &str = "agentdrafter render <sheet-description-or-dwg> [--output <png>] [--overwrite] [--dpi <dpi>] [--scale <scale> | --width <pixels> --height <pixels>] [--margin <mm>] [--fonts <folder>]...";
const DISPLAY_LIST_USAGE: &str = "agentdrafter display-list <sheet-description | dwg | drawing-set-folder> [--sheet <sheet>] [--tolerance <length>] [--fonts <folder>]...";
const PLOT_USAGE: &str = "agentdrafter plot <sheet-description | dwg | drawing-set-folder> [--output <pdf>] [--overwrite] [--plot-style <ctb>] [--plot-styles <folder>]... [--fonts <folder>]...";
const STALE_USAGE: &str =
    "agentdrafter stale <png-or-pdf> [--source <sheet-description-dwg-or-template>]";
const CHECK_USAGE: &str = "agentdrafter check <drawing-set-folder | sheet-description> [--sheet <sheet>]... [--fonts <folder>]...";
const QUERY_USAGE: &str = "agentdrafter query <find | blocks | attributes | measure | extents> <drawing-set-folder | sheet-description> [--sheet <sheet>] [find and attributes: --id <id> --name <name> --block <block> --tag <tag> --value <value> --text <text> --contains] [measure: --from <id> | --from-name <name>, --to <id> | --to-name <name>, --from-point <point> --to-point <point>] [extents: --fonts <folder>...]";

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
        "check" => (CHECK_USAGE, parse_check),
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
        Ok(input) => send(command.run(input, &Settings::from_env())),
        Err(error) => reply::<()>(Err(error)),
    }
}

/// The input a core request gives as plain data, with the font folders
/// `--fonts` names. A path that is not valid text cannot be sent as JSON,
/// through any interface.
fn input_of(request: impl Serialize, fonts: Vec<PathBuf>) -> Result<Value, Error> {
    let not_text = |e: serde_json::Error| {
        Error::usage(format!(
            "every interface takes paths and values as text (UTF-8), and one given is not: {e}"
        ))
    };
    let mut input = serde_json::to_value(request).map_err(not_text)?;
    if !fonts.is_empty() {
        input["fonts"] = serde_json::to_value(fonts).map_err(not_text)?;
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
    let mut fonts = Vec::new();
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
    input_of(request, fonts)
}

fn parse_display_list(mut args: Args) -> Result<Value, Error> {
    let mut request = DisplayListRequest::new(PathBuf::new());
    let mut source: Option<PathBuf> = None;
    let mut fonts = Vec::new();
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

/// The input of `build` and `plot`: one sheet description, DWG or drawing
/// set's folder, which the core tells apart.
#[derive(Serialize)]
struct SourceArgs {
    source: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<PathBuf>,
    overwrite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    plot_style: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    plot_styles: Vec<PathBuf>,
}

fn parse_plot(mut args: Args) -> Result<Value, Error> {
    let mut source: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut overwrite = false;
    let mut plot_style: Option<String> = None;
    let mut fonts = Vec::new();
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
            Some("--plot-style") => {
                let name = value("--plot-style", args.next())?
                    .into_string()
                    .map_err(|_| Error::usage("the plot style's name is not valid text"))?;
                if plot_style.replace(name).is_some() {
                    return Err(Error::usage("--plot-style is given twice"));
                }
            }
            Some("--plot-styles") => plot_styles.push(value("--plot-styles", args.next())?.into()),
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
        overwrite,
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
    input_of(StaleRequest { output, source }, Vec::new())
}

/// Reads `change`'s arguments: the set's folder, the change set's JSON file
/// or `-` for standard input, and the options.
fn parse_change(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {CHANGE_USAGE}"));
    let mut given: Vec<OsString> = Vec::new();
    let mut dry_run = false;
    let mut fonts = Vec::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--dry-run") => dry_run = true,
            Some("--fonts") => match args.next() {
                Some(folder) => fonts.push(PathBuf::from(folder)),
                None => return Err(usage("--fonts needs a value".to_owned())),
            },
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("change takes no option `{flag}`")))
            }
            _ => given.push(arg),
        }
    }
    let [set, change_set] = <[OsString; 2]>::try_from(given).map_err(|given| {
        usage(format!(
            "change takes a drawing set's folder and a change set, found {} arguments",
            given.len()
        ))
    })?;
    let text = if change_set == "-" {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the change set from standard input: {e}"),
            )
        })?;
        text
    } else {
        let path = PathBuf::from(&change_set);
        std::fs::read_to_string(&path).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the change set `{}`: {e}", path.display()),
            )
        })?
    };
    let change_set = serde_json::from_str(&text).map_err(|e| {
        Error::new(
            ErrorCode::WrongType,
            format!("the change set is not JSON: {e}"),
        )
    })?;
    let request = ChangeRequest {
        set: PathBuf::from(set),
        change_set,
        dry_run,
    };
    input_of(request, fonts)
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
            "journal takes a drawing set's folder, found {} arguments; usage: {JOURNAL_USAGE}",
            given.len()
        ))
    })?;
    let request = JournalRequest {
        set: PathBuf::from(set),
    };
    input_of(request, Vec::new())
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
            "undo takes a drawing set's folder and a change set, found {} arguments",
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
    input_of(request, Vec::new())
}

fn parse_query(mut args: Args) -> Result<Value, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {QUERY_USAGE}"));
    let kind = match args.next().as_ref().map(|a| a.to_string_lossy()) {
        Some(kind) => kind.into_owned(),
        None => return Err(usage("query needs what to ask".to_owned())),
    };
    let selecting = matches!(kind.as_str(), "find" | "attributes");
    if !selecting && !matches!(kind.as_str(), "blocks" | "measure" | "extents") {
        return Err(usage(format!(
            "query asks find, blocks, attributes, measure or extents, not `{kind}`"
        )));
    }
    let mut source: Option<PathBuf> = None;
    let mut sheet: Option<String> = None;
    let mut selection = Selection::default();
    let (mut from, mut to) = (End::default(), End::default());
    let mut fonts = Vec::new();
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
            Some("--fonts") => fonts.push(value("--fonts", args.next())?.into()),
            Some("--contains") if selecting => selection.matching = Matching::Contains,
            Some(flag @ ("--id" | "--name" | "--block" | "--tag" | "--value" | "--text"))
                if selecting =>
            {
                let slot = match flag {
                    "--id" => &mut selection.id,
                    "--name" => &mut selection.name,
                    "--block" => &mut selection.block,
                    "--tag" => &mut selection.tag,
                    "--value" => &mut selection.value,
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
            Some(flag) if flag.starts_with('-') && flag.len() > 1 => {
                return Err(usage(format!("`query {kind}` takes no option `{flag}`")))
            }
            _ => {
                if source.replace(arg.into()).is_some() {
                    return Err(usage(
                        "query takes one drawing set's folder or sheet description".to_owned(),
                    ));
                }
            }
        }
    }
    let source = source.ok_or_else(|| {
        usage("query needs a drawing set's folder or a sheet description".to_owned())
    })?;
    let query = match kind.as_str() {
        "find" => Query::Find(selection),
        "attributes" => Query::Attributes(selection),
        "blocks" => Query::Blocks,
        "extents" => Query::Extents,
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
    let mut fonts = Vec::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
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
                        "check takes one drawing set's folder or sheet description".to_owned(),
                    ));
                }
            }
        }
    }
    let source = source.ok_or_else(|| {
        usage("check needs a drawing set's folder or a sheet description".to_owned())
    })?;
    input_of(CheckRequest { source, sheets }, fonts)
}

fn parse_build(mut args: Args) -> Result<Value, Error> {
    let mut description: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut overwrite = false;
    let mut fonts = Vec::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
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
            Some("--overwrite") => overwrite = true,
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
        overwrite,
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
    let mut fonts = Vec::new();
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
             environment variable lists (separated as the system separates paths). There is no \
             default folder. A font in none of them is replaced by a look-alike, and the reply \
             says which."
        ),
        "plot_styles": format!(
            "CTB plot styles are read from the folders --plot-styles names, then from those the \
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
