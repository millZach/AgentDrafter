//! `agentdrafter`: the command line over AgentDrafter's core.
//!
//! This is a thin layer (ADR-0007). It turns arguments into a core request,
//! calls the core, and prints the core's reply as one line of JSON on
//! standard output, on success and on failure. Drawing logic here is a bug.
//!
//! Exit codes: 0 when the reply is `ok`, 1 when the core reports an error,
//! such as a sheet of a drawing set that failed, 2 when the arguments are
//! wrong.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agentdrafter_core::{
    block_library_schema, build_set_with, build_with, check_stale, measure_text_with,
    plot_set_with, plot_with, render_with, rows_schema, set_schema, sheet_description_schema,
    template_schema, BuildRequest, Error, ErrorCode, MeasureTextRequest, PlotRequest,
    PlotSetRequest, RenderRequest, Reply, SetBuildRequest, Settings, StaleRequest,
    BUILT_IN_PLOT_STYLE, FONTS_VARIABLE, PLOT_STYLES_VARIABLE, SET_SETTINGS_FILE,
};
use agentdrafter_core::{change_set_schema, change_with, operations, ChangeRequest};
use agentdrafter_core::{journal, undo, JournalRequest, UndoRequest};
use agentdrafter_core::{query_with, End, Matching, Measure, Query, QueryRequest, Selection};
use serde::Serialize;
use serde_json::json;

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
const PLOT_USAGE: &str = "agentdrafter plot <sheet-description | dwg | drawing-set-folder> [--output <pdf>] [--overwrite] [--plot-style <ctb>] [--plot-styles <folder>]... [--fonts <folder>]...";
const STALE_USAGE: &str =
    "agentdrafter stale <png-or-pdf> [--source <sheet-description-dwg-or-template>]";
const QUERY_USAGE: &str = "agentdrafter query <find | blocks | attributes | measure | extents> <drawing-set-folder | sheet-description> [--sheet <sheet>] [find and attributes: --id <id> --name <name> --block <block> --tag <tag> --value <value> --text <text> --contains] [measure: --from <id> | --from-name <name>, --to <id> | --to-name <name>, --from-point <point> --to-point <point>] [extents: --fonts <folder>...]";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let (line, code) = run(args);
    println!("{line}");
    code
}

/// Runs one command and returns the JSON line to print and the exit code.
fn run(args: Vec<OsString>) -> (String, ExitCode) {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return reply::<()>(Err(Error::usage(format!(
            "no command given; usage: {BUILD_USAGE} | {SCHEMA_USAGE} | {MEASURE_USAGE} | {RENDER_USAGE} | {PLOT_USAGE} | {STALE_USAGE} | {QUERY_USAGE} | {OPERATIONS_USAGE} | {CHANGE_USAGE} | {JOURNAL_USAGE} | {UNDO_USAGE}"
        ))));
    };
    match command.to_str() {
        Some("build") => match parse_build(args) {
            Ok((Build::Sheet(request), settings)) => reply(build_with(&request, &settings)),
            Ok((Build::Set(request), settings)) => send(build_set_with(&request, &settings)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("measure-text") => match parse_measure(args) {
            Ok((request, settings)) => reply(measure_text_with(&request, &settings)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("render") => match parse_render(args) {
            Ok((request, settings)) => reply(render_with(&request, &settings)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("plot") => match parse_plot(args) {
            Ok((Plot::Sheet(request), settings)) => reply(plot_with(&request, &settings)),
            Ok((Plot::Set(request), settings)) => send(plot_set_with(&request, &settings)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("stale") => match parse_stale(args) {
            Ok(request) => reply(check_stale(&request)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("query") => match parse_query(args) {
            Ok((request, settings)) => reply(query_with(&request, &settings)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("operations") => match args.next() {
            None => reply(Ok(operations())),
            Some(extra) => reply::<()>(Err(Error::usage(format!(
                "operations takes no argument, found `{}`; usage: {OPERATIONS_USAGE}",
                extra.to_string_lossy()
            )))),
        },
        Some("change") => match parse_change(args) {
            Ok((request, settings)) => send(change_with(&request, &settings)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("journal") => match parse_journal(args) {
            Ok(request) => send(journal(&request)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("undo") => match parse_undo(args) {
            Ok(request) => send(undo(&request)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("schema") => {
            let which = args.next();
            match (which.as_ref().map(|a| a.to_str()), args.next()) {
                (None | Some(Some("sheet-description")), None) => {
                    reply(Ok(sheet_description_schema()))
                }
                (Some(Some("block-library")), None) => reply(Ok(block_library_schema())),
                (Some(Some("template")), None) => reply(Ok(template_schema())),
                (Some(Some("set")), None) => reply(Ok(set_schema())),
                (Some(Some("rows")), None) => reply(Ok(rows_schema())),
                (Some(Some("change-set")), None) => reply(Ok(change_set_schema())),
                (_, Some(extra)) => reply::<()>(Err(Error::usage(format!(
                    "schema takes at most one argument, found `{}` as well; usage: {SCHEMA_USAGE}",
                    extra.to_string_lossy()
                )))),
                (Some(_), None) => reply::<()>(Err(Error::usage(format!(
                    "schema prints the schema of a sheet description, a block library, a template, a drawing set's settings, its rows or a change set, not `{}`; usage: {SCHEMA_USAGE}",
                    which.unwrap_or_default().to_string_lossy()
                )))),
            }
        }
        Some("help" | "--help" | "-h") => reply(Ok(help())),
        Some("--version" | "-V") => reply(Ok(json!({ "version": env!("CARGO_PKG_VERSION") }))),
        _ => reply::<()>(Err(Error::usage(format!(
            "unknown command `{}`; usage: {BUILD_USAGE} | {SCHEMA_USAGE} | {MEASURE_USAGE} | {RENDER_USAGE} | {PLOT_USAGE} | {STALE_USAGE} | {QUERY_USAGE} | {OPERATIONS_USAGE} | {CHANGE_USAGE} | {JOURNAL_USAGE} | {UNDO_USAGE}",
            command.to_string_lossy()
        )))),
    }
}

fn parse_render(
    mut args: impl Iterator<Item = OsString>,
) -> Result<(RenderRequest, Settings), Error> {
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
                    .replace(number("--dpi", value("--dpi", args.next())?)?)
                    .is_some()
                {
                    return Err(once("--dpi"));
                }
            }
            Some("--margin") => {
                let mm = number("--margin", value("--margin", args.next())?)?;
                if margin.replace(mm).is_some() {
                    return Err(once("--margin"));
                }
            }
            Some("--scale") => {
                let scale = number("--scale", value("--scale", args.next())?)?;
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
    Ok((request, settings(fonts)))
}

/// What `plot` plots: one sheet description or DWG, or every sheet of a
/// drawing set, whose folder it is given.
enum Plot {
    Sheet(PlotRequest),
    Set(PlotSetRequest),
}

fn parse_plot(mut args: impl Iterator<Item = OsString>) -> Result<(Plot, Settings), Error> {
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
    let mut settings = settings(fonts);
    settings.plot_style_folders.splice(0..0, plot_styles);
    // A folder is a drawing set, whose settings say where its outputs go.
    if source.is_dir() {
        if output.is_some() {
            return Err(Error::usage(format!(
                "a drawing set's PDFs go to the folder its settings ({SET_SETTINGS_FILE}) name \
                 under `output`; --output is for one sheet description or DWG"
            )));
        }
        let request = PlotSetRequest {
            set: source,
            overwrite,
            plot_style,
        };
        return Ok((Plot::Set(request), settings));
    }
    let request = PlotRequest {
        source,
        output,
        overwrite,
        plot_style,
    };
    Ok((Plot::Sheet(request), settings))
}

fn parse_stale(mut args: impl Iterator<Item = OsString>) -> Result<StaleRequest, Error> {
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
    Ok(StaleRequest { output, source })
}

/// Reads `change`'s arguments: the set's folder, the change set's JSON file
/// or `-` for standard input, and the options.
fn parse_change(
    mut args: impl Iterator<Item = OsString>,
) -> Result<(ChangeRequest, Settings), Error> {
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
    Ok((
        ChangeRequest {
            set: PathBuf::from(set),
            change_set,
            dry_run,
        },
        settings(fonts),
    ))
}

/// Reads `journal`'s argument: the set's folder.
fn parse_journal(args: impl Iterator<Item = OsString>) -> Result<JournalRequest, Error> {
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
    Ok(JournalRequest {
        set: PathBuf::from(set),
    })
}

/// Reads `undo`'s arguments: the set's folder, the change set's entry in
/// the journal, and the options.
fn parse_undo(args: impl Iterator<Item = OsString>) -> Result<UndoRequest, Error> {
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
    Ok(UndoRequest {
        set: PathBuf::from(set),
        change_set,
        dry_run,
    })
}

fn parse_query(
    mut args: impl Iterator<Item = OsString>,
) -> Result<(QueryRequest, Settings), Error> {
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
    Ok((
        QueryRequest {
            source,
            sheet,
            query,
        },
        settings(fonts),
    ))
}

/// The settings for one command: the font folders `--fonts` names, then
/// those the environment names, and the plot style folders the environment
/// names.
fn settings(fonts: Vec<PathBuf>) -> Settings {
    let mut settings = Settings::from_env();
    settings.font_folders.splice(0..0, fonts);
    settings
}

/// What `build` builds: one sheet description, or every sheet of a drawing
/// set, whose folder it is given.
enum Build {
    Sheet(BuildRequest),
    Set(SetBuildRequest),
}

fn parse_build(mut args: impl Iterator<Item = OsString>) -> Result<(Build, Settings), Error> {
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
    let description = description.ok_or_else(|| {
        Error::usage(format!(
            "build needs a sheet description or a drawing set's folder; usage: {BUILD_USAGE}"
        ))
    })?;
    // A folder is a drawing set, whose settings say where its DWGs go.
    if Path::new(&description).is_dir() {
        if output.is_some() {
            return Err(Error::usage(format!(
                "a drawing set's DWGs go to the folder its settings ({SET_SETTINGS_FILE}) name \
                 under `output`; --output is for one sheet description"
            )));
        }
        let request = SetBuildRequest {
            set: description,
            overwrite,
        };
        return Ok((Build::Set(request), settings(fonts)));
    }
    let request = BuildRequest {
        description,
        output,
        overwrite,
    };
    Ok((Build::Sheet(request), settings(fonts)))
}

fn parse_measure(
    mut args: impl Iterator<Item = OsString>,
) -> Result<(MeasureTextRequest, Settings), Error> {
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
        text.parse().map_err(|_| {
            Error::usage(format!(
                "{flag} takes a number, found `{text}`; usage: {MEASURE_USAGE}"
            ))
        })
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
    Ok((
        MeasureTextRequest {
            contents: contents.ok_or_else(|| missing("the contents to measure"))?,
            font: font.ok_or_else(|| missing("--font"))?,
            height: height.ok_or_else(|| missing("--height"))?,
            width_factor: width_factor.unwrap_or(1.0),
        },
        settings(fonts),
    ))
}

fn help() -> serde_json::Value {
    json!({
        "name": "agentdrafter",
        "version": env!("CARGO_PKG_VERSION"),
        "commands": {
            "build": {
                "usage": BUILD_USAGE,
                "does": "Builds a sheet description into a DWG in the 2018 format (AC1032), \
                         with the blocks it inserts from the block library it names. The DWG \
                         goes beside the sheet description with a .dwg extension unless --output \
                         names another path. An existing file is never replaced without \
                         --overwrite, and --overwrite replaces only a DWG AgentDrafter built. The \
                         reply lists the blocks the DWG defines, and each text style whose font \
                         is not in the font folders, with the look-alike measured in its place.",
                "drawing_set": format!(
                    "Given a folder, builds the drawing set in it: one DWG per row of its rows, \
                     each its template filled in from the row, into the output folder its \
                     settings ({SET_SETTINGS_FILE}) name. A sheet that fails writes nothing and \
                     the others are still built; the reply gives each sheet's outcome under \
                     sheets, and is not ok when any sheet failed."
                ),
            },
            "schema": {
                "usage": SCHEMA_USAGE,
                "does": "Prints the sheet description's schema, or the block library's, a \
                         template's, a drawing set's settings' or its rows': a JSON Schema \
                         (draft 2020-12) of every key it may hold, with what each means.",
            },
            "measure-text": {
                "usage": MEASURE_USAGE,
                "does": "Measures how wide one line of text draws in a font at a height, as \
                         AutoCAD measures a text object: from the left edge of what it draws to \
                         the right edge. Reports the font used, and the look-alike when the font \
                         is not in the font folders.",
            },
            "render": {
                "usage": RENDER_USAGE,
                "does": "Draws a sheet description, or a DWG, to a PNG as AutoCAD plots it: layer \
                         colours on white paper, lineweights as widths on paper at --dpi (150), \
                         linetypes, text in its fonts and hatches. The PNG covers the drawing's \
                         extents with a --margin (5 mm of paper), at a plot --scale (1 for 1:1, \
                         0.02 for 1:50) or fitted to --width and --height pixels; without either, \
                         its longer side is 2000 pixels. It goes beside the source with a .png \
                         extension unless --output names another path, and is stamped with the \
                         source's fingerprint. An existing file is never replaced without \
                         --overwrite, and --overwrite replaces only a PNG AgentDrafter rendered.",
            },
            "plot": {
                "usage": PLOT_USAGE,
                "does": format!(
                    "Plots a sheet description, or a DWG, to a PDF as AutoCAD plots it to PDF: \
                     the page is the sheet's size, its extents at 1:1 (a sheet drawn 17 by 11 \
                     inches plots on a page of exactly 431.8 by 279.4 mm); each pen's colour \
                     and lineweight come from the CTB plot style --plot-style names, found in \
                     the plot style folders, or with none from the {BUILT_IN_PLOT_STYLE} style, \
                     every pen black at its object's own lineweight; text draws as AutoCAD \
                     draws it and can be searched and copied. It goes beside the source with a \
                     .pdf extension unless --output names another path, and is stamped with \
                     every file it was drawn from. An existing file is never replaced without \
                     --overwrite, and --overwrite replaces only a PDF AgentDrafter plotted."
                ),
                "drawing_set": format!(
                    "Given a folder, plots every sheet of the drawing set in it, each its \
                     template filled in from its row, into the output folder its settings \
                     ({SET_SETTINGS_FILE}) name, as <sheet>.pdf. A sheet that fails writes \
                     nothing and the others are still plotted; the reply gives each sheet's \
                     outcome under sheets, and is not ok when any sheet failed."
                ),
            },
            "stale": {
                "usage": STALE_USAGE,
                "does": "Says whether a PNG AgentDrafter rendered or a PDF it plotted is stale: \
                         whether any file its stamp names (the sheet description, DWG or \
                         template, and the files it draws on) has changed since, or is gone. \
                         The files are found from the output's folder; --source says where the \
                         first, the record, is now.",
            },
            "query": {
                "usage": QUERY_USAGE,
                "does": "Asks a drawing set, or one sheet description, what is on its sheets, \
                         from the records, with nothing selected by coordinates. find lists the \
                         objects matching every option given, each with its ID and where it is \
                         written; attributes gives each matching insert's attributes as JSON \
                         from tag to value; blocks lists the blocks each sheet inserts; measure \
                         gives the distance between a point of two objects on one sheet, each \
                         named by ID or name, by default an insert's or a text's position, a \
                         line's from, a circle's centre; extents gives the box round what each \
                         sheet draws. Lengths are in the declared unit. --id, --name, --block \
                         and --tag match whole, in any case, and --value and --text whole, case \
                         and all; with --contains, each matches what holds it, in any case. A \
                         query that matches nothing is ok and says so; --sheet looks at one \
                         sheet only.",
            },
            "operations": {
                "usage": OPERATIONS_USAGE,
                "does": "Lists every operation a change set can hold: its name, its AutoCAD \
                         alias, what it does and its JSON Schema. `schema change-set` gives the \
                         schema of a whole change set.",
            },
            "change": {
                "usage": CHANGE_USAGE,
                "does": "Applies a change set, a JSON file such as {\"operations\": [{\"op\": \
                         \"move\", \"target\": {\"id\": \"DEMO-001/border\"}, \"by\": [0.1, \
                         0, 0]}]}, to a drawing set's own files at once, and builds again the \
                         DWGs of the sheets it touched. Objects are named by ID or name, never \
                         by coordinates. An edit to a template's object edits the template, so \
                         every sheet built from it follows; an object added to a sheet goes in \
                         its row. Each sheet takes its part fully or not at all, a template edit \
                         lands only if every sheet of its template does, and a change set that \
                         lands on some sheets and fails on others marks the set inconsistent \
                         until a later change set names them under fixes. --dry-run reports \
                         what it would do and writes nothing. The reply is not ok when a sheet \
                         failed.",
            },
            "journal": {
                "usage": JOURNAL_USAGE,
                "does": "Lists a drawing set's journal, kept in its folder whether or not it is \
                         a git repository: every change set, with its operations, the sheets it \
                         landed and failed on and the files it wrote; every failed attempt, \
                         which changed nothing; and every undo. It says which change sets can \
                         still be undone: the undo states of the 50 newest change sets are \
                         kept, and an older one's are pruned.",
            },
            "undo": {
                "usage": UNDO_USAGE,
                "does": "Undoes a change set, named by its entry's number in the journal, and \
                         every later change set with it, the newest first: every file they \
                         wrote, records, DWGs and the inconsistent mark, goes back byte for \
                         byte as it was before, and a file they made is taken away. It refuses, \
                         changing nothing and naming the files, when a file was changed since \
                         by hand, and when a change set's undo state was pruned. --dry-run \
                         reports what it would do and writes nothing.",
            },
        },
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
