# Revision clouds

A **revision cloud** marks the revised region of a drawing, as AutoCAD's `REVCLOUD` does: a closed lightweight polyline of outward-bowing arcs around a rectangle, a closed polygon, or the drawn extents of one or more existing objects, carrying AutoCAD's `RevcloudProps` extended data so AutoCAD's `REVCLOUD` recognizes and can modify it.

`examples/revision-cloud.yaml` holds a made-up sheet with rectangular, polygonal, normal, calligraphy and reversed revision clouds.

## In a sheet description

A `revision_cloud` object sits in `objects` (of a sheet, a template, a row or a block) and takes either `rectangle` or `polygon` (never both), together with `arc_length` and optional `style`, `reverse` and standard object properties (`layer`, `color`, `linetype`, `lineweight`, `linetype_scale`).

```yaml
- revision_cloud:
    id: rev-1
    rectangle:
      from: [1.0, 4.7, 0]
      to: [4.8, 6.5, 0]
    arc_length: 0.35
    style: calligraphy
    layer: REVCLOUD
```

Or around a closed polygon of three or more coplanar points:

```yaml
- revision_cloud:
    id: rev-2
    polygon:
      - [1.0, 1.3, 0]
      - [4.4, 1.3, 0]
      - [4.4, 2.5, 0]
      - [2.9, 2.5, 0]
      - [2.9, 3.4, 0]
      - [1.0, 3.4, 0]
    arc_length: 0.35
    style: normal
    reverse: false
    layer: REVCLOUD
```

| Key | Required | What it is |
|---|---|---|
| `rectangle` | one of `rectangle` or `polygon` | Opposite corners `{ from, to }` sharing one `z`, with width and height more than 0. |
| `polygon` | one of `rectangle` or `polygon` | At least 3 coplanar points forming a closed polygon with non-zero enclosed area and no two consecutive points equal. |
| `arc_length` | yes | Approximate chord length of each cloud arc, in the declared unit (or with a unit suffix such as `10mm` or `0.5in`), more than 0. |
| `style` | no | `normal` (uniform line width, the default) or `calligraphy` (each arc tapers from `0` at its start to `0.1 × arc_length` at its end, as AutoCAD's `REVCLOUD` Calligraphy style draws it). |
| `reverse` | no | `false` (the default, arcs bow outward) or `true` (arcs bow inward, as AutoCAD's `REVCLOUD` reverse option does). |
| `layer`, `color`, `linetype`, `lineweight`, `linetype_scale` | no | Standard object properties; see [Sheet description](sheet-description.md). |

Each straight edge of the boundary is divided into `max(1, round(edge_length / arc_length))` equal chords (capped at 10,000 total arcs). The boundary is normalized to anticlockwise order before generating arcs so `reverse: false` always bows outward regardless of whether the rectangle corners or polygon points were given clockwise or anticlockwise.

## In a change set (`add_revision_cloud` / `REVCLOUD`)

The `add_revision_cloud` operation (AutoCAD alias `REVCLOUD`) is available through every interface — CLI, HTTP API, MCP server and Python library — from the core's command registry (ADR-0007). It works both on a drawing set and when revising an original `.dwg` ([Revising an original DWG](revisions.md)).

It accepts three mutually exclusive boundary forms:

1. **`rectangle` + `arc_length`** (with `sheet` or `template`, omitted on an original `.dwg`; `from` and `to` may use snapped points).
2. **`polygon` + `arc_length`** (with `sheet` or `template`, omitted on an original `.dwg`; points may use snapped points).
3. **`around`** — one target object or a non-empty list of target objects whose combined drawn extents are framed as a rectangle:
   - `margin` (optional, `>= 0`): clearance added on every side of the framed extents; defaults to half of `arc_length`.
   - `arc_length` (optional): when omitted, defaults to a size proportional to the framed extents (`clamp(perimeter / 20, min_arc, max_arc)`).
   - `sheet` / `template` (optional on a drawing set): when omitted, the cloud is written to the template if every `around` target lives in the same template, or to the target sheet's row otherwise.

```json
{
  "op": "REVCLOUD",
  "around": [
    { "id": "LOOP-01/motor-tag" },
    { "id": "LOOP-01/motor-symbol" }
  ],
  "margin": 0.25,
  "style": "calligraphy",
  "layer": "REVCLOUD"
}
```

Existing transform and edit operations also support `revision_cloud`:
- `move`, `change`, `delete`, `copy`, `array`
- `scale` scales coordinates and `arc_length`
- `mirror` mirrors coordinates across the axis
- `rotate` keeps `rectangle` on multiples of 90° and converts `rectangle` to its 4-corner `polygon` on arbitrary angles so the rotated cloud keeps its exact orientation.

## How AutoCAD writes and reads it

AutoCAD's `REVCLOUD` writes a revision cloud as a closed `LWPOLYLINE` with:
- Bulge `0.52056705` on every vertex (negated when `reverse: true`), matching AutoCAD 2026's `REVCLOUD` arc curvature.
- Normal style: constant width `0.0`, vertex start/end widths `0.0`.
- Calligraphy style: vertex start width `0.0`, vertex end width `0.1 × arc_length`, constant width `0.0`.
- Extended data (`XDATA`) registered under the `APPID` `RevcloudProps` containing group code `1070` (`0`) and group code `1040` (`arc_length`), alongside AgentDrafter's own `AGENTDRAFTER` XData carrying the object's ID.

Through the one drawing pipeline (ADR-0011), PNG renders, PDF plots and the WebGL2 browser viewer all draw `revision_cloud` from the same polyline arcs and tapered calligraphy bands, matching AutoCAD's plot of the same DWG.
