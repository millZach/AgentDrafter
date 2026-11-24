# Explode (`explode` / `EXPLODE` / `X`)

The `explode` operation (AutoCAD aliases **`EXPLODE`**, **`X`**) turns one or more compound objects into the simpler objects that draw them—on drawing sets and original `.dwg` revisions. The source object(s) are deleted (`deleted`), each newly created object receives its own unique ID, and every new ID is listed in drawing order under `created`.

---

## Examples

Explode a single block insert on a sheet:

```json
{
  "operations": [
    {
      "op": "EXPLODE",
      "target": { "id": "LOOP-201/valve-1" }
    }
  ]
}
```

Explode several objects with a custom ID prefix:

```json
{
  "operations": [
    {
      "op": "X",
      "targets": [
        { "id": "LOOP-201/panel-box" },
        { "id": "LOOP-201/dim-width" }
      ],
      "id_prefix": "exploded"
    }
  ]
}
```

---

## Keys

| Key | Required | Meaning |
|---|---|---|
| `op` | yes | `"explode"`, `"EXPLODE"`, or `"X"` (any case). |
| `target` | give `target` or `targets` | Single compound object to explode, by ID or name. |
| `targets` | give `target` or `targets` | Non-empty list of compound objects on the same sheet to explode in order. |
| `id_prefix` | no | Optional prefix for newly created objects' local IDs (`<prefix>-1`, `<prefix>-2`, ...). When omitted, drawing sets derive `<source-local-id>-1`, `<source-local-id>-2`, ... avoiding collisions, and original `.dwg` revisions allocate fresh DWG handle IDs without extra xdata. |

---

## What each object explodes into

- **`insert` (including dynamic blocks):**
  - Evaluates the block at the insert's dynamic `state` (or its evaluated `*U<n>` block on an original `.dwg`), places, scales, rotates, and mirrors each object in drawing order, resolves layer `0` to the insert's layer and `ByBlock` color/linetype/lineweight to the insert's own properties, and turns each visible non-empty attribute into single-line `text` carrying its value.
  - Nested inserts inside the block are placed as `insert` objects (single-level explode, matching AutoCAD).
  - On drawing sets, any layer, linetype, or text style supplied only by the block library and referenced by the exploded objects is copied into the template so exploding the last insert of a block leaves the sheet valid.
- **`polyline`:** Each straight segment becomes a `line`, each bulged arc segment becomes an `arc` (width is dropped, layer and properties are kept).
- **`donut`:** Two semicircular `arc` objects matching its centerline polyline.
- **`revision_cloud`:** The `arc` (and any straight `line`) segments of its cloud polyline.
- **`hatch`:**
  - Patterned hatch: clipped and dashed pattern `line` segments at the boundary's elevation (refused cleanly if the pattern would exceed 10,000 segments).
  - `SOLID` hatch: triangulated 2D `solid` triangles covering the filled region (bridging island holes).
- **`dimension`:** Extension and dimension `line`s, angular `arc`s, arrowhead `solid` / `donut` / `line`s, and `mtext` (or stacked-fraction `text` and bar `line`s).
- **`mleader`:** Arrowhead shapes, trimmed leader `line`s, landing `line`, and `mtext` or `insert` content.
- **`leader`:** Arrowhead shapes, trimmed leader `line`s (including AutoCAD hook), and optional `mtext` annotation (removing the linked DWG annotation entity on original `.dwg` revisions).
- **`table`:** Border `line`s and non-empty cell `mtext` objects.

## Refusals & undo

- Primitive objects (`line`, `arc`, `circle`, `ellipse`, `spline`, `text`, `mtext`, `solid`, `wipeout`, `viewport`) cannot be exploded and are refused with `bad_value`.
- Objects on locked layers (or inserts with attributes on locked layers in an original `.dwg`) are refused with `locked`.
- Every exploded source ID is recorded as `deleted` and every new ID as `added` in the journal; `undo` restores all files byte for byte.
