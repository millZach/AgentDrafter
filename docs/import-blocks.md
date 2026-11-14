# Importing blocks

A plant's title block and symbols usually already exist as blocks in a DWG or a DXF. `agentdrafter import-blocks` reads them into a [block library](block-library.md), as AutoCAD's DesignCenter (ADCENTER) drags one drawing's blocks into another, so they need not be drawn again by hand or converted by a script written for the job. A drawing set's library takes them through a change set's [`import_blocks` operation](#a-drawing-sets-library), so its journal and undo hold the change.

```sh
agentdrafter import-blocks plant-symbols.dxf --library blocks.yaml
agentdrafter import-blocks title.dwg --library blocks.yaml --block TITLE_BLOCK --replace --dry-run
```

| Option | What it does |
|---|---|
| `<dwg \| dxf>` | The drawing to read, told apart by its extension, `.dwg` or `.dxf`, in any case. It is only read. |
| `--library <file>` | The block library to write. When it is not there, it is made. |
| `--block <name>` | Only this block, with every block it nests, however deep. Given more than once, each. A name is matched in any case, as AutoCAD matches it. Without it, every named block of the drawing. |
| `--replace` | Replace a block the library holds already, rather than refuse. |
| `--units <unit>` | The unit the drawing's blocks are drawn in, for a drawing that states none a library takes. |
| `--dry-run` | Do everything but write: the reply says what would be written. |

Through the HTTP API, the MCP server and the Python library the command is `import-blocks` (`import_blocks` in Python), and its input is `{"drawing", "library", "blocks", "replace", "units", "dry_run"}` ([servers.md](servers.md)).

## What it carries

Each block comes with what the core has a model for, as it reads a DWG back: lines, polylines with their arcs and widths, arcs, circles, ellipses, splines, hatches with their patterns and every loop, 2D solids, donuts, text and multiline text, attribute definitions, and inserts of the drawing's other blocks. Each object keeps its layer and its own colour, linetype, lineweight and linetype scale, `ByBlock` among them. An attribute definition keeps its tag, its prompt, its default, its place, height, style, width factor, rotation and justification, and its modes: invisible, constant, verify, preset and lock position. A tag the block defines more than once stays so, each its own attribute, and a tag in lowercase is taken in capitals ([the block library](block-library.md#a-tag-defined-more-than-once)), on the attribute definition and on every nested insert's value, before an insert's values are written and before a block is compared with the library's. A donut, which a drawing holds as a closed polyline of two half circles as wide as its ring, comes back as a `donut`, a filled disc among them; a polyline that only looks like one stays a polyline, or is not carried. A nested insert keeps the values it fills its block's attributes with.

The library gains the layers, linetypes, hatch patterns and text styles the blocks it writes draw on and it lacks, each as the drawing defines it. One the library defines already, by a name the same in any case, is kept as the library defines it, and the blocks name it as the library spells it; when its definition differs from the drawing's, the reply says so with a `definition_kept` notice. A hatch pattern is shared by name only when its definition is the same: one that has the name of another defined otherwise, in the drawing or in the library, is carried under its name with `_2`, then `_3` and so on, the first free in any case, its hatches name that, and a `pattern_renamed` notice names the block, the old name and the new. Layer `0`, `Continuous` and `Standard` are every drawing's own and are never added. When the blocks' text draws in a `Standard` whose font differs from the one the library leaves it at, a `standard_style` notice says so.

Lengths come in the drawing's unit, as it writes them, and numbers as exactly as a DWG holds them. An angle is written to the nearest billionth of a degree, since a DWG keeps angles in radians: a rotation read back as 29.999999999999996 is written 30. A block is written as a library writes one, its objects an item to a line, each key left out where it is what a library takes without it.

A DWG holds no [cells](block-library.md#cells), so a block imported declares none. Add a cell to each text and attribute definition the checker should hold to its box by hand.

## What it cannot carry

An object the core has no model for is left out of its block, and the reply counts it under `not_carried`, block by block, by the type a DXF names it by, such as `{"block": "TITLE", "object_type": "DIMENSION", "count": 3}`. Among them:

- dimensions, leaders, tables, images, wipeouts, points, 3D faces and solids, and any other type the core does not draw yet;
- an object in a true colour ([#106](https://github.com/millZach/AgentDrafter/issues/106)) or with a lineweight that is not standard;
- an object off the XY plane, such as a circle on a tilted plane, and a 3D polyline that is a mesh or spline-fitted;
- a spline that is not fitted through points by AutoCAD's default, or not a plain clamped B-spline;
- an annotative or multiline attribute, and text that is slanted, upside down or backwards;
- an insert that is mirrored, stretched or an array, or whose attributes are not where its block puts them;
- an object on a layer, or in a linetype or text style, the core cannot read, such as a complex linetype that draws text;
- text that would show nothing.

A block left with nothing it can carry is not imported, and the reply says so under `skipped`, still counting under `not_carried` what it holds; an insert of it in another block is not carried either. Model and paper space, layouts, anonymous blocks, such as those AutoCAD makes for dimensions and dynamic blocks, and external references are never offered; `--block` naming one lists it under `skipped`, with why.

## Units

A block library has one declared unit. A library made by an import is in the drawing's: its insertion unit, `$INSUNITS`, when it is inches, millimetres, centimetres, metres or feet. A drawing that states no unit, or another, is refused unless `--units` names the unit its blocks are drawn in; `--units` naming another unit than the one the drawing states is refused too. A library that is there must be in the drawing's unit, and is refused naming both otherwise. Converting a drawing's blocks from one unit to another is not offered yet.

## A block the library holds already

A block the library holds already, by its name in any case, is refused with `duplicate_name`, naming each, and nothing is written. `--replace` replaces its whole definition with the drawing's, as [`redefine_block`](change-sets.md#blocks) does: its base point and every object. Only a block of the same name is replaced; one the library spells otherwise is refused even then. A nested block, one taken only because a block asked for nests it, that the library holds the same, but for its cells, is used as it is; one the library defines otherwise is refused unless `--replace`. When a replaced block's tags change, a value an insert in another block of the library gives for a tag it no longer carries is removed, and the reply lists it under `attributes_removed`. A replaced block that declared cells loses them, and a `cells_dropped` notice says so.

## How the library is written

The library is written through the record editor a change set writes with, so it keeps every byte it had: each new block goes after its last, a block replaced is written again in its own place, and each new layer, linetype, hatch pattern or text style goes after the last entry of its table, or, for a table the library has not got, at the end of the file. A table the library writes in braces is written again, an entry to a line. What it would write is read back as a block library before anything is written, and the file is put in place whole. A library made by an import holds its unit, the tables it needs and its blocks.

## The reply

```json
{"ok": true, "result": {"drawing": "title.dwg", "library": "blocks.yaml", "created": false, "units": "in", "dry_run": false,
  "blocks_added": [{"name": "TERMINAL_STRIP", "objects": 12, "nests": ["TERMINAL"]}, {"name": "TERMINAL", "objects": 2}],
  "blocks_replaced": [],
  "tables_added": {"linetypes": [], "layers": ["symbols"], "hatch_patterns": [], "text_styles": ["tags"]},
  "not_carried": [{"block": "TERMINAL_STRIP", "object_type": "DIMENSION", "count": 1}],
  "notices": []}}
```

`blocks_added` and `blocks_replaced` give each block written, how many objects it holds, and every block it nests, however deep. A dry run gives the same reply, with `dry_run` true, and writes nothing.

## A drawing set's library

The command refuses a library a drawing set's templates name: a set's library is changed only through a change set, so the set's journal and undo hold the change. The command finds a set by its settings (`set.yaml`) in the library's folder or any folder above it, and in the folder it runs in or any folder above that, and refuses the library when any set it finds names it, by any path to the same file, such as a library in a folder beside the set's. It cannot see a set elsewhere on the disk: run it from the set's folder when the library is outside it. So `--replace` replaces a block of a library that is there only when no set it found names the library, and the reply then carries a `not_journaled` notice: a drawing set elsewhere that names the library would not see the change in its journal, so a set's library is changed with `import_blocks`. A change set's `import_blocks` operation, aliased `ADCENTER`, does what the command does:

```json
{"operations": [{"op": "import_blocks", "from": "../parts/title.dwg", "blocks": ["TITLE_BLOCK"], "replace": true}]}
```

It takes `from`, the drawing's path relative to the set's folder or absolute; `library`, as the [block operations](change-sets.md#blocks) take it, left out when every template names the same library; `blocks`, `replace` and `units`, as the command's options. The drawing must be in the set's declared unit. It lands as the other block operations do: it reaches every sheet whose template names the library, all or nothing, the journal records it, and `agentdrafter undo` takes it back byte for byte. A block it replaces has its inserts brought into line as `redefine_block` brings them, and the reply lists them under `inserts` and each value removed under `attributes_removed`. The operation's `imported` holds what the command's reply holds: the blocks added and replaced, the table entries added, `not_carried`, `skipped` and the notices.

## How it works

`crates/agentdrafter-core/src/dwg/import.rs` reads a DWG or a DXF through acadrust, the one seam (ADR-0009): each named block, once every block it inserts has been read, with the readers a DWG read back uses, counting what they cannot read. `src/import.rs` plans the import against the library as it is, writes it with the record editor (`src/description/edit.rs`), and holds the command; `src/change/apply.rs` applies the operation. The command writes no DWG, so it has nothing for the [judge](../tools/judge/README.md) to open; a sheet built on the imported library is built and judged as any other.

## What is not offered yet

- Converting a drawing's blocks from one unit to another.
- Cells read from a drawing: they are added by hand.
- The objects listed [above](#what-it-cannot-carry), until the core draws them.
- Renaming a block as it is imported, and importing from a drawing a sheet's own objects rather than its blocks (AutoCAD's WBLOCK of a selection).
