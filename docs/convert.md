# Converting DWG versions

AgentDrafter builds new sheets in the **2018 DWG format (`AC1032`)** by default, and revising an existing drawing always preserves the exact DWG version it arrived in ([ADR-0006](adr/0006-new-sheets-are-2018-and-a-revision-keeps-its-version.md)). Converting a drawing to another DWG version is always an explicit step — never a side effect of a revision — and **an original is never overwritten** ([ADR-0004](adr/0004-unchanged-content-survives-revision.md)).

## Supported DWG versions

| Code | AutoCAD release | Notes |
|---|---|---|
| `AC1032` | `2018` | Default for new sheets (AutoCAD 2018–2026) |
| `AC1027` | `2013` | AutoCAD 2013–2017 |
| `AC1024` | `2010` | AutoCAD 2010–2012 (first version with native `PDFUNDERLAY`) |
| `AC1021` | `2007` | AutoCAD 2007–2009 (UTF-16 strings, `MULTILEADER`, `ACAD_SCALELIST`) |
| `AC1018` | `2004` | AutoCAD 2004–2006 (TrueColor, `ACAD_TABLE`, `FIELD`, `AcDb:SummaryInfo`) |
| `AC1015` | `2000` | AutoCAD 2000–2002 (lineweights, multiple layouts, STB plot styles, `WIPEOUT`) |
| `AC1014` | `R14` | AutoCAD R14 (`LWPOLYLINE`, `HATCH`, `IMAGE`) |
| `AC1012` | `R13` | AutoCAD R13 (`MTEXT`, `SPLINE`, `ELLIPSE`, `MLINE`, `TOLERANCE`, `XLINE`, `RAY`, `LEADER`) |
| `AC1009` | `R12` | AutoCAD R12 (classic 2D/3D primitives only) |

Commands accept either the 6-character code (`AC1024`) or the release name (`2010`, `R14`, etc.).

## `agentdrafter convert`

Converts a single `.dwg` or `.dxf` drawing, a folder of drawings, or a drawing set's built drawings to a target DWG version (`SAVEAS` to a version):

```sh
# Convert one drawing down to 2010 (writes panel.2010.dwg beside panel.dwg)
agentdrafter convert panel.dwg --version 2010

# Convert an older 2000 drawing up to 2018 at an explicit path
agentdrafter convert old-loop.dwg --version 2018 --output old-loop.2018.dwg

# Convert a DWG to a 2010 ASCII DXF
agentdrafter convert panel.dwg --version 2010 --output panel.2010.dxf

# Preview losses without writing any file
agentdrafter convert panel.dwg --version 2000 --dry-run

# Convert every built drawing in a drawing set to 2010 (writes into dwg/converted-2010/)
agentdrafter convert examples/demo-set --version 2010
```

Options (and matching JSON fields on the HTTP API, MCP server and Python library `agentdrafter.convert(...)`):

- `source`: path to a `.dwg` or `.dxf` file, a folder of drawings, or a drawing set folder (`set.yaml`).
- `version` (`--version` / `--to`): target DWG version (`AC1009`..`AC1032` or `R12`..`2018`).
- `output` (`--output` / `-o`): output file path (for a single drawing) or output folder (for a folder or drawing set). Must not resolve to `source` itself; an original is never overwritten even when `overwrite` is set.
- `format` (`--format`): `dwg`, `dxf` (ASCII DXF), or `dxf_binary`. Inferred from `output`'s extension (`.dxf` / `.dwg`) or kept from the source when omitted.
- `overwrite` (`--overwrite`): replace an existing non-source file at `output`.
- `allow_loss` (`--allow-loss`): convert even when the DWG library could not decode every record of the source drawing (`incomplete_decode`). Without it, incomplete decodes are refused.
- `dry_run` (`--dry-run`): run the full conversion and post-write re-read verification in memory without writing any file.

## Loss reporting

Every converted drawing is analyzed before writing and re-read after writing. The reply reports:

- `source_version` and `target_version`: exact 6-character version codes.
- `total_objects` and `unchanged_objects`: how many graphical objects (and owned attributes) survived with matching geometry and properties.
- `lossless`: `true` only when the source decoded cleanly, the converted output decoded cleanly, and `losses` is empty.
- `losses`: a list of concrete `{kind, handle, change, detail}` entries, where `change` is:
  - `downgraded`: a feature introduced after the target version (such as `PDFUNDERLAY` below 2010, `MULTILEADER` or annotative scales or non-ASCII text below 2007, `ACAD_TABLE` / `FIELD` / 24-bit TrueColor / transparency / custom drawing properties below 2004, lineweights / multiple layouts / STB plot styles / `WIPEOUT` below 2000, `LWPOLYLINE` / `HATCH` / `IMAGE` below R14, or `MTEXT` / `SPLINE` / `ELLIPSE` / `MLINE` / `TOLERANCE` / `XLINE` / `RAY` below R13).
  - `dropped`: an object, layer, or block absent when the converted drawing was re-read.
  - `changed`: an object or layer whose properties or geometry differed when re-read.
  - `decode`: any decode regression on the converted output.
- `decode` and `output_decode`: full decode reports for the source and converted drawings.

For a folder or drawing set, `convert` reports per-drawing outcomes under `drawings` alongside aggregate `converted`, `failed` and `total_losses` counts.

## Per-job `dwg_version` in `set.yaml` and `build`

A plant job whose deliverables must be issued in an older DWG version can declare `dwg_version` in `set.yaml`:

```yaml
units: mm
output: dwg
rows: rows.yaml
dwg_version: 2010   # or "AC1024", "2004", "AC1015", etc.
```

- `agentdrafter build` honors `set.yaml`'s `dwg_version`, and `--dwg-version <version>` overrides it for a single sheet or a single run of a drawing set.
- `agentdrafter change` on a drawing set continues rebuilding touched sheets in the set's configured `dwg_version`.
- `BuildReport` records the written `dwg_version` and reports any `conversion_losses` when building into a version older than `AC1032` (2018).
- Pre-2004 DWG builds (`AC1015`, `AC1014`, `AC1012`) carry AgentDrafter's build mark in a Named Object Dictionary `XRecord` (`AGENTDRAFTER_BUILD`) because `AcDb:SummaryInfo` was introduced in AutoCAD 2004 (`AC1018`), so `build --overwrite` and `change` continue to recognize AgentDrafter's own builds while refusing to overwrite originals.
