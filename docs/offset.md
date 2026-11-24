# Offset (`offset` / `OFFSET` / `O`)

The `offset` change-set operation (AutoCAD aliases **`OFFSET`** and **`O`**) draws a parallel curve at a given distance on a chosen side—or passing through a point—of an existing `line`, `polyline`, `arc`, `circle`, `ellipse`, or `spline` on a drawing set or original `.dwg` file.

```json
{
  "operations": [
    {
      "op": "OFFSET",
      "target": { "id": "PNL-001/cabinet" },
      "distance": "5 mm",
      "side": "inside",
      "id": "inner-wall"
    }
  ]
}
```

## Keys

| Key | Required | Default | Meaning |
|---|---|---|---|
| `op` | yes | — | `"offset"`, `"OFFSET"`, or `"O"` (any case). |
| `target` | yes | — | Existing curve object by `id` or `name` (`line`, `polyline`, `arc`, `circle`, `ellipse`, or `spline`). |
| `distance` | with `side` | — | Positive offset distance in the set's declared unit, or carrying its own unit such as `"5 mm"` or `"0.25 in"`. Give with `side`, or give `through` alone. |
| `side` | with `distance` | — | Which side to offset to: `"left"` or `"right"` (looking from start to end), `"inside"` or `"outside"` (for circles, arcs, closed polylines, ellipses, and closed splines), or a point `[x, y, z]` / snapped point `{"snap": {...}}` on the desired side. |
| `through` | instead of `distance`+`side` | — | Point `[x, y, z]` or snapped point `{"snap": {...}}` that the offset curve passes through (AutoCAD's Through mode). |
| `id` | no | auto | Explicit ID for the new offset object; when left out, assigned the first free `<kind>-1`, `<kind>-2`, … on the sheets reached. |
| `name` | no | none | Optional name for the new offset object. |
| `layer` | no | `target`'s layer | Layer for the new offset object. |
| `erase_source` | no | `false` | Delete `target` after creating the offset (AutoCAD's Erase option). Refused with `locked` if `target` sits on a locked layer. |

## Geometry rules

- **Line:** Shifted perpendicular in the XY plane by `distance`, keeping each endpoint's `z`.
- **Circle & Arc:** Concentric circle or arc with radius `r + distance` (outside) or `r - distance` (inside), preserving `center`, `start_angle`, and `end_angle`. Refused with `bad_value` when an inward offset leaves a non-positive radius.
- **Polyline (`OFFSETGAPTYPE 0`):** Flat 2D polylines (straight segments and arc bulges, open or closed) are offset segment-by-segment and adjacent offset segments are extended or trimmed to their projected intersections (line–line, line–arc, and arc–arc), matching AutoCAD's `OFFSETGAPTYPE = 0`. Segments that collapse on an inward offset are trimmed out iteratively; an inward offset that collapses the entire polyline or inverts a closed loop's winding is refused with `bad_value`.
- **Ellipse:** A circular ellipse (`ratio == 1`) offsets to a concentric ellipse; a general ellipse offsets to a smooth cubic fit-point `spline` through exact perpendicular offset points (matching AutoCAD's `OFFSET` of an `ELLIPSE`).
- **Spline:** Offsets to a cubic fit-point `spline` through perpendicular offset points along the B-spline (preserving closure for closed splines).

## Placement, templates, and original DWGs

- On a drawing set, the offset object is written in the same record (sheet row or shared template) and the same space (`model` or paper-space layout) as `target`, inheriting its `color`, `linetype`, `lineweight`, and `linetype_scale`.
- On an original `.dwg` file (`agentdrafter change <original.dwg> ...`), the offset object is added directly in `target`'s space while every untouched object keeps its raw DWG bytes and handle verbatim (ADR-0004).
