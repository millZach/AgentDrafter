# Groups (`GROUP`)

AutoCAD's **groups** (`GROUP` / `ACAD_GROUP`) let an agent collect objects on a sheet into a named unit so operations can target the group as one by its name and AutoCAD selects all of its members together.

Groups carry no geometry of their own and add no ink to PNG, PDF, or browser-viewer drawings.

## In a sheet description, template, or rows file

Declare groups under `groups:` at the top level of a sheet description or template, or under `groups:` on an individual row of `rows.yaml`:

```yaml
units: mm
objects:
  - circle: {center: [50, 50, 0], radius: 10, id: ft-body, name: ft-101-body}
  - text: {contents: "FT-101", position: [50, 50, 0], height: 3, id: ft-tag}
  - line: {from: [60, 50, 0], to: [90, 50, 0], id: ft-lead}

groups:
  FT-101:
    objects: [ft-body, ft-tag, ft-lead]
    description: Flow transmitter 101 symbol and lead
    selectable: true
```

Each entry under `groups:` maps a unique group name (case-insensitive across the sheet, including template and row groups combined) to:

| Key | Required | Meaning |
|---|---|---|
| `objects` | yes | Non-empty list of member object references, each matched by local `id`, full `SHEET/id`, or unique `name` on the sheet. |
| `description` | no | Human-readable description stored on the AutoCAD `GROUP` record (default `""`). |
| `selectable` | no | Whether AutoCAD selects the whole group when any member is picked (default `true`). |

A single object may belong to more than one group. A template's `groups:` apply to every sheet built from that template; a row's `groups:` apply to that sheet alone and may reference both template objects and that row's own objects.

## DWG and DXF representation

- Built DWGs (`AC1032`) and DXFs (ASCII and binary) write every group into the drawing's `ACAD_GROUP` dictionary as an AutoCAD `GROUP` object with its `description`, `selectable` flag, member entity handles, and persistent group reactor handles attached to each member entity.
- Reading a DWG or DXF (`read_dwg`, `read`, `query`, `display-list`, `change`) decodes all `ACAD_GROUP` entries (including unnamed `*A...` groups, marked `unnamed: true`).
- When revising an original DWG or DXF (`agentdrafter change`), untouched groups are preserved verbatim (ADR-0004). Deleting a grouped entity automatically prunes its handle from every `ACAD_GROUP` entry that referenced it so no dangling handle is ever written.

## Querying and display list

- **`query groups`**: lists every group on each sheet looked at (`name`, `description`, `selectable`, `unnamed`, `count`, and member `objects` with `id`, `name`, and `type`).
  ```bash
  agentdrafter query groups demo-set --sheet DEMO-001
  ```
- **`--group <name>` / `"group": "<name>"`** on `query find`, `query attributes`, and `query geometry` (and `set_attributes` / `GATTE`'s `where`): filters to objects belonging to a matching group (exact case-insensitive by default, or substring with `--contains` / `"match": "contains"`).
- **`display-list`**: exposes sheet groups under `groups` on the report and each object's group memberships under `groups` on each `SheetObject` without changing drawn strokes or fills.

## Change-set operations

Three operations manage groups on drawing sets (templates or sheet rows) and on original DWGs/DXFs:

| Operation | AutoCAD alias | What it does |
|---|---|---|
| `add_group` | `GROUP`, `G` | Creates a named group (`name`, `targets`, optional `description` and `selectable`) on a `sheet` (row `groups:`), a `template` (template `groups:`), or an original DWG (`ACAD_GROUP`). |
| `ungroup` | `UNGROUP` | Removes the named group without deleting any of its member objects. |
| `edit_group` | `GROUPEDIT` | Edits a named group: `add` members, `remove` members, `rename`, or update `description` / `selectable`. |

### Targeting a group in operations

A target object may name a group instead of a single object:

```json
{ "group": "FT-101" }
```

or, when a group name appears on more than one sheet in a drawing set:

```json
{ "group": "FT-101", "sheet": "DEMO-001" }
```

Operations that act on a group expand it into all of its member objects in order:

- `move` (`MOVE`), `delete` (`ERASE`), `change` (`CHANGE`)
- `copy` (`COPY`), `array` (`ARRAY` / `ARRAYRECT` / `ARRAYPOLAR` / `ARRAYPATH`), `rotate` (`ROTATE` / `RO`), `scale` (`SCALE` / `SC`), `mirror` (`MIRROR` / `MI`)
- `draw_order` (`DRAWORDER` / `DR`), `explode` (`EXPLODE` / `X`), and `add_revision_cloud` (`REVCLOUD` `around`)

Deleting an individual member object prunes it from any group that held it (removing a group in YAML records if its last member was deleted). Single-object-only operations (such as `offset`, `break`, or `set_attribute`) refuse a group target with `bad_value`.
