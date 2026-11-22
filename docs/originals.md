# Reading an original

An [original](../CONTEXT.md) is a DWG someone else drew, such as a plant's drawing. Its record is the DWG itself (ADR-0003), and AgentDrafter never writes to it (ADR-0004). Every job that revises one starts by reading it: an agent has to see what is on the drawing, find what it was asked to change, and know what it cannot see. `agentdrafter read` reads an original whole and says all three.

```sh
agentdrafter read P-101.dwg
agentdrafter read P-101.dwg --mirror review/P-101.mirror.yaml
agentdrafter read P-101.dwg --no-mirror
```

| Option | What it does |
|---|---|
| `<dwg>` | The original to read. It is only read. |
| `--mirror <path>` | Where to write the [text mirror](#the-text-mirror). Without it, beside the original, named after it with `.mirror.yaml` in place of `.dwg`: `P-101.dwg`'s is `P-101.mirror.yaml`. |
| `--no-mirror` | Read only, and write nothing. Not given with `--mirror`. |

Through the HTTP API, the MCP server and the Python library the command is `read`, and its input is `{"source", "mirror", "no_mirror"}` ([servers.md](servers.md)).

The reply, on a made-up drawing:

```json
{"source": "P-101.dwg", "sheet": "P-101",
 "message": "read P-101.dwg (AC1027): the DWG library could not decode 2 MADE_UP_DEVICE; 412 objects read; 3 the core has no model for, which no output draws: 2 MADE_UP_DEVICE, 1 MLINE; wrote its text mirror to P-101.mirror.yaml",
 "dwg_version": "AC1027", "units": "in",
 "decoded": false,
 "not_decoded": [{"kind": "MADE_UP_DEVICE", "count": 2, "handles": ["2A7", "2A9"]}],
 "skipped": 0, "recovered_errors": 0,
 "objects": {"insert": 40, "line": 310, "mtext": 12, "text": 50},
 "not_read": [{"kind": "MADE_UP_DEVICE", "count": 2, "handles": ["2A7", "2A9"]},
              {"kind": "MLINE", "count": 1, "handles": ["31C"]}],
 "tables_not_read": {"layers": 1},
 "mirror": "P-101.mirror.yaml"}
```

## The decode report

The DWG library, acadrust, reads every record of the file. A record of a kind it has a model for it decodes into that model; one it has none for, such as a custom object of an application AutoCAD loads, it keeps as the bytes it read, so that a revision saved in the version the original arrived in writes it back as it was (ADR-0004, ADR-0006), but nothing can draw, query or change it.

- **`decoded`** is true when the library decoded every record: none is under `not_decoded`, it skipped none and it recovered from no error.
- **`not_decoded`** names each record it read but could not decode, by `kind`, as the file names it, its `count`, and the `handles` AutoCAD lists each by, in hexadecimal, in the order of the file's handles. A graphical object and a non-graphical one, such as an application's dictionary entry, are both named.
- **`skipped`** counts the records the library could not read at all and left out, and **`recovered_errors`** the errors it recovered from. Either makes `decoded` false.

A drawing whose report says `decoded: false` will not be revised unless the reviewer allows it for that drawing, and the loss is then listed with the revision (ADR-0004, [#71](https://github.com/millZach/AgentDrafter/issues/71)). Reading never needs that: an original is read whatever it holds.

## What the core reads

The core reads what it has a model for, as `render`, `plot` and `display-list` read a DWG: the drawing's linetypes, layers, text styles, table, multileader and dimension styles, its block definitions, model space and each layout's paper space with its page setup and viewports, and every object on them it has a model for ([sheet-description.md](sheet-description.md)). `objects` counts what it read by type.

- **`not_read`** names, by kind, count and handle, each object on the sheet, model space's or a layout's paper space's, the core has no model for: what no output draws. A record the library did not decode is among them when it is on the sheet. An object of a kind the core reads can be here too, when it uses something the core has no model for yet, such as a true colour, or an insert of a block the core cannot read.
- **`tables_not_read`** counts the entries of each table, and the block definitions and layouts, the core could not read, such as `{"layers": 1}`; only those with any are named.

## IDs

Every object the core reads has an [ID](../CONTEXT.md) an agent can name it by. An object AgentDrafter built carries its own in the DWG, such as `DEMO-001/border`, and keeps it. Any other takes the drawing's name, its file's name without `.dwg`, joined to its handle: `P-101/handle:2F`. A handle is the DWG's own name for a record, which AutoCAD keeps for the object's life through every save, so the ID is the same each time the same file is read, and stays while the object does. The [text mirror](#the-text-mirror) gives each object's ID and handle; `display-list` gives each object of a DWG its ID the same way.

## The text mirror

The text mirror is a readable text rendering of what the core read, for reading and comparing. It is never the record: AgentDrafter never reads it back, and editing it changes nothing. It is written whole or not at all, and replaces a file only when that file is a mirror AgentDrafter wrote, as its first line shows; any other file at its path may be someone's own, and the read is refused with `not_an_output`. A mirror is never written over the original.

It is YAML, a key to a line. Each entry of a table, each object of a block and each object of the sheet is on a line of its own, in the order the file holds them, in braces, so that two mirrors compare line by line: the same file always gives the same mirror, byte for byte. A handle, and the fingerprint, are always in quotes, so that a handle such as `1E3` is never read as a number. A named entry gives its `name` first, then its other keys in order of their names. An object's keys are those a sheet description gives it, after its `id`, its `name` when it has one, its `handle` and, on a drawing with layouts, its `space`: `model` or its layout's name.

```yaml
# AgentDrafter's text mirror of P-101.dwg, for reading and comparing. The DWG is the record: this file is never read back, and editing it changes nothing.
mirror_of: P-101.dwg
sha256: "9f2c..."
dwg_version: AC1027
units: in
sheet: P-101
decoded: false
not_decoded: [{kind: MADE_UP_DEVICE, count: 2, handles: ["2A7", "2A9"]}]
skipped: 0
recovered_errors: 0
not_read: [{kind: MADE_UP_DEVICE, count: 2, handles: ["2A7", "2A9"]}, {kind: MLINE, count: 1, handles: ["31C"]}]
tables_not_read: {layers: 1}
ltscale: 1
linetypes: []
layers:
  - {name: "0", color: 7, linetype: Continuous}
  - {name: E-WIRE, color: 1, linetype: Continuous}
...
objects:
  - {id: P-101/handle:2F, handle: "2F", type: line, from: [0, 0, 0], layer: E-WIRE, to: [4, 0, 0]}
```

`sha256` is the original's fingerprint, as an output's [stamp](render.md) records its source's, so a mirror shows which bytes it was read from.

## Drawing an original

`render`, `plot` and `display-list` take an original as they take a sheet description, and draw what the core reads of it. Each reply counts the objects it could not draw under `objects_not_drawn` and names them, by kind, count and handle as `read` does, under `not_drawn`, which is left out when everything was drawn.

## Not yet

- `query` and `check` take a drawing set's folder or a sheet description, not yet an original.
- Drawing an original has not yet been measured against AutoCAD's own plot of it.
- Only DWG is read as an original; a DXF is read only by `import-blocks`.
