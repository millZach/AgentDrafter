# Trim and extend (`trim` / `TRIM` / `TR`, `extend` / `EXTEND` / `EX`)

Two change-set operations shorten or lengthen curve geometry against one or more boundary objects—including block inserts (evaluated in their chosen dynamic state)—on drawing sets and original `.dwg` revisions:

- **`trim`** (AutoCAD aliases **`TRIM`**, **`TR`**) cuts `target` back to `boundary` / `boundaries`, naming the part to keep by an end rather than a picked point.
- **`extend`** (AutoCAD aliases **`EXTEND`**, **`EX`**) carries an open `target` on at its `start`, `end`, or `both` ends to the nearest boundary crossing.

---

## `trim` (`TRIM`, `TR`)

```json
{
  "operations": [
    {
      "op": "TR",
      "target": { "id": "LOOP-001/wire-through" },
      "boundary": { "id": "LOOP-001/transmitter" },
      "keep": "ends",
      "new_id": "wire-through-right"
    }
  ]
}
```

### Keys

| Key | Required | Meaning |
|---|---|---|
| `op` | yes | `"trim"`, `"TRIM"`, or `"TR"` (any case). |
| `target` | yes | Object to cut (`line`, `polyline`, `arc`, `circle`, `ellipse`, or `spline`) by `id` or `name`. Refused if on a locked layer. |
| `boundary` / `boundaries` | yes (one) | One cutting-edge object (`boundary`) or a non-empty list (`boundaries`), by `id` or `name`, in the same space as `target`. Any object whose geometry has curves—including block inserts—can act as a boundary. |
| `keep` | yes | Which part to keep: `"start"` (start to first crossing), `"end"` (last crossing to end), `"between"` (first to last crossing, ≥ 2 crossings), or `"ends"` (removes the middle between first and last crossings, splitting an open curve into two pieces). On closed curves (`circle`, whole `ellipse`, closed `polyline`, ≥ 2 crossings), `"start"`/`"between"` keeps the forward span between crossings 1 and 2 and `"end"`/`"ends"` keeps the complementary span. |
| `new_id` | no | Explicit ID for the second piece created when `keep: "ends"` splits an open object; when omitted, an ID is assigned automatically. |

### IDs and splits

- `target` keeps its ID (and name, layer, properties, and space), even when trimming a `circle` converts it into an `arc`.
- When `keep: "ends"` splits an open object in two, the start piece keeps `target`'s ID and the end piece receives `new_id` (or an assigned free ID), reported under the operation's `split_id` and recorded as added in the journal.

---

## `extend` (`EXTEND`, `EX`)

```json
{
  "operations": [
    {
      "op": "EX",
      "target": { "id": "LOOP-001/wire-short" },
      "boundaries": [
        { "id": "LOOP-001/bus-left" },
        { "id": "LOOP-001/bus-right" }
      ],
      "end": "both"
    }
  ]
}
```

### Keys

| Key | Required | Meaning |
|---|---|---|
| `op` | yes | `"extend"`, `"EXTEND"`, or `"EX"` (any case). |
| `target` | yes | Open curve object to extend (`line`, open `polyline` including arc bulges, `arc`, elliptical arc, or open `spline`). Closed curves (`circle`, whole `ellipse`, closed `polyline`, closed `spline`) and locked layers are refused. |
| `boundary` / `boundaries` | yes (one) | One boundary object (`boundary`) or a non-empty list (`boundaries`), by `id` or `name`, in the same space as `target`. |
| `end` | yes | `"start"`, `"end"`, or `"both"`. Each requested end is carried on along its line, arc, ellipse, or endpoint tangent to the nearest crossing with any boundary; refused with `bad_value` if it does not meet a boundary. |
