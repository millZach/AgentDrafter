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

use agentdrafter_core::{build, BuildRequest, Error, ErrorCode, Reply};
use serde::Serialize;
use serde_json::json;

const BUILD_USAGE: &str = "agentdrafter build <sheet-description> [--output <dwg>] [--overwrite]";

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
            "no command given; usage: {BUILD_USAGE}"
        ))));
    };
    match command.to_str() {
        Some("build") => match parse_build(args) {
            Ok(request) => reply(build(&request)),
            Err(error) => reply::<()>(Err(error)),
        },
        Some("help" | "--help" | "-h") => reply(Ok(help())),
        Some("--version" | "-V") => reply(Ok(json!({ "version": env!("CARGO_PKG_VERSION") }))),
        _ => reply::<()>(Err(Error::usage(format!(
            "unknown command `{}`; usage: {BUILD_USAGE}",
            command.to_string_lossy()
        )))),
    }
}

fn parse_build(mut args: impl Iterator<Item = OsString>) -> Result<BuildRequest, Error> {
    let mut description: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut overwrite = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
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
    Ok(BuildRequest {
        description,
        output,
        overwrite,
    })
}

fn help() -> serde_json::Value {
    json!({
        "name": "agentdrafter",
        "version": env!("CARGO_PKG_VERSION"),
        "commands": {
            "build": {
                "usage": BUILD_USAGE,
                "does": "Builds a sheet description into a DWG in the 2018 format (AC1032). \
                         The DWG goes beside the sheet description with a .dwg extension unless \
                         --output names another path. An existing file is never replaced \
                         without --overwrite.",
            },
        },
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
