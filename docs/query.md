# Queries

A query is what an agent asks a drawing set about what is on its sheets, to check its own work: which objects carry a tag or say a piece of text, which blocks a sheet inserts, what an insert's attributes are, how far apart two objects are and how large a sheet is (see [`CONTEXT.md`](../CONTEXT.md)). Objects are selected by [ID](sheet-description.md#ids), name, block, attribute or text, never by coordinates. A query changes nothing.

`agentdrafter query <kind> <drawing-set-folder | sheet-description> [options]` asks a drawing set's folder, or one sheet description, and prints one line of JSON. It is a thin layer over the core's `query_with`, which takes a request and returns the answer as plain data (ADR-0007).

## What a query reads

A query reads the records: each sheet description, and each template filled in from its row of the set, as a build reads them. It does not read the built DWGs. The records are each sheet's truth (ADR-0003), so a query needs no build, cannot read a DWG that has gone stale, and answers in the set's declared unit exactly as the record gives each length. That is the smallest thing that answers every question here: the IDs, names, blocks, attribute values and text are all in the records, and a sheet's extents come from the one drawing pipeline (ADR-0011) the PNG is drawn by. Every object a build writes carries its ID into the DWG as well, so a later query of a built or revised DWG can find it again; querying DWGs is not offered yet.

A sheet that cannot be read stops the query with its error, placed in its file: an answer that left a sheet out could say nothing matched when something did.

## The reply

Every answer has the same frame, then what its kind found:

| Key | What it is |
|---|---|
| `source` | The drawing set's folder, or the sheet description, asked. |
| `units` | The declared unit every length in the answer is in: the set's, or the sheet description's. |
| `sheets` | The sheets looked at, by name, in the set's order. A sheet description on its own is named by its file without the extension. |
| `message` | What was found, in words. When nothing matched, it starts `nothing matches:` and says what was looked for and where. |
| `kind` | The query's kind. |

`--sheet <sheet>` looks at the one sheet of that name, in any case; a sheet the set does not hold is the `unknown_name` error, which lists its sheets.

**A query that matches nothing is not an error.** Its reply is `ok`, its `count` is 0 and its `message` says so:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001","DEMO-002","DEMO-003"],"message":"nothing matches: no object on the 3 sheets looked at has the text `no such`","kind":"find","count":0,"objects":[]}}
```

## Find

`agentdrafter query find <source> [--id <id>] [--name <name>] [--block <block>] [--tag <tag>] [--value <value>] [--text <text>] [--contains] [--sheet <sheet>]` lists the objects that match every option given; with none, every object.

| Option | Matches |
|---|---|
| `--id` | The object's ID, such as `DEMO-001/border`. |
| `--name` | The object's name, such as `title-block`. Only inserts carry names so far. |
| `--block` | An insert of the block. |
| `--tag` | An insert carrying the attribute; with `--value`, the attribute the value is looked for in. |
| `--value` | An insert one of whose attributes has the value, or the attribute `--tag` names. |
| `--text` | A text or an mtext saying it. |

**How each matches.** An ID, a name, a block and an attribute tag are identifiers, and no two of a kind differ only in case, so each matches the whole of it in any case: `--name TITLE-BLOCK` finds `title-block`. A value and a text match the whole of it, case and all. With `--contains`, every option matches a value that holds it, in any case: `--id border --contains` finds the border on every sheet, and `--text made-up --contains` the notes.

Each object found comes with its sheet, its ID, whether the record writes that ID (`id_written`; an ID the record does not write is the object's place and moves when an object is added before it), where it is written (`file`, `field`, `line`, `column`), and the object as the sheet holds it:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-002"],"message":"1 object has the ID `DEMO-002/border`, on sheet `DEMO-002`","kind":"find","count":1,"objects":[{"sheet":"DEMO-002","id":"DEMO-002/border","id_written":true,"file":"examples/demo-set/templates/ansi-b.yaml","field":"objects[1]","line":18,"column":5,"object":{"type":"polyline","points":[[0.5,0.5,0.0],[16.5,0.5,0.0],[16.5,10.5,0.0],[0.5,10.5,0.0]],"closed":true,"layer":"border"}}]}}
```

## Attributes

`agentdrafter query attributes <source>` with the options of `find` gives each matching insert's attributes as JSON, from tag to value in the order the block defines them:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001"],"message":"1 insert has the name `title-block`, on sheet `DEMO-001`","kind":"attributes","count":1,"inserts":[{"sheet":"DEMO-001","id":"DEMO-001/objects[2]","name":"title-block","block":"TITLE_BLOCK","attributes":{"TITLE":"AREA 100 GENERAL ARRANGEMENT","DWG_NO":"DEMO-001","SHEET":"1 OF 3","REV":"0","DATE":"2026-11-11","DRAWN":"AD"},"file":"examples/demo-set/templates/ansi-b.yaml","field":"objects[2]","line":19,"column":5}]}}
```

## Blocks

`agentdrafter query blocks <source> [--sheet <sheet>]` lists, for each sheet, the blocks it inserts, in the order it first inserts each, with the IDs of the inserts. `count` is how many inserts there are.

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001","DEMO-002","DEMO-003"],"message":"3 inserts of 1 block on the 3 sheets looked at: TITLE_BLOCK","kind":"blocks","count":3,"blocks":[{"sheet":"DEMO-001","blocks":[{"block":"TITLE_BLOCK","inserts":["DEMO-001/objects[2]"]}]},"..."]}}
```

## Measure

`agentdrafter query measure <source> (--from <id> | --from-name <name>) [--from-point <point>] (--to <id> | --to-name <name>) [--to-point <point>] [--sheet <sheet>]` gives the distance from a point of one object to a point of another, in the declared unit, and how far apart they are along X, Y and Z (`delta`). Both objects are on one sheet: two sheets are two drawings.

An object is named by its ID or by its name. A name is given once on a sheet, and a template's names are on every sheet built from it, so a name found on more than one of the sheets looked at needs `--sheet`.

A point is named as its key in the sheet description. Without one, the object's own point is used, the first in this table:

| Object | Points |
|---|---|
| `insert`, `text`, `mtext` | `position` |
| `line` | `from`, `to` |
| `polyline` | `points[0]`, `points[1]`, ... |
| `circle`, `arc`, `ellipse` | `center` |
| `spline` | `fit_points[0]`, ... or `control_points[0]`, ... |
| `hatch` | none: measure from an object at its boundary |

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001","DEMO-002","DEMO-003"],"message":"18.179658962697843 in from the position of `DEMO-001/heading` to the position of `DEMO-001/objects[2]`, on sheet `DEMO-001`","kind":"measure","distance":18.179658962697843,"delta":[15.5,-9.5,0.0],"from":{"sheet":"DEMO-001","id":"DEMO-001/heading","type":"text","point":"position","at":[1.0,10.0,0.0]},"to":{"sheet":"DEMO-001","id":"DEMO-001/objects[2]","name":"title-block","type":"insert","point":"position","at":[16.5,0.5,0.0]}}}
```

## Extents

`agentdrafter query extents <source> [--sheet <sheet>] [--fonts <folder>]...` gives, for each sheet, the box round everything it draws, seen from above, as AutoCAD's extents are: `min` and `max` corners, `width` and `height`, in the declared unit, or `null` for a sheet that draws nothing. The sheet is drawn through the drawing pipeline that draws the PNG, so text counts as its font draws it, and the reply lists, under `font_substitutions`, each text style whose font was not in a font folder and the look-alike its text was measured with. Curves are drawn as chords within a millionth of the sheet's larger side, so how far a curve reaches is found to within that; straight lines are exact. The demo set's sheets draw within their trim line:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-003"],"message":"sheet `DEMO-003` draws within 17 by 11 in, from [0, 0] to [17, 11]","kind":"extents","count":1,"extents":[{"sheet":"DEMO-003","extents":{"min":[0.0,0.0],"max":[17.0,11.0],"width":17.0,"height":11.0}}],"font_substitutions":[{"style":"notes","font":"romans.shx","used":"Hershey Roman Simplex","why":"no font folder is set"},"..."]}}
```

## Errors

Errors take the shape and codes a sheet description's do ([Errors](sheet-description.md#errors)); `field` is the place in the request, such as `query.from.id`.

| Code | In a query it means |
|---|---|
| `no_such_object` | A measure's end names an ID or a name no object carries on the sheets looked at. The message says none matched and lists nothing. |
| `bad_value` | A measure's ends are on two sheets, a name to measure from is given on more than one of the sheets looked at, or the object has no point of its own, such as a hatch. |
| `unknown_name` | A sheet the set does not hold, or a point the object does not have; the message lists the choices. |
| `usage` | A measure's end names its object by both ID and name, or by neither, or the command was called wrongly. |
| any other | A sheet that cannot be read, with its own error in its file, such as `duplicate_id`. |

## How it works

`crates/agentdrafter-core/src/query.rs` reads the set's settings and rows and each sheet's record (`description/records.rs`, and `description/template.rs` for a sheet of a set), selects and measures, and draws a sheet's extents with `draw::draw_sheet`.

## What is not offered yet

- Querying DWGs, built or someone else's. Every DWG a build writes carries each object's ID, ready for it.
- Names on objects other than inserts.
- Patterns other than a whole value or a part of it, such as wildcards.
- A distance from a point given by coordinates. Nothing is selected by coordinates.
