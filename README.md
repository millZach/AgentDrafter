# AgentDrafter

An open-source CAD engine built for agents first. An agent creates, revises and checks technical drawings with no window open, and a person reviews the result. It reads and writes DWG without AutoCAD and runs on Windows, Linux and macOS.

**Status: planned, not built.** There is nothing to install yet.

The engine will be written in Rust and read and write DWG through [acadrust](https://github.com/hakanaktt/acadrust). It is reached from a command line, an MCP server, a Python library and a local HTTP API, and a reviewer watches the work in a browser.

- [The plan](docs/plan.md): the first usable version, what comes after, and what is still open.
- [The glossary](CONTEXT.md): the words this project uses.
- [The decisions](docs/adr/): what was decided and why.

## Licence

[Apache-2.0](LICENSE).
