# The block library

A block library holds the blocks a drawing set shares, each defined once for the whole set. It is a file of its own, written in the same [strict subset of YAML](sheet-description.md#the-yaml-subset) as a sheet description, with the same objects, plus attribute definitions. A [sheet description](sheet-description.md#inserts) names the library it draws on under `block_library` and inserts its blocks by name.

`agentdrafter schema block-library` prints the same keys as a JSON Schema (draft 2020-12), with what each means.

## An example

```yaml
# A made-up block library: a marker with a tag and a hidden note.
units: mm
layers:
  symbols: {color: yellow, lineweight: 0.35}
text_styles:
  tags: {font: romans.shx, width_factor: 0.8}
blocks:
  MARKER:
    objects:
      - circle: {center: [0, 0, 0], radius: 6, layer: symbols}
      - attdef: {tag: TAG, prompt: Tag, default: '?', position: [0, 2.5, 0], height: 2.5, style: tags, justify: middle_center, layer: symbols}
      - attdef: {tag: NOTE, position: [8, 0, 0], height: 2, invisible: true}
```

`examples/blocks/` holds this library, grown a little, and two sheets that insert its blocks: `agentdrafter build examples/blocks/sheet-1.yaml` builds the first.

## What it holds

| Key | Required | What it is |
|---|---|---|
| `units` | yes | The library's declared unit, as a sheet's. A sheet draws only on a library in its own unit; a drawing set has one declared unit. |
| `linetypes` | no | The linetypes the library's layers draw with, as a sheet's. |
| `layers` | no | The layers the blocks' objects sit on, as a sheet's. |
| `hatch_patterns` | no | The hatch patterns the blocks' hatches fill with, as a sheet's. |
| `text_styles` | no | The text styles the blocks' text and attributes draw with, as a sheet's. |
| `blocks` | yes | The blocks: a mapping from each block's name to its definition. |

A block's name follows the rules for a layer's name: 1 to 255 characters, not starting or ending with a space, holding none of `< > / \ " : ; ? * | = , `` ` `` and no control characters. AutoCAD ignores case in names, so two block names that differ only in case are an error. An insert names a block exactly as it is written here.

### Blocks

| Key | Required | What it is |
|---|---|---|
| `base_point` | no | The point of the block that sits at an insert's position, in the block's own coordinates: a point. Without it, `[0, 0, 0]`. |
| `objects` | yes | The block's objects, in drawing order: at least one. Each is a mapping with one key, its type: any [object a sheet holds](sheet-description.md#objects) but `insert`, or `attdef`. An object names a layer, text style or hatch pattern of the library's. It takes no `id` and no `name`: an insert of the block is the object on a sheet, with an [ID and a name](sheet-description.md#ids-and-names) of its own. A `text` takes a [`cell`](#cells) as well. |

An object on layer `0` in a block draws with the layer of the insert it is drawn by, as in AutoCAD; an object on any other layer draws with that layer.

### Attribute definitions

An `attdef` is AutoCAD's ATTDEF: a labelled text field each insert of the block fills in. It draws as a single line of [text](sheet-description.md#text) does.

| Key | Required | What it is |
|---|---|---|
| `tag` | yes | The attribute's tag: the label an insert fills it in by, such as `TAG` or `TERMINAL`. 1 to 255 characters, written in capitals as AutoCAD keeps it, with no spaces and no control characters. A block defines each tag once. |
| `prompt` | no | What AutoCAD asks for when a person inserts the block: one line of text with no control characters. Without it, empty, and AutoCAD asks with the tag. |
| `default` | no | The value an insert takes when it gives none: one line of text, as a `text`'s contents are, which may be empty. Without it, empty. |
| `position` | yes | The point the attribute's text is justified at, in the block's own coordinates. |
| `height` | yes | The height of the capitals: a length more than 0. |
| `style` | no | The text style: `Standard` or a name under `text_styles`. Without it, `Standard`. |
| `width_factor` | no | How wide characters are drawn, from 0.01 to 100. Without it, the style's. |
| `rotation` | no | Degrees anticlockwise from the block's X axis. Without it, 0. |
| `justify` | no | Where the text sits against its position: one of a `text`'s justifications. Without it, `left`. |
| `invisible` | no | `true` to hide the attribute on every insert, as AutoCAD's Invisible mode hides it. Without it, `false`. |
| `layer` | no | The layer, as for every object. An attribute defined on layer `0` sits on the layer of each insert; one defined on any other layer sits on that layer. |
| `cell` | no | The box the attribute draws within on each insert, such as its title-block cell. See [Cells](#cells). |

### Cells

```yaml
      - line: {from: [-6.5, 0.6, 0], to: [0, 0.6, 0]}
      - text: {contents: DWG NO., position: [-6.45, 0.52, 0], height: 0.08, justify: top_left, cell: [[-6.5, 0, 0], [-4, 0.6, 0]]}
      - attdef: {tag: DWG_NO, position: [-5.25, 0.22, 0], height: 0.14, justify: middle_center, cell: [[-6.5, 0, 0], [-4, 0.6, 0]]}
```

Zach decided on 2026-11-11 that a title-block cell is declared in the block library. An attribute definition, or a `text` of a block, may give under `cell` the box it draws within: two opposite corners, `[[x, y, z], [x, y, z]]`, in the block's own coordinates, which differ in x and in y. It is seen from above, so z is not used. On each insert the cell is moved, scaled and turned with the insert, and [the checker](check.md) reports text that draws past it, measured as its font draws it. A text or an attribute with no cell is not checked for overflow; an `mtext` and the objects of a sheet take no cell. Only the checker reads it: a DWG is the same with it or without it. The demo set's title block declares a cell for each of its texts and attributes.

## What a sheet takes from the library

A sheet holds the blocks it inserts, as the library defines them, and the library's layers, linetypes and text styles those blocks use; blocks it does not insert stay out of its DWG. A layer, linetype or text style the sheet defines too must be defined the same in both, with its name written the same: otherwise the build stops with an error at the sheet's definition, since the block would not draw as the library defines it. Layer `0`, `Continuous` and `Standard`, when the library does not list them, are the sheet's own, as they are in AutoCAD.

## How it is written

Each block is written as AutoCAD writes one: a block definition with the library's base point, in the drawing's unit so that AutoCAD scales no insert again, holding its objects in order, its attribute definitions among them. An insert is AutoCAD's block reference, with one attribute for each attribute definition of its block, in the order the block defines them, each drawn where its definition is, moved, scaled and turned with the insert. In AutoCAD the result is an ordinary block whose attributes a person edits as any other: AutoCAD 2026 opened two built sheets and their audit found no errors, read each block's attribute definitions and each insert's values, and an attribute value changed in AutoCAD saved and audited clean.

An insert's ID and name are kept in the DWG as extended data under the registered application `AGENTDRAFTER`, as every object's [ID and name](sheet-description.md#ids-and-names) are: the text `sheet` followed by the sheet's name, `id` followed by the insert's `id` or its place, then, when it has a name, `name` followed by the name. AutoCAD carries them through a save.

Reading a DWG back gives its block definitions and its inserts with their attribute values, scale, rotation and name, and each object's ID. An insert is read only when each attribute is where its block puts it, with its definition's tag, mode, style and justification.

## What is not offered yet

- Dynamic blocks, which come in a later ticket.
- A block inside a block: a block holds no inserts.
- Annotative blocks and attributes.
- Constant, Verify and Preset attributes, multiline attributes, and attributes whose place an insert changes.
- Mirrored and stretched inserts: an insert's scale is the same along every axis and more than 0.
- Arrays of inserts (AutoCAD's MINSERT).
