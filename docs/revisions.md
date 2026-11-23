# Revising an original DWG

An [original](../CONTEXT.md) is a DWG someone else drew, such as a plant's drawing. Its record is the DWG itself, and AgentDrafter never writes to it (ADR-0004). `agentdrafter change` revises an original directly in memory—keeping every untouched object's raw DWG record, handle, extended data, table entry, dictionary and custom object intact—and saves a new [revision](../CONTEXT.md) beside it in the **exact DWG version the original arrived in** (ADR-0006), together with its [change list](../CONTEXT.md) and diff report (`.changes.json`) and a revision PDF.

```sh
agentdrafter change P-101.dwg edits.json
agentdrafter change P-101.dwg edits.json --output P-101.revA.dwg
agentdrafter change P-101.dwg edits.json --dry-run
agentdrafter change P-101.dwg edits.json --allow-loss --no-pdf
```

| Option | What it does |
|---|---|
| `<dwg>` | The original `.dwg` file to revise. Opened once, read-only, never overwritten. |
| `<change-set.json \| ->` | The change set as JSON, from a file or standard input. |
| `--output <revision.dwg>` | Where to write the revision DWG (must end `.dwg`, must not be the original or any existing file, symlink, or folder). Without it, `<stem>.rev1.dwg`, `<stem>.rev2.dwg`, ... beside the original. |
| `--allow-loss` | Allow revising an original the DWG library could not decode whole; accepted losses are recorded in both the reply and `.changes.json` (ADR-0004). |
| `--dry-run` | Validate, apply in memory, serialize + re-read diff check, and run `check`, writing zero files to disk. |
| `--plot-style <name>` | CTB plot style name (such as `monochrome.ctb`) for the revision PDF. |
| `--plot-styles <folder>` | Additional folder to search for CTB plot styles (repeatable). |
| `--no-pdf` | Skip plotting `<rev-stem>.pdf` beside the revision DWG. |
| `--fonts <folder>` / `--no-system-fonts` | Font folders for measuring text, checking, and plotting the revision PDF. |

Through the HTTP API, the MCP server and the Python library the command is `change` with `set` pointing at the `.dwg` file, taking the same options (`output`, `allow_loss`, `dry_run`, `plot_style`, `plot_styles`, `no_pdf`, `fonts`, `no_system_fonts`) from the one core registry (ADR-0007). Passing original-only options (`output`, `allow_loss`, `plot_style`, `plot_styles`, `no_pdf`) against a drawing-set folder is refused with `usage`.

## Decode-loss gate & collateral changes (ADR-0004)

Before applying any operation, AgentDrafter inspects the [decode report](originals.md#the-decode-report) from reading the original (falling back to read-past-damage when a damaged file has recoverable records):

- If `decoded` is `false` (any `not_decoded` custom/proxy/opaque record, `skipped > 0`, `recovered_errors > 0`, `read_past_damage`, or `diagnostics_truncated`) and `--allow-loss` is **not** set, the revision is refused with `incomplete_decode`. The error and reply name the undecoded kinds, counts, handles and any `read_past_damage` reason, and **zero files are written**.
- With `--allow-loss` (`allow_loss: true`), the revision proceeds. Undecoded custom records preserved as raw bytes by acadrust stay untouched in the saved revision (in the original's DWG version), and the full `decode_report` and `allowed_loss: true` are recorded in both the reply and `<rev-stem>.changes.json`.
- After serializing the revision DWG, AgentDrafter strictly re-reads those exact bytes and diffs entities (including owner handles, raw records, geometry, attributes, and extended data), non-graphical objects, dictionaries, symbol tables, requested additions/deletions, and decode health against the pre-edit snapshot. If any `unexpected_changes` occurred outside the requested operations and `--allow-loss` is **not** set, publishing is refused (writing **zero files**) and the unexpected changes are returned in the reply; with `--allow-loss`, the revision is published and the unexpected changes are recorded in both the reply and `<rev-stem>.changes.json`.

## Preserving the original DWG version (ADR-0006)

A revision is always written back in the exact DWG version the original arrived in (`AC1009`/R12, `AC1012`/R13, `AC1014`/R14, `AC1015`/R2000, `AC1018`/R2004, `AC1021`/R2007, `AC1024`/R2010, `AC1027`/R2013, `AC1032`/R2018), verified byte-for-byte across all 6 bytes of the DWG header before and after serialization. If an original arrived in a format the DWG library cannot write back without converting (such as `AC1006`/R10 or an `AD10xx` header prefix), revising is refused with `dwg_write` rather than silently converting the version.

## Client drawings stay outside the repository (ADR-0008)

> [!CAUTION]
> Never commit a revision DWG, its `.changes.json` change list (which carries file names, text, attribute values, and handles), or its revision PDF for a real plant drawing into a public repository. Keep plant originals and their revisions in a private folder outside the repository. `.gitignore` ignores `*.dwg`, `*.changes.json`, `*.rev[0-9]*.pdf`, and AgentDrafter's temporary files as a backstop, not a substitute for keeping client material outside the checkout.

## Operations on an original DWG

Operations use the same typed schema and AutoCAD aliases as [change sets](change-sets.md), including object snaps (`{"snap": {...}}`):

- **Sheet defaulting:** Operations that take `sheet` (`add_*`, `add_layer`, `change_layer`, `target` by `name`) may omit `sheet` (defaulting to the original's sheet name) or specify the original's sheet name (case-insensitive; any other sheet name or `template` is refused).
- **Target resolution:** Objects are named by ID (carried AgentDrafter ID or `{sheet}/handle:{HEX}`) or unambiguous carried `name` (ambiguous names fail with every matching ID).
- **Supported operations:**
  - `set_attribute` (`ATTEDIT`) — edits an insert's attribute by tag (and optional 1-based `occurrence`), preserving handle, style, position, alignment, extended data and sibling attributes.
  - `change` (`CHANGE`) — edits `layer`, properties (`color`, `linetype`, `lineweight`, `linetype_scale`), `name`, and geometry/text keys (`contents`/`text`, `position`, `height`, `rotation`, `width`, `width_factor`, `style`, `justify`, `from`/`to`, `center`, `radius`, `start_angle`/`end_angle`, `major_axis`, `ratio`, `points`, `closed`, `corners`, `scale`).
  - `move` (`MOVE`), `rotate` (`ROTATE`/`RO`), `scale` (`SCALE`/`SC`), `mirror` (`MIRROR`/`MI`, keeping `MIRRTEXT 0` text-reading-forwards behavior and `erase_source` default `false`).
  - `copy` (`COPY`) and `array` (`ARRAY`, `ARRAYRECT`, `ARRAYPOLAR`, `ARRAYPATH`) — clones objects with newly allocated handles into the same space.
  - `delete` (`ERASE`) — removes an object (and owned attributes/seqend for an insert) by handle.
  - Adding objects into model space or a named layout's paper space (`space`): `add_line`, `add_polyline`, `add_arc`, `add_circle`, `add_ellipse`, `add_spline`, `add_hatch`, `add_text`, `add_mtext`, `add_donut`, `add_solid`, `insert_block` (inserting any block defined in the original DWG).
  - Layer table edits without dropping existing layers: `add_layer`, `change_layer`.
- **Locked layers & all-or-nothing (ADR-0005):** Edits on a locked layer fail with `locked`. Any operation that fails—or any operation kind not yet supported on originals (such as `import_blocks` or `add_viewport`)—refuses the entire change set and writes zero files.

## Files written beside the original

Unless `--dry-run` is set, every revision writes up to three brand-new files atomically using non-clobbering renames (never overwriting the original, an existing file, or a symlink):

1. `<rev-stem>.dwg` — the revision DWG in the original's exact DWG version.
2. `<rev-stem>.changes.json` — the **change list** and revision report (`format: 1`): `original` and `revision` filenames, SHA-256 fingerprints and `dwg_version`, `decode_report` and `allowed_loss`, the recorded `change_set`, per-operation outcomes (`id`, `handle`, touched/added/deleted handles, `copies`, `snapped`), `unexpected_changes` (a structural self-check re-reading the written DWG bytes against the pre-edit snapshot to confirm nothing changed outside the requested operations), `checks` (checker findings on the revision), and `pdf` / `pdf_error`.
3. `<rev-stem>.pdf` — unless `--no-pdf`, a 1:1 plot of the revision DWG stamped with the revision DWG. If the revision cannot be plotted (for example, an empty drawing), the revision DWG and `.changes.json` are still written and `pdf_error` explains why the PDF was skipped.

## Not yet (Part 2)

- `journal`, `undo` and `accept` on original-DWG revisions (`#71` Part 2).
