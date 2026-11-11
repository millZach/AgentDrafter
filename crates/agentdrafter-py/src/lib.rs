//! `agentdrafter._native`: the Python library's binding to the core.
//!
//! This is a thin layer (ADR-0007), as thin as the HTTP API and the MCP
//! server: it lists the core's registry of commands and runs one of them
//! on its input, both as the JSON the other interfaces send, which the
//! Python package (`python/agentdrafter/`) turns into a function per
//! command and into dicts. It also runs the `agentdrafter` command itself,
//! the command line's own code, for the package's command. Drawing logic
//! here is a bug.
//!
//! A command runs without holding Python's lock, so a long plot does not
//! stop the program's other threads.

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use agentdrafter_core::{commands, find_command, CommandInfo, Settings};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::Value;

/// Every command of the registry, in its order, as JSON: a list of each
/// command's `name`, what it `does` and its `input_schema`, as the HTTP
/// API lists them.
#[pyfunction(name = "commands")]
fn list_commands() -> String {
    let listed: Vec<CommandInfo> = commands().iter().map(|command| command.info()).collect();
    // A command's listing holds only strings, lists and string-keyed maps.
    serde_json::to_string(&listed).expect("a command's listing serializes to JSON")
}

/// Runs the command `name` on `input`, its input as a JSON object, with the
/// settings the environment gives now, as the command line takes them. The
/// reply is the JSON line the command line prints.
#[pyfunction]
fn run(py: Python<'_>, name: &str, input: &str) -> PyResult<String> {
    let command = find_command(name)
        .ok_or_else(|| PyValueError::new_err(format!("no command `{name}` in the registry")))?;
    let input: Value = serde_json::from_str(input)
        .map_err(|e| PyValueError::new_err(format!("the input is not JSON: {e}")))?;
    let reply = py.detach(|| command.run(input, &Settings::from_env()));
    // A reply holds only strings, numbers, lists and string-keyed maps.
    Ok(serde_json::to_string(&reply).expect("a reply serializes to JSON"))
}

/// Runs the `agentdrafter` command on `args`, the arguments after its own
/// name, as the binary runs it: it prints on standard output, or serves
/// until it is stopped. Gives the exit code.
#[pyfunction]
fn main(py: Python<'_>, args: Vec<OsString>) -> u8 {
    py.detach(|| {
        let code = agentdrafter_cli::main(args);
        // The binary's standard output is flushed as it exits; Python's
        // exit does not flush Rust's, so it is flushed here.
        let _ = std::io::stdout().flush();
        exit_number(code)
    })
}

/// The number an exit code stands for, which Python exits with.
fn exit_number(code: ExitCode) -> u8 {
    (0..=u8::MAX)
        .find(|&number| ExitCode::from(number) == code)
        .unwrap_or(1)
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(list_commands, module)?)?;
    module.add_function(wrap_pyfunction!(run, module)?)?;
    module.add_function(wrap_pyfunction!(main, module)?)?;
    Ok(())
}
