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
| `--mirror <path>` | Where to write the [text mirror](#the-text-mirror). Without it, beside the original, named after it with `.mirror.yaml` in place of `.dwg`: `P-101.dwg`'s is `P-101.mirror.yaml`. A path ending `.dwg` is refused. |
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

The DWG library, acadrust, reads every record of the file. A record of a kind it has a typed model for it decodes into that model; an undecoded custom, proxy or opaque record (such as an application's custom object, an `ACAD_PROXY_ENTITY`/`ACAD_PROXY_OBJECT`, an architectural AEC object, or a raw-only record) it keeps as the bytes it read, so that a revision saved in the version the original arrived in writes it back as it was (ADR-0004, ADR-0006), though nothing can draw, query or change it. By contrast, a damaged record skipped while reading past damage could not be read at all and is left out altogether.

- **`decoded`** is true when the library decoded every record whole: none is under `not_decoded`, it skipped none, it recovered from no error, the record stream completed, and diagnostics were not cut off at the library's cap.
- **`not_decoded`** names each record it could not decode whole, by `kind`, as the file names it, its `count`, and the `handles` AutoCAD lists each by, in hexadecimal, in the order of the file's handles. That covers opaque, proxy and custom records (both graphical and non-graphical), records cut short such as a `MESH` exceeding the reader's face budget, truncated or skipped sections, and damaged records skipped while reading past damage.
- **`skipped`** counts the records the library could not read at all and left out, and **`recovered_errors`** the errors it recovered from. Either makes `decoded` false.
- **`warnings`**, given only when greater than 0, counts non-fatal reader warnings (such as inconsistent source references or unknown enum values), without mixing them into `not_decoded`.
- **`diagnostics_truncated`**, given only when true, says that acadrust hit its internal diagnostic cap (999 entries) or notification limit and omitted further diagnostics, so `not_decoded` or `warnings` may be incomplete. It also makes `decoded` false.
- **`read_past_damage`**, given only when it happened, says why the library could not read the file whole. `read` then reads it again past the records it cannot read, as AutoCAD's RECOVER reads a damaged drawing but repairing nothing, and names each record it passed under `not_decoded`, by its DXF name when its fixed type code or class is known, `DWG_TYPE_` and its type number otherwise, and `UNREADABLE_RECORD` when its type could not be read. `decoded` is then false. A non-DWG file (or one whose header or record stream cannot be read at all) is always refused with `dwg_read` rather than succeeding with zero objects. Only `read` reads past damage: `render`, `plot` and `display-list` refuse such a file with `dwg_read`, as before.

A drawing whose report says `decoded: false` is refused when revising unless the reviewer allows it for that drawing with `--allow-loss`, and the loss is then listed in the reply and the revision report ([revisions.md](revisions.md), ADR-0004, [#71](https://github.com/millZach/AgentDrafter/issues/71)). Reading never needs that: an original is read whatever it holds, refused only when the library cannot read it even past its damage.

## What the core reads

The core reads what it has a model for, as `render`, `plot` and `display-list` read a DWG: the drawing's linetypes, layers, text styles, table, multileader and dimension styles, its block definitions, model space and each layout's paper space with its page setup and viewports, and every object on them it has a model for ([sheet-description.md](sheet-description.md)). `objects` counts what it read by type.

- **`not_read`** names, by kind, count and handle, each object on the sheet, model space's or a layout's paper space's, the core has no model for or that was skipped unread while reading past damage: what no output draws. A record the library did not decode is among them when it is on the sheet, and a skipped graphical or unidentified record is among them too. An object of a kind the core reads can be here too, when it uses something the core has no model for yet, such as a true colour, or an insert of a block the core cannot read.
- **`tables_not_read`** counts the entries of each table, and the block definitions and layouts, the core could not read, such as `{"layers": 1}`; only those with any are named.

## IDs

Every object the core reads, both on the sheet and inside block definitions, has a unique [ID](../CONTEXT.md) an agent can name it by. An object AgentDrafter built carries its own in the DWG, such as `DEMO-001/border`, and keeps it when it does not collide with another object's ID or handle ID. Any other takes the drawing's name, its on-disk file's name without `.dwg`, joined to its handle: `P-101/handle:2F`. A handle is the DWG's own name for a record, which AutoCAD keeps for the object's life through every save, so the ID is the same each time the same file is read (whatever letter case is used to pass its path), and stays while the object does. The [text mirror](#the-text-mirror) gives each object's ID and handle for both sheet objects and block-definition objects; `display-list`, `query` and `check` give the same IDs for sheet objects, so an agent can `read` an original and immediately `query` or `check` it by those IDs.

## The text mirror

The text mirror is a readable text rendering of what the core read, for reading and comparing. It is never the record: AgentDrafter never reads it back, and editing it changes nothing. It is written whole or not at all to a brand-new temporary file, replaces an existing file only when it is a regular non-symlink file that is a mirror AgentDrafter wrote, as its first line shows, and never follows or replaces a symbolic link or writes over a folder or the original; any other file at its path may be someone's own, and the read is refused with `not_an_output`.

> **Never commit a text mirror of a real drawing to a public repository.** A text mirror holds every layer, block, attribute, tag and text of the drawing, so for a plant's drawing it is [client material](../CONTEXT.md) (ADR-0008). `.gitignore` ignores `*.mirror.yaml` beside `*.dwg` and `*.dxf`.

It is YAML, a key to a line. Each entry of a table, each object of a block and each object of the sheet is on a line of its own, in the order the file holds them, in braces, so that two mirrors compare line by line: the same file always gives the same mirror, byte for byte. A handle, and the fingerprint, are always in quotes, so that a handle such as `1E3` is never read as a number. A named entry gives its `name` first, then its other keys in order of their names. An object's keys are those a sheet description gives it, after its `id`, its `name` when it has one, its `handle` and, on a sheet object of a drawing with layouts, its `space`: `model` or its layout's name.

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

## Drawing, querying and checking an original

`render`, `plot` and `display-list` take an original as they take a sheet description, and draw what the core reads of it. Each reply counts the objects it could not draw under `objects_not_drawn` and names them, by kind, count and handle as `read` does, under `not_drawn`, which is left out when everything was drawn.

`query` and `check` take an original (or a built DWG) as they take a sheet description or drawing set, strictly read-only, refusing damaged DWGs with `dwg_read` as `render`, `plot` and `display-list` do:

- Every `query` kind (`find`, `blocks`, `attributes`, `measure`, `extents`, `layers`, `snap`, `geometry`, `layout`) works on a DWG using the same sheet name and handle IDs as `read` and `display-list`. Provenance names the DWG `file` and object `handle` (never YAML `field`, `line` or `column`), `query layout` reports each layout's stored plotter, paper name, plot area, scale and plot style from the DWG, spatial queries (`--touching`, `snap` with `--with`/`--from`, `measure`) never mix different spaces, and duplicate carried `name`s fail with an ambiguity error listing every matching ID.
- `check` runs `paper_size`, `off_sheet`, `text_overflow`, `text_on_text` and `text_on_line` on the DWG. A layout with a 1:1 layout page setup (preserving exact stored dimensions, printer margins, plot rotation, plot origin, and `plot_paper_units` for both scale and paper tolerance/pen widths, without altering model-space units) or a built DWG whose build mark names its set's paper is held to that paper (refusing without modifying the set if that set holds an unfinished journal commit); a model-space original or unsupported layout page setup reports `paper_size` and `off_sheet` under `checks_skipped` and in `message` with the reason rather than silently claiming every check passed. When a layout's `plot_paper_units` differs from `CheckReport.units`, each finding on that layout carries its own `units`.
- Both report objects of the DWG the core has no model for under `objects_not_read` and `not_read` (left out when 0/empty) and note them in `message`. A unitless DWG reports `units: null` and checks model space as millimetres (`0.01` drawing units tolerance) and each layout in its own `plot_paper_units`.

## Revising an original

`agentdrafter change <original.dwg> <change-set.json | ->` revises an original DWG directly in memory without modifying the original, saving a new revision DWG beside it in its exact original DWG version with its `.changes.json` change list & diff report and revision PDF. See [revisions.md](revisions.md).

## Not yet

- Drawing an original has not yet been measured against AutoCAD's own plot of it.
- Only DWG is read as an original; a DXF is read only by `import-blocks`.
