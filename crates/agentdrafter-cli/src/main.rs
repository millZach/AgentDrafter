//! `agentdrafter`: the command line over AgentDrafter's core.
//!
//! This is a thin layer (ADR-0007). It turns arguments into a core request,
//! calls the core, and prints the core's reply as one line of JSON on
//! standard output, on success and on failure. Drawing logic here is a bug.
//!
//! Exit codes: 0 when the reply is `ok`, 1 when the core reports an error,
//! 2 when the arguments are wrong.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use agentdrafter_core::{
    block_library_schema, build_with, measure_text_with, sheet_description_schema, BuildRequest,
    Error, ErrorCode, MeasureTextRequest, Reply, Settings, FONTS_VARIABLE,
};
use serde::Serialize;
use serde_json::json;

const BUILD_USAGE: &str =
    "agentdrafter build <sheet-description> [--output <dwg>] [--overwrite] [--fonts <folder>]...";
const SCHEMA_USAGE: &str = "agentdrafter schema [sheet-description | block-library]";
const MEASURE_USAGE: &str = "agentdrafter measure-text <contents> --font <file> --height <height> [--width-factor <factor>] [--fonts <folder>]...";

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
            "no command given; usage: {BUILD_USAGE} | {SCHEMA_USAGE} | {MEASURE_USAGE}"
        ))));
    };
    match command.to_str() {
        Some("build") => match parse_build(args) {
            Ok((request, settings)) => reply(build_with(&request, &settings)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("measure-text") => match parse_measure(args) {
            Ok((request, settings)) => reply(measure_text_with(&request, &settings)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("schema") => {
            let which = args.next();
            match (which.as_ref().map(|a| a.to_str()), args.next()) {
                (None | Some(Some("sheet-description")), None) => {
                    reply(Ok(sheet_description_schema()))
                }
                (Some(Some("block-library")), None) => reply(Ok(block_library_schema())),
                (_, Some(extra)) => reply::<()>(Err(Error::usage(format!(
                    "schema takes at most one argument, found `{}` as well; usage: {SCHEMA_USAGE}",
                    extra.to_string_lossy()
                )))),
                (Some(_), None) => reply::<()>(Err(Error::usage(format!(
                    "schema prints the schema of a sheet description or of a block library, not `{}`; usage: {SCHEMA_USAGE}",
                    which.unwrap_or_default().to_string_lossy()
                )))),
            }
        }
        Some("help" | "--help" | "-h") => reply(Ok(help())),
        Some("--version" | "-V") => reply(Ok(json!({ "version": env!("CARGO_PKG_VERSION") }))),
        _ => reply::<()>(Err(Error::usage(format!(
            "unknown command `{}`; usage: {BUILD_USAGE} | {SCHEMA_USAGE} | {MEASURE_USAGE}",
            command.to_string_lossy()
        )))),
    }
}

/// The settings for one command: the font folders `--fonts` names, then
/// those the environment names.
fn settings(fonts: Vec<PathBuf>) -> Settings {
    let mut settings = Settings::from_env();
    settings.font_folders.splice(0..0, fonts);
    settings
}

fn parse_build(
    mut args: impl Iterator<Item = OsString>,
) -> Result<(BuildRequest, Settings), Error> {
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
                        "build takes one sheet description; usage: {BUILD_USAGE}"
                    )));
                }
            }
        }
    }
    let description = description.ok_or_else(|| {
        Error::usage(format!(
            "build needs a sheet description; usage: {BUILD_USAGE}"
        ))
    })?;
    Ok((
        BuildRequest {
            description,
            output,
            overwrite,
        },
        settings(fonts),
    ))
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
            },
            "schema": {
                "usage": SCHEMA_USAGE,
                "does": "Prints the sheet description's schema, or with block-library the block \
                         library's: a JSON Schema (draft 2020-12) of every key it may hold, with \
                         what each means.",
            },
            "measure-text": {
                "usage": MEASURE_USAGE,
                "does": "Measures how wide one line of text draws in a font at a height, as \
                         AutoCAD measures a text object: from the left edge of what it draws to \
                         the right edge. Reports the font used, and the look-alike when the font \
                         is not in the font folders.",
            },
        },
        "fonts": format!(
            "Fonts are read from the folders --fonts names, then from those the {FONTS_VARIABLE} \
             environment variable lists (separated as the system separates paths). There is no \
             default folder. A font in none of them is replaced by a look-alike, and the reply \
             says which."
        ),
        "output": "One line of JSON on standard output: {\"ok\": true, \"result\": ...} or \
                   {\"ok\": false, \"error\": {\"code\", \"message\", \"field\", \"line\", \
                   \"column\", \"file\"}}.",
        "exit_codes": { "0": "ok", "1": "the core reported an error", "2": "wrong arguments" },
    })
}

fn reply<T: Serialize>(outcome: Result<T, Error>) -> (String, ExitCode) {
    let code = match &outcome {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) if error.code == ErrorCode::Usage => ExitCode::from(2),
        Err(_) => ExitCode::FAILURE,
    };
    // A reply holds only strings, numbers, lists and string-keyed maps, which
    // always serialize.
    let line = serde_json::to_string(&Reply::from(outcome)).expect("a reply serializes to JSON");
    (line, code)
}
