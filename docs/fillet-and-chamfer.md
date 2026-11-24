# Fillet and chamfer (`fillet` / `FILLET` / `F`, `chamfer` / `CHAMFER` / `CHA`)

Two change-set operations round or bevel corners between two objects or at every corner of a flat polyline, on drawing sets and original `.dwg` revisions:

- **`fillet`** (AutoCAD aliases **`FILLET`**, **`F`**) rounds the corner between two objects (`first` and `second` — lines, arcs, or circles) or every straight corner of a flat polyline (`target`) with an arc of `radius`, or uses `radius: 0` to bring two objects to a sharp corner (or square up existing arc corners on a polyline). Two parallel lines are capped with a semicircle whose radius is half their separation, as AutoCAD does.
- **`chamfer`** (AutoCAD aliases **`CHAMFER`**, **`CHA`**) bevels the corner between two lines (`first` and `second`) or every straight corner of a flat polyline (`target`) using equal `distance`, asymmetric `distance1` and `distance2`, or `length` and `angle` (in degrees), including zero distances to bring two lines to a sharp corner.

---

## `fillet` (`FILLET`, `F`)

### Two-object mode

```json
{
  "operations": [
    {
      "op": "F",
      "first": { "id": "PNL-001/wire-horiz" },
      "second": { "id": "PNL-001/wire-vert" },
      "radius": "5 mm",
      "new_id": "wire-bend"
    }
  ]
}
```

### Polyline mode (every corner of a flat polyline)

```json
{
  "operations": [
    {
      "op": "FILLET",
      "target": { "id": "PNL-001/enclosure" },
      "radius": 5
    }
  ]
}
```

### Keys

| Key | Required | Meaning |
|---|---|---|
| `op` | yes | `"fillet"`, `"FILLET"`, or `"F"` (any case). |
| `first`, `second` | two-object mode | Two objects (`line`, `arc`, or `circle`), by `id` or `name`, in the same sheet, space, and Z plane. Give `first` + `second`, or `target`, not both. |
| `target` | polyline mode | Flat `polyline` whose straight-to-straight corners are all filleted in place (or whose existing arc corners are squared back when `radius: 0`). |
| `radius` | yes | Non-negative fillet radius in the declared unit (or carrying its own unit, e.g. `"5 mm"`). `0` brings two objects to a sharp corner (with no arc added) or squares arc corners on a polyline. |
| `trim` | no (two-object) | Whether `first` and `second` are trimmed or extended to the fillet arc (AutoCAD's `TRIMMODE`). `true` when omitted; `false` leaves both untouched and adds only the arc. |
| `first_end`, `second_end` | no (two-object) | `"start"` or `"end"` to disambiguate which leg to keep when two objects cross. When omitted, each object keeps the endpoint farther from the corner. |
| `new_id` | no (two-object) | Explicit ID for the created fillet arc (when `radius > 0`); assigned automatically when omitted. |
| `layer` | no (two-object) | Layer for the created fillet arc; inherits `first`'s layer when omitted. |

---

## `chamfer` (`CHAMFER`, `CHA`)

### Two-object mode

```json
{
  "operations": [
    {
      "op": "CHA",
      "first": { "id": "PNL-001/edge-top" },
      "second": { "id": "PNL-001/edge-right" },
      "distance1": 6,
      "distance2": 4,
      "new_id": "top-right-bevel"
    }
  ]
}
```

### Polyline mode (every corner of a flat polyline)

```json
{
  "operations": [
    {
      "op": "CHAMFER",
      "target": { "id": "PNL-001/cutout" },
      "length": 5,
      "angle": 45
    }
  ]
}
```

### Keys

| Key | Required | Meaning |
|---|---|---|
| `op` | yes | `"chamfer"`, `"CHAMFER"`, or `"CHA"` (any case). |
| `first`, `second` | two-object mode | Two non-parallel `line` objects, by `id` or `name`, in the same sheet, space, and Z plane. Give `first` + `second`, or `target`, not both. |
| `target` | polyline mode | Flat `polyline` whose straight-to-straight corners are all bevelled in place. |
| `distance` | one distance/angle form | Equal non-negative chamfer cutback along both legs (`0` brings two lines to a sharp corner). |
| `distance1`, `distance2` | one distance/angle form | Non-negative chamfer cutback along `first` (or each incoming polyline segment) and `second` (or each outgoing segment). |
| `length`, `angle` | one distance/angle form | Non-negative cutback `length` along `first` (or incoming polyline segment) and chamfer `angle` in degrees strictly between `0` and `180`. |
| `trim`, `first_end`, `second_end`, `new_id`, `layer` | no (two-object) | Same meaning as in `fillet`. |

---

## Behaviour notes

- **Locked layers:** Refused with `locked` if any object that would be modified sits on a locked layer. In two-object mode with `trim: false`, locked source objects are left untouched and only the connecting arc or line is added.
- **Polyline short segments:** Corners whose cutback exceeds an adjacent segment's available length are skipped (reported in the operation's `message`), matching AutoCAD; if no corner on the polyline can be modified, the operation fails with `bad_value`.
- **Journal and undo:** In two-object mode, modified `first`/`second` objects are recorded under `transformed` (changed) and any created arc/line is reported under `id` (added); in polyline mode, `target` keeps its ID (changed). Both drawing-set and original-DWG edits undo byte for byte.
