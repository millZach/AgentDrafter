# Break and join (`break` / `BREAK` / `BR` / `BREAKATPOINT`, `join` / `JOIN` / `J`)

Two change-set operations split or cut a gap in an object, or merge separate curve pieces back into one, on drawing sets and original `.dwg` revisions:

- **`break`** (AutoCAD aliases **`BREAK`**, **`BR`**, **`BREAKATPOINT`**) splits `target` at a single point `at`, cuts a centered `gap` around `at` (for example breaking a wire around a crossing wire via an intersection snap), or removes the span between `from` and `to`.
- **`join`** (AutoCAD aliases **`JOIN`**, **`J`**) merges two or more lines, arcs, and open polylines under `targets` into one object: collinear lines merge into a single `line` (even across a gap), co-circular arcs merge into an `arc` or `circle`, and end-to-end curves chain into a single `polyline` (auto-closing when start meets end).

---

## `break` (`BREAK`, `BR`, `BREAKATPOINT`)

### Wire-crossing gap around an intersection snap

```json
{
  "operations": [
    {
      "op": "BREAK",
      "target": { "id": "LOOP-101/wire-h" },
      "at": {
        "snap": {
          "id": "LOOP-101/wire-h",
          "snap": "intersection",
          "with": { "id": "LOOP-101/wire-v" }
        }
      },
      "gap": "3 mm",
      "new_id": "wire-h-right"
    }
  ]
}
```

### Split at one point (`BREAKATPOINT`) or remove a span (`from` / `to`)

```json
{
  "operations": [
    {
      "op": "BREAKATPOINT",
      "target": { "id": "LOOP-101/wire-1" },
      "at": { "snap": { "id": "LOOP-101/wire-1", "snap": "midpoint" } },
      "new_id": "wire-1-right"
    },
    {
      "op": "BR",
      "target": { "id": "LOOP-101/ring" },
      "from": [155, 40, 0],
      "to": [140, 55, 0]
    }
  ]
}
```

### Keys

| Key | Required | Meaning |
|---|---|---|
| `op` | yes | `"break"`, `"BREAK"`, `"BR"`, or `"BREAKATPOINT"` (any case). |
| `target` | yes | Object to cut (`line`, `polyline`, `arc`, `circle`, `ellipse`, or `spline`) by `id` or `name`. Refused if on a locked layer. |
| `at` | one mode | Point `[x, y, z]` or snapped point at which to split `target` (`BREAKATPOINT`) or center `gap`. Give `at` (with optional `gap`), or `from` + `to`, not both. |
| `gap` | no (with `at`) | Positive arc-length gap centered at `at` (removes `gap / 2` to each side along the curve). |
| `from`, `to` | one mode | Two points (or snapped points) bounding the span to remove. On closed curves (`circle`, whole `ellipse`, closed `polyline`), removes the anticlockwise/forward span from `from` to `to`, matching AutoCAD. |
| `new_id` | no | Explicit ID for the second object created when an open curve splits into two pieces; assigned automatically when omitted. |

### Which IDs carry on and which are new

- `target`'s ID **carries on** on the surviving start piece (or the single surviving piece when breaking a closed curve into an open `arc` / elliptical arc / open `polyline`, or trimming one end). It is reported under the operation's `id`.
- When an open curve splits into two pieces, the second piece's **new** ID (`new_id` or assigned) is reported under `split_id` and recorded as `added` in the journal.

---

## `join` (`JOIN`, `J`)

```json
{
  "operations": [
    {
      "op": "JOIN",
      "targets": [
        { "id": "LOOP-101/seg-a" },
        { "id": "LOOP-101/seg-arc" },
        { "id": "LOOP-101/seg-b" }
      ]
    }
  ]
}
```

### Keys

| Key | Required | Meaning |
|---|---|---|
| `op` | yes | `"join"`, `"JOIN"`, or `"J"` (any case). |
| `targets` | yes | List of at least two objects (`line`, `arc`, or open `polyline`) on the same sheet, space, and Z plane, in any order. Refused if any target is on a locked layer or is a closed polyline. |
| `tolerance` | no | Non-negative endpoint-matching fuzz distance in the declared unit (or carrying its own unit). Defaults to `0.0001`. |

### Join rules & IDs

- **Collinear lines** (including collinear lines separated by a gap, such as a previously broken wire) merge into a single `line`.
- **Co-circular arcs** merge into a single `arc`, or into a `circle` when their spans cover 360°.
- **Lines, arcs, and open polylines** meeting end to end within `tolerance` (automatically chained regardless of input order or segment orientation) merge into a single `polyline`, preserving arc bulges and widths and setting `closed: true` when the chain closes on itself.
- **`targets[0]`'s ID carries on** on the merged object (along with its name, layer, and properties) and is reported under `id`; the absorbed objects (`targets[1..]`) are removed and reported under `deleted` (and marked `deleted` in the journal).
