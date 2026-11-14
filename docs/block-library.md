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
| `linetypes` | no | The linetypes the library's layers and objects draw with, as a sheet's. |
| `layers` | no | The layers the blocks' objects sit on, as a sheet's. |
| `hatch_patterns` | no | The hatch patterns the blocks' hatches fill with, as a sheet's. |
| `text_styles` | no | The text styles the blocks' text and attributes draw with, as a sheet's. |
| `blocks` | yes | The blocks: a mapping from each block's name to its definition. |

A block's name follows the rules for a layer's name: 1 to 255 characters, not starting or ending with a space, holding none of `< > / \ " : ; ? * | = , `` ` `` and no control characters. AutoCAD ignores case in names, so two block names that differ only in case are an error. An insert names a block exactly as it is written here.

### Blocks

| Key | Required | What it is |
|---|---|---|
| `base_point` | no | The point of the block that sits at an insert's position, in the block's own coordinates: a point. Without it, `[0, 0, 0]`. |
| `objects` | yes | The block's objects, in drawing order: at least one. Each is a mapping with one key, its type: any [object a sheet holds](sheet-description.md#objects), an `insert` of another of the library's blocks among them (see [Nested blocks](#nested-blocks)), or `attdef`. An object names a layer, text style or hatch pattern of the library's. It takes no `id` and no `name`: an insert of the block is the object on a sheet, with an [ID and a name](sheet-description.md#ids-and-names) of its own. A `text` takes a [`cell`](#cells) as well. |

An object on layer `0` in a block draws with the layer of the insert it is drawn by, as in AutoCAD; an object on any other layer draws with that layer. An object, or an attribute definition, whose `color`, `linetype` or `lineweight` is `ByBlock` draws with the insert's own, as AutoCAD draws it; see [An object's own colour, linetype and lineweight](sheet-description.md#an-objects-own-colour-linetype-and-lineweight).

### Nested blocks

A block may insert another block of the same library, as AutoCAD's blocks nest: a terminal strip that holds terminals, a title block that holds a revision row. The `insert` takes the keys an insert on a sheet takes, but no `id` and no `name`, and its position, scale and rotation are in the enclosing block's own coordinates. The library lists its blocks in any order. A block that inserts itself, directly or through other blocks, is an error that names the round, such as `A to B to A`, and an insert of a block the library does not define lists the blocks it does.

Zach decided on 2026-11-13 that a block that inserts another fills in the inner block's attributes in its own definition, once, as AutoCAD does: the nested insert's `attributes` give their values, and every insert of the outer block shows them. An insert of the outer block on a sheet gives values only for the outer block's own attribute definitions. Giving one for a nested block's tag is an error that says where its value is set, and a change set's `set_attribute` on it is refused the same way.

```yaml
blocks:
  STRIP:
    objects:
      - polyline: {points: [[0, 0, 0], [30, 0, 0], [30, 10, 0], [0, 10, 0]], closed: true}
      - insert: {block: TERMINAL, position: [8, 5, 0], attributes: {NUMBER: '1'}}
      - insert: {block: TERMINAL, position: [22, 5, 0], attributes: {NUMBER: '2'}}
  TERMINAL:
    objects:
      - circle: {center: [0, 0, 0], radius: 4}
      - attdef: {tag: NUMBER, position: [0, 0, 0], height: 3, justify: middle_center}
```

A nested insert draws as an insert on a sheet does, placed by its own position, scale and turn and then by the enclosing insert's. On layer `0` it takes the enclosing insert's layer, and with `ByBlock` the enclosing insert's colour, linetype or lineweight, which its own layer `0` and `ByBlock` objects then take in turn, as AutoCAD draws them. On a frozen layer it draws nothing. A sheet that inserts the outer block holds every block it nests, however deep, and its DWG defines each, the outer block's definition holding the nested insert with its attributes. [The checker](check.md) holds a nested insert's text and lines as it holds the outer block's own.

### Attribute definitions

An `attdef` is AutoCAD's ATTDEF: a labelled text field each insert of the block fills in. It draws as a single line of [text](sheet-description.md#text) does.

| Key | Required | What it is |
|---|---|---|
| `tag` | yes | The attribute's tag: the label an insert fills it in by, such as `TAG` or `TERMINAL`. 1 to 255 characters with no spaces and no control characters; punctuation is kept as written, such as `NOTE#1`, `A/B`, `MM/DD/YY`, `X.Y`, `REV-1`, `#` or `1`. AutoCAD keeps a tag in capitals, so one written in lowercase is taken in capitals, with a notice. A block may define a tag more than once ([below](#a-tag-defined-more-than-once)). |
| `prompt` | no | What AutoCAD asks for when a person inserts the block: one line of text with no control characters. Without it, empty, and AutoCAD asks with the tag. |
| `default` | no | The value an insert takes when it gives none: one line of text, as a `text`'s contents are, which may be empty. A constant attribute's value on every insert. Without it, empty. |
| `position` | yes | The point the attribute's text is justified at, in the block's own coordinates. |
| `height` | yes | The height of the capitals: a length more than 0. |
| `style` | no | The text style: `Standard` or a name under `text_styles`. Without it, `Standard`. |
| `width_factor` | no | How wide characters are drawn, from 0.01 to 100. Without it, the style's. |
| `rotation` | no | Degrees anticlockwise from the block's X axis. Without it, 0. |
| `justify` | no | Where the text sits against its position: one of a `text`'s justifications. Without it, `left`. |
| `invisible` | no | `true` to hide the attribute on every insert, as AutoCAD's Invisible mode hides it. Without it, `false`. |
| `constant` | no | `true` to give the attribute the same value, its `default`, on every insert, as AutoCAD's Constant mode does. See [Attribute modes](#attribute-modes). Without it, `false`. |
| `verify` | no | `true` to have AutoCAD ask a person to confirm the value they type when they insert the block, as its Verify mode does. Without it, `false`. |
| `preset` | no | `true` to have AutoCAD take the default without asking a person who inserts the block, as its Preset mode does. Without it, `false`. |
| `lock_position` | no | `true` to stop a person in AutoCAD moving the attribute apart from its insert, as its Lock position mode does. Without it, `false`, so every DWG built before stays the same (Zach decided on 2026-11-13). |
| `layer` | no | The layer, as for every object. An attribute defined on layer `0` sits on the layer of each insert; one defined on any other layer sits on that layer. |
| `color`, `linetype`, `lineweight`, `linetype_scale` | no | The attribute's own, as for every object; each insert's attribute is written with them. `ByBlock` takes the insert's. |
| `cell` | no | The box the attribute draws within on each insert, such as its title-block cell. See [Cells](#cells). |

### Attribute modes

An attribute definition takes AutoCAD's ATTDEF modes, each `true` or `false`: `invisible`, `constant`, `verify`, `preset` and `lock_position`.

Zach decided on 2026-11-13 how a constant attribute behaves:

- Its value is its `default`, the same on every insert, as AutoCAD's ATTDEF relabels its Default box Value for one. No new key holds it.
- An insert carries no value for it, as an AutoCAD insert carries none. An insert that gives one is an error saying where its value is set, and so is a change set's `set_attribute` on it. This holds through a [nested block](#nested-blocks) too.
- It draws its value on every insert, unless it is invisible too.
- `query` and the viewer's inspector list only the attributes an insert carries, as AutoCAD's Properties palette shows them. A query for text finds a constant attribute's value as it finds a block's own text ([query.md](query.md)).
- It takes no `verify` or `preset`, which AutoCAD's ATTDEF turns off for a constant attribute, since nobody is asked for its value.

`verify`, `preset` and `lock_position` change only what AutoCAD does when a person inserts the block or edits it: nothing draws differently. Each insert's attribute carries its definition's Invisible, Verify, Preset and Lock position, as AutoCAD 2026 gave an insert of the same block it made itself.

### A tag defined more than once

```yaml
blocks:
  REVISIONS:
    objects:
      - attdef: {tag: NOTE, position: [0, 0, 0], height: 2.5, default: '-'}
      - attdef: {tag: NOTE, position: [0, 4, 0], height: 2.5, default: '-'}
      - attdef: {tag: 'NOTE#1', position: [0, 8, 0], height: 2.5}
      - attdef: {tag: rev, position: [0, 12, 0], height: 2.5}
```

A plant's block may define one tag on several attribute definitions, as AutoCAD allows, such as a revision block with a placeholder tag on each line. Zach decided on 2026-11-13 (#99) how AgentDrafter takes one:

- **Each definition is its own attribute.** An insert carries one value for each definition but the constant ones, in the block's order, as AutoCAD's does, and draws each where its own definition is.
- **An insert gives a repeated tag's values as a list**, in the order the block defines them, the first item being the tag's first occurrence: `attributes: {NOTE: [ISSUED, AS BUILT]}`. A tag defined once takes one value. [Inserts](sheet-description.md#a-tag-defined-more-than-once) sets out the rules: a shorter list leaves the rest at their defaults, a longer one is an error, and a null item takes its default.
- **A constant occurrence carries no value**, so a tag that repeats with some occurrences constant takes a list covering only the others, in order; when only one is left, it takes one value.
- **A change set names one occurrence** with `set_attribute`'s `occurrence`, counted from 1, which it needs for a repeated tag ([change sets](change-sets.md)), and a [query](query.md) reports a repeated tag's values as a list.
- **A tag written in lowercase is taken in capitals**, `rev` as `REV`, as AutoCAD keeps every tag, rather than refused. Each letter takes its own capital; a letter whose capital is more than one letter, such as `ß`, stays as written, so a tag keeps the length the schema counts. The reply of the command that read it carries a `tag_in_capitals` notice naming the file, the field and the line: a build's `notices`, a drawing set's sheet's `notices`, and a change set's sheet's `notices`. A query, a check, a render and a plot read the tag in capitals too, without a notice. A tag still cannot hold a space, which AutoCAD refuses as well.

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

Each block is written as AutoCAD writes one: a block definition with the library's base point, in the drawing's unit so that AutoCAD scales no insert again, holding its objects in order, its attribute definitions among them. An insert is AutoCAD's block reference, with one attribute for each attribute definition of its block but the constant ones, in the order the block defines them, each drawn where its definition is, moved, scaled and turned with the insert. In AutoCAD the result is an ordinary block whose attributes a person edits as any other: AutoCAD 2026 opened two built sheets and their audit found no errors, read each block's attribute definitions and each insert's values, and an attribute value changed in AutoCAD saved and audited clean.

An insert's ID and name are kept in the DWG as extended data under the registered application `AGENTDRAFTER`, as every object's [ID and name](sheet-description.md#ids-and-names) are: the text `sheet` followed by the sheet's name, `id` followed by the insert's `id` or its place, then, when it has a name, `name` followed by the name. AutoCAD carries them through a save.

Reading a DWG back gives its block definitions, the inserts nested in them among their objects, and its inserts with their attribute values, scale, rotation and name, and each object's ID. An insert is read only when each attribute is where its block puts it, with its definition's tag, mode, style and justification. Attributes are matched to definitions by their place, so a tag the block defines more than once reads back with each value where its own definition is.

## Changing it through a change set

A change set adds a block to a drawing set's library with `add_block` (AutoCAD's BLOCK), replaces a block's whole definition with `redefine_block` (BEDIT), and brings inserts into line with their blocks' definitions with `attsync` (ATTSYNC), for a library edited by hand. An edit to the library reaches every sheet whose template names it, all or nothing, and a value an insert gives for a tag its block no longer carries is removed from its record as AutoCAD's ATTSYNC removes it. A repeated tag keeps its values by occurrence: a tag the block now defines fewer times loses the list's items past the last it keeps, each listed with its `occurrence`; one it now defines once takes the list's first item alone; and one it now defines more than once takes a single value as its list's first item. [Change sets](change-sets.md#blocks) sets out how each is written and what the reply lists.

## What is not offered yet

- Renaming or deleting a block, and editing the library's own layers, linetypes and text styles, through a change set.
- Dynamic blocks, which come in a later ticket.
- Cells in a nested block: the checker holds only the cells of the block a sheet inserts.
- Annotative blocks and attributes.
- Multiline attributes ([#117](https://github.com/millZach/AgentDrafter/issues/117)), and attributes whose place an insert changes.
- Mirrored and stretched inserts: an insert's scale is the same along every axis and more than 0.
- Arrays of inserts (AutoCAD's MINSERT).
