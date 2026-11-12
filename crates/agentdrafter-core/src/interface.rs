//! The interfaces a command reaches the core through.
//!
//! Every interface hands the core a command and its input, and says which
//! interface it is, so that what a command records can say how it came:
//! the journal records the interface each accept came through, so the
//! reviewer can tell an agent's accept from their own. A command's input
//! never says it, so no caller can choose it.

use serde::{Deserialize, Serialize};

/// One of the ways in: the command line, the HTTP API, the MCP server and
/// the Python library, and the viewer, whose page reaches the core through
/// the HTTP API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interface {
    /// The `agentdrafter` command, the Python package's among it.
    Cli,
    /// The HTTP API, `agentdrafter serve`, from anything but the viewer's
    /// page.
    Http,
    /// The MCP server, `agentdrafter mcp`.
    Mcp,
    /// The Python library's functions.
    Python,
    /// The viewer's page, through the HTTP API.
    Viewer,
}

impl Interface {
    /// What the interface is called in a message.
    pub fn words(self) -> &'static str {
        match self {
            Interface::Cli => "the command line",
            Interface::Http => "the HTTP API",
            Interface::Mcp => "the MCP server",
            Interface::Python => "the Python library",
            Interface::Viewer => "the viewer",
        }
    }
}
