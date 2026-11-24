# Stretch (`stretch` / `STRETCH` / `S`)

The `stretch` operation (AutoCAD aliases **`STRETCH`**, **`S`**) moves endpoints, vertices, and rigid objects lying inside a selection `box` by a displacement while leaving vertices outside the box where they are, so connected wires, enclosures, and devices follow together—on drawing sets and original `.dwg` revisions.

It is the one change-set operation that selects by area, so its reply lists **every object it touched** under `transformed`.

---

## Example

Make 30 mm of room on the right side of a loop sheet using a box defined from a coordinate and a snapped corner of `panel-box`:

```json
{
  "operations": [
    {
      "op": "STRETCH",
      "sheet": "LOOP-201",
      "box": [
        [80, 15],
        { "snap": { "id": "LOOP-201/panel-box", "snap": "endpoint", "which": "points[2]" } }
      ],
      "by": ["30 mm", 0, 0]
    }
  ]
}
```

Or specify the displacement with `from` and `to` (each a point or snapped point):

```json
{
  "operations": [
    {
      "op": "S",
      "box": [[80, 15], [105, 105]],
      "from": [90, 80, 0],
      "to": [120, 80, 0]
    }
  ]
}
```

---

## Keys

| Key | Required | Meaning |
|---|---|---|
| `op` | yes | `"stretch"`, `"STRETCH"`, or `"S"` (any case). |
| `sheet` / `template` | yes on sets (omit on original `.dwg`) | Sheet to stretch on (`sheet`) or template to stretch across every sheet built from it (`template`), not both. On an original `.dwg`, omit `sheet` to target that drawing. |
| `space` | no | Space to stretch in: `"model"` (default) or a layout name. |
| `targets` | no | Optional list of object targets (`{"id": ...}` or `{"name": ...}`) to restrict stretching to. When omitted, selects every unlocked object in `space` with vertices or anchors inside `box`. |
| `box` | yes | Two opposite corners `[corner1, corner2]` with positive width and height. Each corner is `[x, y]`, `[x, y, z]` in the declared unit (or carrying its own unit), or a snapped point `{"snap": {...}}` on a named object. |
| `by` | one motion form | Displacement `[dx, dy, dz]` in the declared unit or carrying its own. Give `by`, or `from` and `to`, not both. |
| `from`, `to` | one motion form | Base point and destination point (coordinates or snapped points); displacement is `to - from`. |

---

## Behaviour

- **Partially inside `box` (stretched):**
  - `line`: only the endpoint inside `box` moves; the other endpoint stays put.
  - `polyline`: every vertex inside `box` moves; vertices outside stay put (preserving bulges, widths, and `closed`).
  - `arc`: if only one endpoint lies inside `box` (and the center is outside), that endpoint moves while preserving the arc's included angle; if the center or both endpoints lie inside, the arc moves whole.
  - `spline`, `solid`, `wipeout`, `revision_cloud`: vertices/corners inside `box` move; vertices outside stay put.
- **Wholly inside `box` or rigid objects (moved whole):**
  - `insert`, `text`, `mtext`, `table`, `circle`, `ellipse`, `donut`: move whole when their insertion point or center lies inside `box`.
  - `leader`, `mleader`, `dimension`, `hatch`, and any curve whose vertices are all inside `box`: move whole (preserving template `{row: ...}` bindings and original DWG handles).
- **Locked layers & shared templates:**
  - In area-selection mode (`targets` omitted), objects on locked layers (such as a locked sheet border) are skipped cleanly and counted in the reply's `message`. If `targets` explicitly names a locked object, the operation fails with `locked`.
  - Stretching a single `sheet` refuses if an unlocked object inside `box` belongs to a template shared by multiple sheets (suggesting `template`, locking its layer, or using `targets`).
- **Reply & journal:** Every touched object ID is listed in `transformed` and recorded as `changed` in the journal; `undo` restores all touched files byte for byte.
