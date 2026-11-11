# AgentDrafter

An open-source CAD engine built for agents first. An agent creates, revises and checks technical drawings with no window open, and a person reviews the result. It reads and writes DWG without AutoCAD and runs on Windows, Linux and macOS.

**Status: walking skeleton.** There is nothing to install yet. From a checkout, `cargo run -p agentdrafter-cli -- build examples/one-line.yaml` builds a sheet description that holds one line into a DWG in the 2018 format and prints the result as JSON. `examples/geometry.yaml` adds polylines, arcs, circles, ellipses and splines on layers with linetypes, `examples/hatch.yaml` adds solid and patterned hatches, `examples/text.yaml` adds text and multiline text in TrueType and SHX text styles, `cargo run -p agentdrafter-cli -- schema` prints what a sheet description may hold, `cargo run -p agentdrafter-cli -- measure-text "PUMP 101" --font romans.shx --height 2.5` measures how wide a line of text draws, and `cargo run -p agentdrafter-cli -- render examples/geometry.yaml` draws a sheet to a PNG as AutoCAD plots it.

The engine is written in Rust and reads and writes DWG through [acadrust](https://github.com/hakanaktt/acadrust). It will be reached from a command line, an MCP server, a Python library and a local HTTP API, and a reviewer will watch the work in a browser.

- [The plan](docs/plan.md): the first usable version, what comes after, and what is still open.
- [The sheet description](docs/sheet-description.md): the text a new sheet is built from.
- [Rendering](docs/render.md): drawing a sheet to a PNG, and the stamp that says what it came from.
- [The glossary](CONTEXT.md): the words this project uses.
- [The decisions](docs/adr/): what was decided and why.

## Licence

[Apache-2.0](LICENSE).
