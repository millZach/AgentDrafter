# AgentDrafter

An open-source CAD engine built for agents first. An agent creates, revises and checks technical drawings with no window open, and a person reviews the result. It reads and writes DWG without AutoCAD and runs on Windows, Linux and macOS.

**Status: walking skeleton.** There is nothing to install yet. From a checkout, `cargo run -p agentdrafter-cli -- build examples/one-line.yaml` builds a sheet description that holds one line into a DWG in the 2018 format and prints the result as JSON. `examples/geometry.yaml` adds polylines, arcs, circles, ellipses and splines on layers with linetypes, `examples/hatch.yaml` adds solid and patterned hatches, `examples/text.yaml` adds text and multiline text in TrueType and SHX text styles, `examples/blocks/` holds a block library and two sheets that insert its blocks with their attributes, `cargo run -p agentdrafter-cli -- build examples/demo-set` builds a drawing set, one sheet per row of its data, `cargo run -p agentdrafter-cli -- schema` prints what a sheet description may hold, `cargo run -p agentdrafter-cli -- measure-text "PUMP 101" --font romans.shx --height 2.5` measures how wide a line of text draws, `cargo run -p agentdrafter-cli -- render examples/geometry.yaml` draws a sheet to a PNG as AutoCAD plots it, and `cargo run -p agentdrafter-cli -- plot examples/demo-set` plots each sheet of the drawing set to a PDF at its exact size, monochrome, with text that can be searched.

The engine is written in Rust and reads and writes DWG through [acadrust](https://github.com/hakanaktt/acadrust). It will be reached from a command line, an MCP server, a Python library and a local HTTP API, and a reviewer will watch the work in a browser.

- [The plan](docs/plan.md): the first usable version, what comes after, and what is still open.
- [The sheet description](docs/sheet-description.md): the text a new sheet is built from.
- [The block library](docs/block-library.md): the blocks a drawing set shares.
- [The drawing set](docs/drawing-set.md): its settings, its rows and its templates, and how a set is built.
- [Rendering](docs/render.md): drawing a sheet to a PNG, and the stamp that says what it came from.
- [Plotting](docs/plot.md): drawing a sheet to a PDF at its sheet size, through a CTB plot style, with searchable text.
- [The glossary](CONTEXT.md): the words this project uses.
- [The decisions](docs/adr/): what was decided and why.

## Licence

[Apache-2.0](LICENSE).
