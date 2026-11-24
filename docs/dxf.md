# DXF read and write

AgentDrafter reads a DXF (ASCII or binary) everywhere it reads a DWG, and builds a sheet description or a drawing set to DXF (ASCII by default, or binary DXF) with the same build mark, object IDs and 2018 (`AC1032`) drawing structure as a DWG (AutoCAD's `DXFIN` and `DXFOUT`).

## Building a DXF

### One sheet description

`agentdrafter build` writes a DWG by default (`format: dwg`), an ASCII DXF when `format` is `dxf` or `--output` ends `.dxf`, and a binary DXF when `format` is `dxf_binary`:

```sh
# Auto-detected ASCII DXF from the .dxf output extension
agentdrafter build examples/geometry.yaml --output geometry.dxf

# Explicit ASCII DXF beside the sheet description (examples/geometry.dxf)
agentdrafter build examples/geometry.yaml --format dxf

# Binary DXF (starts with "AutoCAD Binary DXF")
agentdrafter build examples/geometry.yaml --format dxf_binary --output geometry-bin.dxf
```

When both `format` and `output` are given, their extensions must agree: asking for `format: dwg` with a `.dxf` output path, or `format: dxf` / `dxf_binary` with a `.dwg` output path, is refused with a `usage` error.

The build report keeps `dwg` set to the written path for compatibility, and when writing a DXF adds:

- `dxf`: the `.dxf` path written
- `format`: `"dxf"` (ASCII) or `"dxf_binary"` (binary)

### A drawing set

A drawing set's `set.yaml` may declare `format`:

```yaml
units: mm
output: dxf
rows: rows.yaml
format: dxf        # dwg (default), dxf (ASCII DXF), or dxf_binary (binary DXF)
```

- `agentdrafter build <set>` writes `<output>/<sheet>.dxf` for each row (overridable per run with `--format dwg | dxf | dxf_binary`).
- `agentdrafter change <set> <change-set.json>` rebuilds touched sheets in the set's `format`.
- `agentdrafter accept <set>` issues the set's built `.dxf` files alongside its records.
- Leftover `.dxf` outputs built by the set for removed rows are reported under `leftovers` with `kind: "dxf"` and moved into `superseded/` when `clean` is enabled; unmarked DXFs (originals) are never listed or moved.

### Build mark and object IDs

Every DXF AgentDrafter builds carries:

- **The same build mark** (`AgentDrafter`) in the DXF `HEADER` section's `$CUSTOMPROPERTYTAG` / `$CUSTOMPROPERTY` custom drawing properties (`build`, `build; set=...`, or `build; rows=...; set=...`). `--overwrite` replaces an existing `.dxf` only when it carries AgentDrafter's build mark; an unmarked original `.dxf` is never overwritten.
- **The same object IDs and names** in each entity's `AgentDrafter` extended data (XDATA), so `read`, `query`, `check`, `display-list` and `change` see the exact same IDs as on a DWG.

## Reading a DXF

Every command that accepts a `.dwg` also accepts an ASCII or binary `.dxf`:

- `agentdrafter read <drawing.dxf>` reads the DXF whole, reports `dwg_version`, `format` (`"dxf"` or `"dxf_binary"`), `decoded`, `not_decoded`, `not_read`, `tables_not_read`, `sha256` and `units`, assigns carried or handle IDs (`<stem>/handle:<HEX>`), and writes `<stem>.mirror.yaml` (never overwriting `.dwg` or `.dxf`).
- `agentdrafter render <drawing.dxf>` and `agentdrafter plot <drawing.dxf>` draw through the one drawing pipeline (ADR-0011), resolve declared paper and issued revision status from a built DXF's build mark, and stamp the output with source `kind: "dxf"`. `agentdrafter stale` checks that stamp.
- `agentdrafter display-list <drawing.dxf>` produces the display list for the viewer and drafting app.
- `agentdrafter query <kind> <drawing.dxf>` and `agentdrafter check <drawing.dxf>` inspect the DXF strictly read-only.
- `agentdrafter import-blocks <drawing.dxf> --library <blocks.yaml>` imports its blocks into a block library.

## Revising an original DXF

`agentdrafter change`, `journal`, `undo` and `accept` work on an original `.dxf` exactly as on an original `.dwg` ([`docs/revisions.md`](revisions.md)):

- The original `.dxf` is never modified.
- A revision is written beside it as `<stem>.rev1.dxf` (or `--output <path.dxf>`), preserving the original's DXF format (ASCII stays ASCII, binary stays binary) and DXF version, alongside `<stem>.rev1.changes.json` and `<stem>.rev1.pdf`.
- Revisions are tracked in the per-original write-ahead journal under `.agentdrafter/originals/`, with crash recovery, stack `undo`, `accept`, and issued stamps on revision outputs.
