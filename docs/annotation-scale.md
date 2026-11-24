# Annotation Scale (`ANNOTATIVE`, `CANNOSCALE`, `OBJECTSCALE`)

Annotative objects declare their **paper size** (e.g., 2.5 mm text height or 1.0x paper-space hatch spacing) and the list of viewport scales they support. Through any paper-space viewport whose scale an object supports, AgentDrafter draws it at its exact paper size; through a viewport whose scale it does not support, it is hidden unless `anno_all_visible: true` (`ANNOALLVISIBLE`).

## Supported objects, styles and blocks

- **Objects**: `text`, `mtext`, `dimension`, `mleader`, `hatch`, and `insert`.
- **Styles**: `text_styles`, `dimension_styles`, and `mleader_styles` take `annotative: true`. Any object referencing an annotative style is annotative unless it explicitly sets `annotative: false`.
- **Blocks**: block definitions in a block library take `annotative: true`. Any `insert` of an annotative block is annotative unless it explicitly sets `annotative: false`.

## Sheet description and template keys

```yaml
units: mm
paper: ISO A3 landscape
layout: { name: Plan }
annotation_scales:
  '1:1': [1, 1]
  '1:50': [1, 50]
  '1:100': [1, 100]
annotation_scale: '1:50'      # active model-space scale (CANNOSCALE)
anno_all_visible: false       # ANNOALLVISIBLE (default: false in viewports, true in model space)

text_styles:
  NOTES: { font: txt.shx, annotative: true }

objects:
  - text:
      id: room-tag
      space: model
      contents: ROOM 101
      position: [0, 0, 0]
      height: 2.5             # paper size: draws 2.5 mm tall in BOTH 1:50 and 1:100 viewports
      annotative: true
      annotation_scales: ['1:50', '1:100']
  - viewport:
      id: vp-50
      space: paper
      center: [120, 150, 0]
      width: 180
      height: 160
      view_center: [0, 0, 0]
      scale: '1:50'
      annotation_scale: '1:50'
```

- `annotation_scales`: mapping from scale name to `[paper_units, drawing_units]` (`ACAD_SCALELIST`). Standard ratios such as `'1:50'`, `'2:1'`, `'1/4" = 1\'-0"'`, and `'1" = 20\''` are parsed automatically even if not listed explicitly under `annotation_scales`.
- `annotation_scale`: active model-space annotation scale (`CANNOSCALE`, default `'1:1'`).
- `anno_all_visible`: controls `ANNOALLVISIBLE`. When `false` (default inside paper-space viewports), annotative objects are drawn only in viewports whose scale matches one of their `annotation_scales`. When `true`, all annotative objects draw in every viewport at their exact paper size.
- Per-object `annotative: true | false` and `annotation_scales: [...]`: when an object is annotative and omits `annotation_scales`, it supports the sheet's active `annotation_scale`.
- Per-viewport `annotation_scale`: optional explicit annotation scale name for a viewport; when omitted, the viewport uses its geometric `scale`.

## Single drawing pipeline (ADR-0011)

PNG (`render`), PDF (`plot`), and the browser viewer (`display-list`) all run through `draw_sheet`:

- Inside a paper-space viewport, each model-space annotative object's paper-size fields are multiplied by `1 / viewport_annotation_scale` before the viewport's scale transform projects it onto paper, yielding the exact declared paper size.
- Patterned hatches scale their pattern spacing; solid fills stay unchanged.
- Dimensions scale their style's `scale` factor (text height, arrows, gaps, extension offsets) while measured geometry stays in true model units.
- Multileaders scale their text height, arrowhead size, landing length, gap, and block-content scale while leader tip points stay fixed in model space.
- Inserts of annotative blocks scale their placement scale while keeping insertion point and rotation fixed.

## DWG and DXF representation

Non-annotative sheets emit no extra annotation dictionaries, classes or XDATA, keeping existing DWG output byte-identical. When a sheet uses annotation scales:

- `ACAD_SCALELIST` in the root named-objects dictionary stores `SCALE` objects (`AcDbScale`).
- `AcDbVariableDictionary` stores `DICTIONARYVAR` entries for `CANNOSCALE` and (when set) `ANNOALLVISIBLE`, alongside header `CANNOSCALE` / `CANNOSCALEVALUE`, so both DWG 2018 and ASCII/binary DXF roundtrip cleanly.
- Styles (`TextStyle`, `DimStyle`, `MultiLeaderStyle`) and block records carry AutoCAD's annotative flags and `AcadAnnotative` (`AnnotativeData`) XDATA.
- Annotative model-space entities store their active-scale model geometry, carry `AcadAnnotative` XDATA and native flags (`MText.is_annotative`, `MultiLeader.enable_annotation_scale`), and attach `AcDbContextDataManager` -> `ACDB_ANNOTATIONSCALES` -> `*A1..*An` typed `ObjectContextData` leaves (`Text`, `MText`, `BlkRef`, `HatchScale`, `Dim`, `MLeader`).
- On `read` / `query` / `render` / `plot` / `display-list`, model-space annotative entities are unscaled back to their exact paper size using the entity's default context scale (or active `CANNOSCALE`).

## Querying annotation scales

`agentdrafter query annotation-scales <source> [--sheet <sheet>]` (JSON `{"kind": "annotation_scales"}`) reports each sheet's:

- `current`: active `CANNOSCALE`
- `factor`: active scale factor (`paper_units / drawing_units`)
- `anno_all_visible`: explicit `ANNOALLVISIBLE` override when present
- `scales`: declared/referenced scales (`name`, `paper_units`, `drawing_units`, `factor`)
- `objects`: every annotative object (`id`, `name`, `type`, `scales`)

`query layout` also includes each viewport's `annotation_scale` when set, and `display-list` includes `annotation_scale` and `annotation_scales` for annotative sheets.

## Change-set operations

Both operations work on drawing sets (templates, with full journal undo) and on original DWGs/DXFs (preserving untouched objects verbatim):

- **`set_annotation_scale`** (aliases: `CANNOSCALE`, `SCALELISTEDIT`): sets `scale` (`CANNOSCALE`), `anno_all_visible` (`ANNOALLVISIBLE`), adds/updates entries under `add_scales`, or removes entries under `remove_scales`.
- **`set_object_scales`** (aliases: `OBJECTSCALE`, `ANNOUPDATE`): targets one object (`target`) or several (`targets`, including `{"group": "<name>"}` group targets) and sets `annotative`, replaces supported scales (`scales`), or edits them (`add` / `remove`). Refuses object kinds that cannot be annotative.
