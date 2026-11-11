# AgentDrafter

An open-source CAD engine built for agents first. An agent creates, revises and checks technical drawings with no window open, and a person reviews the result. It reads and writes DWG without AutoCAD and runs on Windows, Linux and macOS.

**Status: walking skeleton.** There is nothing to install yet. From a checkout, `cargo run -p agentdrafter-cli -- build examples/one-line.yaml` builds a sheet description that holds one line into a DWG in the 2018 format and prints the result as JSON. `examples/geometry.yaml` adds polylines, arcs, circles, ellipses and splines on layers with linetypes, `examples/hatch.yaml` adds solid and patterned hatches, `examples/text.yaml` adds text and multiline text in TrueType and SHX text styles, `examples/blocks/` holds a block library and two sheets that insert its blocks with their attributes, `cargo run -p agentdrafter-cli -- build examples/demo-set` builds a drawing set, one sheet per row of its data, `cargo run -p agentdrafter-cli -- schema` prints what a sheet description may hold, `cargo run -p agentdrafter-cli -- measure-text "PUMP 101" --font romans.shx --height 2.5` measures how wide a line of text draws, `cargo run -p agentdrafter-cli -- render examples/geometry.yaml` draws a sheet to a PNG as AutoCAD plots it, `cargo run -p agentdrafter-cli -- plot examples/demo-set` plots each sheet of the drawing set to a PDF at its exact size, monochrome, with text that can be searched, `cargo run -p agentdrafter-cli -- query find examples/demo-set --name title-block` asks a set what is on its sheets, and `cargo run -p agentdrafter-cli -- change <set> <change-set.json>` changes a set with typed operations, which `cargo run -p agentdrafter-cli -- operations` lists, and `cargo run -p agentdrafter-cli -- undo <set> <change-set>` undoes a change set from the journal in the set's folder, and `cargo run -p agentdrafter-cli -- check examples/demo-set` checks each sheet for its paper size, objects off the sheet, text overflowing its cell and text on other text.

## The demo plant

`examples/demo-plant/` is a made-up plant's loop sheets as a drawing set: eight sheets, each a loop from its field device through a junction box's terminals to a controller I/O channel, with tags, cable and wire numbers and a title block. Every name and number in it is invented. From a checkout:

```
cargo run -p agentdrafter-cli -- build examples/demo-plant
cargo run -p agentdrafter-cli -- plot examples/demo-plant
cargo run -p agentdrafter-cli -- check examples/demo-plant
```

`build` writes each sheet's DWG into `examples/demo-plant/dwg/`, `plot` its PDF beside it, and `check` reports every check a sheet fails; add `--overwrite` to build or plot again. CI does the same on Windows, Linux and macOS, fails on any failed check, and compares the three systems' DWGs and PDFs byte for byte.

The engine is written in Rust and reads and writes DWG through [acadrust](https://github.com/hakanaktt/acadrust). It will be reached from a command line, an MCP server, a Python library and a local HTTP API, and a reviewer will watch the work in a browser.

- [The plan](docs/plan.md): the first usable version, what comes after, and what is still open.
- [The sheet description](docs/sheet-description.md): the text a new sheet is built from.
- [The block library](docs/block-library.md): the blocks a drawing set shares.
- [The drawing set](docs/drawing-set.md): its settings, its rows and its templates, and how a set is built.
- [Rendering](docs/render.md): drawing a sheet to a PNG, and the stamp that says what it came from.
- [Plotting](docs/plot.md): drawing a sheet to a PDF at its sheet size, through a CTB plot style, with searchable text.
- [Queries](docs/query.md): finding objects by ID, name, block, attribute or text across a drawing set, listing blocks, reading attributes and measuring.
- [Change sets](docs/change-sets.md): adding, changing, moving and deleting objects and setting attribute values across a drawing set, each sheet fully or not at all.
- [The journal and undo](docs/journal.md): every change set and failed attempt recorded in the set's folder, and undone from it, byte for byte, in a later session.
- [The checker](docs/check.md): the default checks each sheet is held to after every change.
- [The glossary](CONTEXT.md): the words this project uses.
- [The decisions](docs/adr/): what was decided and why.

## Licence

[Apache-2.0](LICENSE).
