# Fields (`FIELD`)

AutoCAD's **fields** (`FIELD`) place live values — the drawing's file name, the date it was saved or plotted, a layout or sheet tab name, a custom drawing property (`DWGPROPS`), a property of another object on the sheet, or a raw AutoCAD field expression — inside single-line text (`text`), multiline text (`mtext`), or a block insert's attributes (`insert.attribute_fields`).

When AgentDrafter builds or revises a DWG, every field is written as AutoCAD's `ACAD_FIELD` / `FIELD` / `FIELDLIST` records (with `ACFD_FIELDTEXT_CHECKSUM` matching the displayed text) so AutoCAD updates it live on open, save, plot, or `UPDATEFIELD`. When AgentDrafter renders a PNG, plots a PDF, builds a display list for the viewer, or answers a query, it draws the field's value as it stands through the one drawing pipeline (ADR-0011).

When AgentDrafter revises an existing DWG that already carries fields, untouched fields and their `ACAD_FIELD` / `FIELD` / `FIELDLIST` records come through unchanged (ADR-0004, ADR-0006).

## Custom drawing properties (`custom_properties`)

A sheet description or template may declare custom drawing properties (`DWGPROPS`) at top level:

```yaml
units: mm
custom_properties:
  Project: Demo Mill
  Area: Mixing
```

Property names are compared case-insensitively (matching AutoCAD's `DWGPROPS`). `AgentDrafter` is reserved for AgentDrafter's own build mark and cannot be set under `custom_properties`.

## Field kinds

| `kind` | AutoCAD code | Extra keys | Deterministic evaluation in AgentDrafter |
|---|---|---|---|
| `filename` | `\AcVar Filename` | optional `format` (`%fn2` stem, `%fn6` name + `.dwg`, `%fn7` full path) | `<sheet>.dwg` (or `<sheet>` with `%fn2`) when the sheet name is known, else `----` |
| `save_date` | `\AcVar SaveDate` | optional `format` (e.g. `yyyy-MM-dd`) | explicit `value` if given, else `----` (updated by AutoCAD on save) |
| `plot_date` | `\AcVar PlotDate` | optional `format` | explicit `value` if given, else `----` (updated by AutoCAD on plot) |
| `create_date` | `\AcVar CreateDate` | optional `format` | explicit `value` if given, else `----` |
| `date` | `\AcVar Date` | optional `format` | explicit `value` if given, else `----` |
| `sheet_number` | `\AcVar ctab` | optional `format` | layout tab name (or sheet name / `Model`) |
| `custom_property` | `\AcVar CustomDP.<property>` | `property` (required) | matching `custom_properties` value (case-insensitive), else `----` |
| `object_property` | `\AcObjProp Object(%<\_ObjIdx 0>%).<property>` | `object` (local ID or `#<HEX>`), `property` (`Length`, `Area`, `Radius`, `Diameter`, `Circumference`, `Layer`, `Color`, `Linetype`) | computed from the referenced object, formatted to `%prN` precision (default 2) |
| `code` | verbatim raw field code | `code` (required), optional `objects` (list of referenced object IDs) | explicit `value` if given, else `----` |

All field items accept optional `format` (AutoCAD's `\f "..."` switch, including text case `%tc1` uppercase, `%tc2` lowercase, `%tc3` first capital, `%tc4` title case) and optional `value` (an explicit cached display string).

Date fields never consult the system clock during a build, keeping builds byte-for-byte identical across Windows, Linux and macOS.

## Writing fields in a sheet description

On `text` and `mtext`, `field` may be written in single-item shorthand (with an optional `template`, defaulting to `"{0}"`) or with a multi-item `items` list. When `field` is given, `contents` may be omitted and is evaluated automatically:

```yaml
objects:
  - line:
      id: pipe-1
      from: [0, 0, 0]
      to: [30, 40, 0]
  - text:
      id: pipe-len
      field:
        template: "PIPE LEN: {0} mm"
        kind: object_property
        object: pipe-1
        property: Length
        format: "%lu2%pr1"
      position: [10, 60, 0]
      height: 3.5
  - mtext:
      id: stamp
      field:
        template: "PROJECT: {0}\nFILE: {1}\nSAVED: {2}"
        items:
          - kind: custom_property
            property: Project
            format: "%tc1"
          - kind: filename
            format: "%fn6"
          - kind: save_date
            format: "yyyy-MM-dd"
      position: [10, 120, 0]
      height: 3
      width: 100
```

On an `insert`, `attribute_fields` maps carried attribute tags (or a list of field specs / nulls for a repeated tag) to field definitions:

```yaml
  - insert:
      id: title-block
      block: TITLE_BLK
      position: [180, 10, 0]
      attribute_fields:
        FILE:
          kind: filename
          format: "%fn2"
        PROJECT:
          kind: custom_property
          property: Project
```

## Editing fields with `set_field` (`FIELD`)

In a change set (on a drawing set or an original DWG), `set_field` (alias `FIELD`) attaches, updates, or clears a field on a `text`, `mtext`, or `insert` attribute:

```json
{
  "operations": [
    {
      "op": "FIELD",
      "target": { "id": "SH-001/pipe-len" },
      "field": {
        "template": "PIPE={0}",
        "kind": "object_property",
        "object": "pipe-1",
        "property": "Length",
        "format": "%lu2%pr0"
      }
    },
    {
      "op": "set_field",
      "target": { "id": "SH-001/title-block" },
      "tag": "FILE",
      "field": { "kind": "filename", "format": "%fn6" }
    },
    {
      "op": "set_field",
      "target": { "id": "SH-001/old-field" },
      "field": null,
      "value": "PLAIN TEXT"
    }
  ]
}
```

- Pass `tag` (and optional `occurrence`, counted from 1, for repeated tags) when `target` is an `insert`.
- Pass `"field": null` to remove the field and leave plain text (using `"value"` if given, or keeping the current display text).
- On a drawing set, `set_field` edits the sheet's or template's record and rebuilds touched DWGs with full byte-for-byte undo.
- On an original DWG, `set_field` writes the `ACAD_FIELD` / `FIELD` / `FIELDLIST` records in place in the original's DWG version while keeping every untouched entity and untouched field verbatim.
