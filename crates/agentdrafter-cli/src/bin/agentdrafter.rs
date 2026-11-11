//! The `agentdrafter` binary: the command line, whose code is the library's
//! (`src/lib.rs`).

use std::process::ExitCode;

fn main() -> ExitCode {
    agentdrafter_cli::main(std::env::args_os().skip(1).collect())
}
