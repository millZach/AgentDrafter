# Dynamic blocks

A dynamic block is a block that declares named parameters — visibility states, lookup tables, flips, linear stretch distances, or rotations — so each insert of the block can be placed in a chosen state without defining a separate block for every variant.

All five parameter kinds draw through the one drawing pipeline (ADR-0011) into PNG, PDF, the WebGL2 viewer, extents, object snaps, geometric queries, and the checker, and every built DWG opens in AutoCAD without recovery and with zero audit errors.

## Defining parameters in a block library

Under `blocks.<NAME>.parameters`, map each parameter name (as AutoCAD's Properties palette lists it) to a parameter definition:

```yaml
blocks:
  SELECTOR_SWITCH:
    base_point: [0, 0, 0]
    objects:
      - circle: { center: [0, 0, 0], radius: 5 }
      - line: { from: [0, 0, 0], to: [9, 0, 0] }
      - line: { from: [0, 0, 0], to: [0, 9, 0] }
      - attdef: { tag: TAG, default: HS-100, position: [0, -8.5, 0], height: 2.5, justify: middle_center }
    parameters:
      Visibility:
        kind: visibility
        default: 2-POS
        states:
          2-POS: [0, 1, 3]
          3-POS: [0, 1, 2, 3]
      Flip:
        kind: flip
        axis: [[0, 0, 0], [0, 10, 0]]
        default: false
        labels: [Normal, Reversed]
        objects: [0, 1, 2]
      Angle:
        kind: rotation
        base_point: [0, 0, 0]
        default: 0
        allowed: [0, 90, 180, 270]
        objects: [1, 2]
      Preset:
        kind: lookup
        default: Standard-2P
        rows:
          Standard-2P: { Visibility: 2-POS, Flip: false }
          Reversed-3P: { Visibility: 3-POS, Flip: true }
```

### Parameter kinds

- **`visibility`**: `states` maps each state name to the indices of `objects` (counted from 0) visible in that state. `default` names the default state (first state if omitted). Hidden `attdef` objects keep their carried attribute slot order and become invisible in that state.
- **`flip`**: mirrors `objects` (or all objects if omitted) across the 2-point line `axis` when flipped. Text and attribute definitions remain readable (`MIRRTEXT 0`). Accepts `true`/`false`, `1`/`0`, or the two `labels` (defaults: `Not Flipped`, `Flipped`).
- **`linear`**: stretches vertices inside `stretch_box` on `stretch_objects` (or all objects when both `stretch_objects` and `move_objects` are empty) and translates `move_objects` whole along `from` → `to` by `value - default`. Supports `default`, `min`, `max`, `increment`, and `allowed` lists, all in the library's unit (or explicit unit strings such as `2 in`).
- **`rotation`**: rotates `objects` (or all objects if omitted) anticlockwise about `base_point` by `value - default` degrees. Supports `default`, `min`, `max`, and `allowed` angle lists.
- **`lookup`**: `rows` maps each row name to a mapping of underlying parameter values. Lookup rows expand first; any explicit non-lookup parameter in an insert's `state` overrides the lookup value.

## Placing an insert in a chosen state

On any `insert` in a sheet description, template, row, or `insert_block` (`INSERT`) operation, add `state:`:

```yaml
- insert:
    id: sw-1
    block: SELECTOR_SWITCH
    position: [85, 95, 0]
    attributes:
      TAG: HS-102
    state:
      Preset: Reversed-3P
      Angle: 90
```

Omitted or `null` parameters take their defaults. Unknown parameter names, unknown states or rows, or out-of-range distances and angles are refused with the exact field path and allowed choices.

## Changing an insert's state (`set_block_state` / `PROPERTIES`)

The `set_block_state` operation (AutoCAD alias `PROPERTIES`) changes one or more dynamic parameters of an existing insert in a drawing set or on an original DWG:

```json
{
  "operations": [
    {
      "op": "set_block_state",
      "target": { "id": "SHEET-1/sw-1" },
      "state": { "Visibility": "3-POS", "Flip": true }
    }
  ]
}
```

Setting a parameter to `null` removes it from `state` so it returns to its default. Because `set_block_state` is a typed change-set operation, it is recorded in the journal, previewed by `--dry-run` and proposals, and undone byte for byte by `undo`.

## Reading and revising existing plant DWGs

When AgentDrafter reads an existing DWG (`read`, `query`, `render`, `plot`, `display-list`, `check`):

- Dynamic block definitions (`ACAD_ENHANCEDBLOCK`) and their evaluated anonymous blocks (`*U<n>`) are read whole; entities hidden by an inactive visibility state (`invisible`) inside a dynamic block are skipped cleanly instead of failing the block read.
- Each insert resolves back to its effective dynamic block name (so `query blocks` and `query find --block` find it by its real block name rather than `*U1`), exposes its current parameter values under `state`, and draws its exact evaluated geometry.
- Untouched dynamic blocks, `*U<n>` representations, `ACAD_ENHANCEDBLOCK` evaluation graphs, `AcDbBlockRepresentationData`, and `ACAD_ENHANCEDBLOCKDATA` caches pass through revisions unchanged (ADR-0004).
- On an original DWG whose dynamic block defines an AutoCAD `BlockVisibilityParameter`, `set_block_state` switches the insert's visibility state in place by writing a fresh evaluated `*U<n>` block and updating the insert's `ACAD_ENHANCEDBLOCKDATA` cache while keeping the original DWG read-only and untouched objects verbatim.

## Built DWGs

When `agentdrafter build` writes a dynamic block to DWG 2018:

- The master block record holds the default-evaluated geometry.
- Each distinct non-default state writes an evaluated anonymous block (`*U1`, `*U2`, …) holding the exact evaluated geometry and attribute positions, and the `INSERT` records its effective dynamic block name and state in `AGENTDRAFTER` extended data so reading the built DWG back preserves both the geometry and the state.

See [`examples/dynamic-blocks/`](../examples/dynamic-blocks/) for a complete worked example.
