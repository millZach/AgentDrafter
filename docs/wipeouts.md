# Wipeouts and draw order

A **wipeout** (`WIPEOUT`) is a closed polygonal region that masks whatever is drawn beneath it in the drawing's background colour — white on paper and in plotted PDFs and PNGs, and the model-space dark background on screen in the viewer — without trimming or deleting the geometry underneath. Wipeouts work on sheets, in templates, in rows, and inside blocks.

**Draw order** (`DRAWORDER` / `DR`, `TEXTTOFRONT`, and `draw_order` on any `add_*` operation) controls which objects draw first (behind) and which draw last (on top). Because wipeouts mask earlier objects and leave later objects visible, draw order and wipeouts work together.

## Sheet description

### `wipeout` object

A `wipeout` takes a closed polygon of at least three coplanar points, in order, with no consecutive duplicates and non-zero area, plus the standard object keys (`id`, `name`, `layer`, `color`, `linetype`, `lineweight`, `space`):

```yaml
objects:
  - line:
      id: bus-wire
      from: [2, 6, 0]
      to: [14, 6, 0]
  - wipeout:
      id: tag-mask
      points:
        - [6.6, 5.65, 0]
        - [9.4, 5.65, 0]
        - [9.4, 6.35, 0]
        - [6.6, 6.35, 0]
  - text:
      id: tag-label
      contents: "SIG-204"
      position: [8, 6, 0]
      height: 0.25
      justify: middle_center
```

Objects in a sheet description draw in the order they appear in `objects` (and in a drawing set, template objects draw before row objects). In the example above, `tag-mask` masks `bus-wire` where they overlap while `tag-label` draws cleanly on top.

A `wipeout` may also appear inside a block in a block library (`blocks.yaml`), using the same `points` and property keys (without `id`, `name`, or `space`), so inserting the block masks background geometry behind its symbol or tag.

### `wipeout_frame` setting

The sheet-level key `wipeout_frame` controls whether wipeout boundary outlines are drawn, matching AutoCAD's `WIPEOUTFRAME` system variable (`ACAD_WIPEOUT_VARS` / `WIPEOUTVARIABLES`):

| Value | AutoCAD `WIPEOUTFRAME` | Screen (viewer) | Paper / Plot (`render`, `plot`) |
| --- | --- | --- | --- |
| `off` *(default)* | `0` | Hidden | Hidden |
| `on` | `1` | Drawn | Drawn |
| `display` | `2` | Drawn | Hidden |

## Change-set operations

### `add_wipeout` (`WIPEOUT`)

Adds a `wipeout` to a sheet's row, a template, or an original DWG, taking `points` (each point may be coordinates or a snapped point), optional style keys, and optional `draw_order`:

```json
{
  "op": "WIPEOUT",
  "sheet": "DEMO-001",
  "points": [[6.6, 5.65, 0], [9.4, 5.65, 0], [9.4, 6.35, 0], [6.6, 6.35, 0]],
  "draw_order": { "under": { "id": "DEMO-001/tag-label" } }
}
```

### Optional `draw_order` on any `add_*` operation

Every `add_*` operation (including `add_wipeout`, `add_line`, `add_text`, `add_revision_cloud`, `insert_block`, etc.) accepts an optional `draw_order` key:

- `"front"` — default; drawn last (on top)
- `"back"` — drawn first (behind existing objects in the same list or space)
- `{"above": target}` — placed immediately after `target` (drawn on top of it)
- `{"under": target}` — placed immediately before `target` (drawn behind it)

### `draw_order` (`DRAWORDER`, `DR`)

Reorders one object (`target`) or several objects (`targets`) within the same record or DWG space, preserving their relative order:

```json
{
  "op": "DRAWORDER",
  "targets": [{ "id": "DEMO-001/tag-mask" }],
  "order": { "under": { "id": "DEMO-001/tag-label" } }
}
```

When reordering objects in a YAML record shifts an object that had no explicit `id`, AgentDrafter writes its previous positional ID into `id` so existing IDs never drift (reported in `ids_written`).

### `text_to_front` (`TEXTTOFRONT`)

Brings annotations to the front of draw order so they draw above geometry and wipeouts:

```json
{
  "op": "TEXTTOFRONT",
  "sheet": "DEMO-001",
  "which": "all"
}
```

`which` is one of `"text"` (`text` and `mtext`), `"dimensions"` (`dimension`), `"leaders"` (`mleader` and `leader`), or `"all"` (the default when omitted).

## DWG representation and revisions

- Each `wipeout` is written as a native AutoCAD `WIPEOUT` entity (with polygonal boundary normalized into the wipeout's unit rectangle and its closing vertex repeated, matching AutoCAD 2026) and `wipeout_frame` is stored in the root Named Object Dictionary under `ACAD_WIPEOUT_VARS` (`WIPEOUTVARIABLES`).
- On `agentdrafter read` and when revising an original DWG (`agentdrafter change <original.dwg> ...`), AgentDrafter honours each space's `ACAD_SORTENTS` (`SORTENTSTABLE`) extension dictionary when ordering objects for rendering, plotting, display lists, and queries.
- `draw_order`, `text_to_front`, and `add_*` with `draw_order` on an original DWG update only the space's `ACAD_SORTENTS` table, leaving untouched entities verbatim.

## Checker interaction

The `text_on_line` check suppresses false positives when a `wipeout` drawn strictly after the line and strictly before the text in the same space/viewport covers the intersection point.

## Example

See [examples/wipeout.yaml](../examples/wipeout.yaml).
