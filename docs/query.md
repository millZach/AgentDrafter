# Queries

A query is what an agent asks a drawing set about what is on its sheets, to check its own work: which objects carry a tag or say a piece of text, which blocks a sheet inserts, what an insert's attributes are, how far apart two objects are, how large a sheet is, which points AutoCAD's object snaps find on an object and how long an object is and what it encloses (see [`CONTEXT.md`](../CONTEXT.md)). Objects are selected by [ID](sheet-description.md#ids-and-names), name, block, attribute or text, and by where they are: within or crossing a box, or touching another object. A box is the only coordinates a query takes: it is how AutoCAD selects, a window or a crossing dragged on the sheet, and how an agent asks what is in a part of a sheet, which no ID or name says. A query changes nothing.

`agentdrafter query <kind> <drawing-set-folder | sheet-description> [options]` asks a drawing set's folder, or one sheet description, and prints one line of JSON. It is a thin layer over the core's `query_with`, which takes a request and returns the answer as plain data (ADR-0007).

## What a query reads

A query reads the records: each sheet description, and each template filled in from its row of the set, with the row's [own objects](drawing-set.md#a-rows-own-objects), as a build reads them. It does not read the built DWGs. The records are each sheet's truth (ADR-0003), so a query needs no build, cannot read a DWG that has gone stale, and answers in the set's declared unit exactly as the record gives each length. That is the smallest thing that answers every question here: the IDs, names, blocks, attribute values and text are all in the records, and a sheet's extents come from the one drawing pipeline (ADR-0011) the PNG is drawn by. Every object a build writes carries its ID into the DWG as well, so a later query of a built or revised DWG can find it again; querying DWGs is not offered yet.

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

`--sheet <sheet>` looks at the one sheet of that name, in any case; a sheet the set does not hold is the `unknown_name` error, which lists its sheets. While a change set has landed on some of the set's sheets and failed on others, the reply also carries the set's mark under `inconsistent` ([Change sets](change-sets.md#an-inconsistent-set)).

**A query that matches nothing is not an error.** Its reply is `ok`, its `count` is 0 and its `message` says so:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001","DEMO-002","DEMO-003"],"message":"nothing matches: no object on the 3 sheets looked at has the text `no such`","kind":"find","count":0,"objects":[]}}
```

## Find

`agentdrafter query find <source> [--id <id>] [--name <name>] [--block <block>] [--tag <tag>] [--value <value>] [--text <text>] [--contains] [--window <x1,y1,x2,y2>] [--crossing <x1,y1,x2,y2>] [--touching <id> | --touching-name <name>] [--sheet <sheet>] [--fonts <folder>]...` lists the objects that match every option given; with none, every object.

| Option | Matches |
|---|---|
| `--id` | The object's ID, such as `DEMO-001/border`. |
| `--name` | The object's name, such as `title-block`. Any object may carry one. |
| `--block` | An insert of the block. |
| `--tag` | An insert carrying the attribute; with `--value`, the attribute the value is looked for in. |
| `--value` | An insert one of whose attributes has the value, or the attribute `--tag` names: any occurrence of a tag its block defines more than once. |
| `--text` | A text or an mtext saying it, a table one of whose cells says it, a multileader whose text or an older leader whose annotation says it, or an insert whose block draws a text or an mtext saying it. |
| `--window` | An object wholly within the box, as AutoCAD's selection window takes it: see [Within a box, and touching](#within-a-box-and-touching). |
| `--crossing` | An object within the box or crossing its edge, as AutoCAD's crossing selection takes it. |
| `--touching`, `--touching-name` | An object whose geometry meets the object of that ID or name. |

**How each matches.** An ID, a name, a block and an attribute tag are identifiers, and no two of a kind differ only in case, so each matches the whole of it in any case: `--name TITLE-BLOCK` finds `title-block`. A value and a text match the whole of it, case and all. With `--contains`, every option matches a value that holds it, in any case: `--id border --contains` finds the border on every sheet, and `--text made-up --contains` the notes.

Each object found comes with its sheet, its ID, whether the record writes that ID (`id_written`; an ID the record does not write is the object's place and moves when an object is added before it), where it is written (`file`, `field`, `line`, `column`: the rows file its row is in and a place such as `rows[1].objects[0]` there for a row's own object), and the object as the sheet holds it:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-002"],"message":"1 object has the ID `DEMO-002/border`, on sheet `DEMO-002`","kind":"find","count":1,"objects":[{"sheet":"DEMO-002","id":"DEMO-002/border","id_written":true,"file":"examples/demo-set/templates/ansi-b.yaml","field":"objects[1]","line":18,"column":5,"object":{"type":"polyline","points":[[0.5,0.5,0.0],[16.5,0.5,0.0],[16.5,10.5,0.0],[0.5,10.5,0.0]],"closed":true,"layer":"border"}}]}}
```

**Text an insert's block draws.** `--text` also looks at the text and multiline text in the block each insert draws, as its block library defines the block, and at its [constant attributes](block-library.md#attribute-definitions)' values, which the definition holds. The object found is the insert, and its `block_text` lists each text of the block that matched: the block library's `file`, with `folder/..` taken out of its path where the shorter path names the same file, as it does unless a folder on the way is a link, the text's place in the block definition (`field`) and what it says. The demo set's title block says `DEMO PLANT` on every sheet:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001"],"message":"1 object has the text `DEMO PLANT`, on sheet `DEMO-001`","kind":"find","count":1,"objects":[{"sheet":"DEMO-001","id":"DEMO-001/objects[2]","id_written":false,"file":"examples/demo-set/templates/ansi-b.yaml","field":"objects[2]","line":19,"column":5,"object":{"type":"insert","...":"..."},"block_text":[{"file":"examples/demo-set/blocks.yaml","field":"blocks.TITLE_BLOCK.objects[7]","contents":"DEMO PLANT"}]}]}}
```

An attribute's value is not text the block draws: it is found with `--value`.

### Within a box, and touching

`--window` and `--crossing` take a box by two opposite corners, `x1,y1,x2,y2`, in the declared unit and in either order; in JSON, `"window": [[x1, y1], [x2, y2]]`, a corner taking a height that is not looked at. Everything is seen from above. An object is looked at as it draws, through the one drawing pipeline (ADR-0011), every layer continuous as for [extents](#extents), so its text counts as its font draws it, an insert as everything its block draws, its attributes among it, and a table or a multileader as its lines and its text; an object on a layer that is off or frozen draws nothing and is not selected, as in AutoCAD. The box's edges are in it. On a sheet drawn in a [layout](sheet-description.md#layouts-and-viewports), each object is looked at where it is in its own space, as AutoCAD selects in the space it is in: a model-space object at full size, whether or not a viewport shows it, and a paper-space object, a viewport's frame or boundary among them, on the paper; a box takes objects of every space that lie in it. `--space` (`space` in JSON) takes only the objects of one space: `model`, or a layout's paper space by its name, or `paper` on a sheet drawn in one, so a box on the paper of a sheet drawn in [several layouts](sheet-description.md#several-layouts) names its layout. On a sheet drawn in a layout each object found says its `space`, `model` or its layout's name.

- **A window** takes an object that draws something and draws all of it inside the box, a hatch's boundary too, which its pattern's lines may stop short of, and a wide polyline's or a donut's whole band.
- **A crossing** takes an object that draws something inside the box or across its edge: a line or a curve, the outline of a filled area such as a solid, a hatch's boundary or its pattern's lines, and each wide segment's band, so the seam where two wide segments meet, or a donut's two half circles, is an edge too. A wide arc, a donut's among them, is taken by a box anywhere inside its band. Otherwise a box wholly inside a filled area crosses nothing, as in AutoCAD 2026: inside a solid, a straight band, a hatch, solid or patterned, between its lines, or a circle's outline, and in the corner of a hatch's box outside it. A wide polyline is taken alike wherever it is drawn: on the sheet, in an insert's block or in a multileader's.
- **An older leader** is selected by its own line, hook and arrow: AutoCAD holds its annotation as an MTEXT of its own, which AutoCAD's window selects apart from the LEADER. A multileader is one object with its text, as in AutoCAD.

```json
{"ok":true,"result":{"source":"examples/snaps/lugs.yaml","units":"mm","sheets":["lugs"],"message":"1 object is within the window from [26, 33] to [57, 59], on the sheet `lugs`","kind":"find","count":1,"objects":[{"sheet":"lugs","id":"lugs/strip","...":"..."}]}}
```

`--touching` takes an object by its ID, or `--touching-name` by its name (`"touching": {"id": ...}` in JSON), and finds the objects whose geometry meets it, so an agent can check that a wire reaches its terminal. Two objects meet where one crosses or touches the other, as an [intersection snap](#snap) finds it, or where an end of a part of either lies on a part of the other, a line carrying on from another's end among them, within a billionth of their size, in space: a wire a unit above a terminal does not reach it. Their geometry is their curves as [`snap`](#snap) reads them, an insert's its block's, nested inserts too; text, hatches and multileaders have none and touch nothing. The object is looked for on each sheet looked at, by its ID on the sheet it gives, or by its name on every sheet that has it. Each object found lists under `touches` where it meets it, `at`, with `part`, its own part there, and `with_part`, the named object's, as a snap's `part` and `with_part` name them, each place once:

```json
{"ok":true,"result":{"source":"examples/snaps/lugs.yaml","units":"mm","sheets":["lugs"],"message":"1 object is touching `lugs/wire`, on the sheet `lugs`","kind":"find","count":1,"objects":[{"sheet":"lugs","id":"lugs/strip","...":"...","touches":[{"part":"objects[0].objects[1]","at":[50.0,52.0,0.0]},{"part":"objects[2].segments[2]","at":[28.0,57.86666666666667,0.0]}]}]}}
```

A box and `--touching` combine with every other option, and work in `attributes` and `geometry` as in `find`: `query find <set> --touching-name wire-12 --block TERMINAL` finds the terminals a wire reaches. An object to touch that no sheet looked at holds is not an error: nothing matches, and the message names it.

**What AutoCAD 2026 gave.** On a made-up sheet of every kind of object, with inserts scaled, turned and nested, a solid, solid and patterned hatches of straight and curved edges, wide polylines straight, mitred and arced, on the sheet, in an insert and in a multileader's block, a donut and turned text, 161 boxes, each asked as a window and as a crossing, selected the same objects as AutoCAD 2026's `ssget "_W"` and `ssget "_C"` with the box zoomed into view, 317 of 322, but five crossings, each wholly inside a filled area, crossing none of its edges, that AutoCAD took: it takes a solid hatch or a straight band across the inner edges of the triangles it fills it with, which are its own, so a box on a square hatch's or a band's one diagonal took it and a box off it, or on the other diagonal, did not. AgentDrafter does not take them by a box inside them. A box whose edge passes within a pixel of an object at the zoom it is taken at may differ, as AutoCAD's selection is made on the screen.

## Attributes

`agentdrafter query attributes <source>` with the options of `find` gives each matching insert's attributes as JSON, from tag to value in the order the block defines them, with `block_text` as `find` gives it:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001"],"message":"1 insert has the name `title-block`, on sheet `DEMO-001`","kind":"attributes","count":1,"inserts":[{"sheet":"DEMO-001","id":"DEMO-001/objects[2]","name":"title-block","block":"TITLE_BLOCK","attributes":{"TITLE":"AREA 100 GENERAL ARRANGEMENT","DWG_NO":"DEMO-001","SHEET":"1 OF 3","REV":"0","DATE":"2026-11-11","DRAWN":"AD"},"file":"examples/demo-set/templates/ansi-b.yaml","field":"objects[2]","line":19,"column":5}]}}
```

A tag the insert's block defines more than once ([the block library](block-library.md#a-tag-defined-more-than-once)) maps to a list of its values, in the order the block defines them, occurrence 1 first, as an insert gives them: `"attributes":{"NOTE":["ISSUED","-","AS BUILT"],"NOTE#1":"A"}`. A tag defined once maps to its one value, as above. Zach decided this on 2026-11-13 (#99).

## Blocks

`agentdrafter query blocks <source> [--sheet <sheet>]` lists, for each sheet, the blocks it inserts, in the order it first inserts each, with the IDs of the inserts. `count` is how many inserts there are.

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001","DEMO-002","DEMO-003"],"message":"3 inserts of 1 block on the 3 sheets looked at: TITLE_BLOCK","kind":"blocks","count":3,"blocks":[{"sheet":"DEMO-001","blocks":[{"block":"TITLE_BLOCK","inserts":["DEMO-001/objects[2]"]}]},"..."]}}
```

## Measure

`agentdrafter query measure <source> (--from <id> | --from-name <name>) [--from-point <point>] (--to <id> | --to-name <name>) [--to-point <point>] [--sheet <sheet>]` gives the distance from a point of one object to a point of another, in the declared unit, and how far apart they are along X, Y and Z (`delta`). Both objects are on one sheet: two sheets are two drawings.

An object is named by its ID or by its name. A name is given once on a sheet, and a template's names are on every sheet built from it, so a name found on more than one of the sheets looked at needs `--sheet`.

**An end that matches nothing is not an error.** When no object on the sheets looked at has the ID or name an end gives, the reply is `ok`, with no `distance` and no `delta`, the end that matched, and a `message` naming the end that matched nothing:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001","DEMO-002","DEMO-003"],"message":"nothing matches: no object on the 3 sheets looked at has the ID `DEMO-001/nope`, the end to measure from","kind":"measure","to":{"sheet":"DEMO-001","id":"DEMO-001/border","type":"polyline","point":"points[0]","at":[0.5,0.5,0.0]}}}
```

A point is named as its key in the sheet description. Without one, the object's own point is used, the first in this table:

| Object | Points |
|---|---|
| `insert`, `text`, `mtext`, `table` | `position` |
| `line` | `from`, `to` |
| `polyline` | `points[0]`, `points[1]`, ... |
| `circle`, `arc`, `ellipse` | `center` |
| `spline` | `fit_points[0]`, ... or `control_points[0]`, ... |
| `hatch` | none: measure from an object at its boundary |
| `donut` | `center` |
| `solid` | `corners[0]`, `corners[1]`, ... |

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-001","DEMO-002","DEMO-003"],"message":"18.179658962697843 in from the position of `DEMO-001/heading` to the position of `DEMO-001/objects[2]`, on sheet `DEMO-001`","kind":"measure","distance":18.179658962697843,"delta":[15.5,-9.5,0.0],"from":{"sheet":"DEMO-001","id":"DEMO-001/heading","type":"text","point":"position","at":[1.0,10.0,0.0]},"to":{"sheet":"DEMO-001","id":"DEMO-001/objects[2]","name":"title-block","type":"insert","point":"position","at":[16.5,0.5,0.0]}}}
```

## Layers

`agentdrafter query layers <set | sheet-description> [--sheet <sheet>]` lists each sheet's layers: layer `0` first, as every drawing has it, then the sheet's own in order, those its block library brings among them. Each gives its `name`, `color`, `linetype` and `lineweight`, its states where they are not AutoCAD's defaults (`on`, `frozen`, `locked`, `plot`) and its `description` if it has one, as the sheet description writes them, and `objects`: how many of the sheet's objects sit on it, its inserts among them, not counting the objects of a block. The message names every layer that is off, frozen or does not plot.

```json
{"kind": "layers", "count": 2, "layers": [{"sheet": "frame-plate", "layers": [{"name": "0", "color": 7, "linetype": "Continuous", "objects": 1}, {"name": "trim", "color": 8, "linetype": "Continuous", "plot": false, "objects": 2}]}]}
```

## Layout

`agentdrafter query layout <set | sheet-description> [--sheet <sheet>]` gives each sheet's [layout](sheet-description.md#layouts-and-viewports), or, for a sheet drawn in [several](sheet-description.md#several-layouts), each of its layouts in the order of their tabs, one entry each: its name, or none for a sheet drawn in model space alone, its `paper`, its `page_setup` as the DWG holds it (the plotter, AutoCAD's name of the paper, the plot area, the plot scale and the plot style), how many of the sheet's objects are in its `paper_space`, its viewports among them, and in the `model_space` its sheet's layouts share, `model_extents`, the box round what model space draws at full size, and its `viewports` in order. Each viewport gives its ID, name and where it is written, its `center`, `width`, `height` and `corners` on the paper, its `view_center`, its `scale` and `ratio`, such as `1:4`, whether it is `locked`, its `clip`, the [boundary](sheet-description.md#clipped-viewports) it is clipped to, if any, its `frozen_layers`, the [layers frozen in it](sheet-description.md#layers-frozen-in-one-viewport), if any, its `layer`, and `view`, the part of model space it shows, by its corners in model space. `count` is how many viewports the sheets hold.

```json
{"kind":"layout","count":3,"layouts":[{"sheet":"layout","layout":"Backplane","paper":{"name":"ISO A3 landscape","width":420.0,"height":297.0},"page_setup":{"plotter":"DWG To PDF.pc3","paper":"ISO_full_bleed_A3_(420.00_x_297.00_MM)","area":"layout","scale":"1:1","plot_style":"monochrome.ctb"},"paper_space":15,"model_space":34,"model_extents":{"min":[0.0,-45.0],"max":[645.0,800.0],"width":645.0,"height":845.0},"viewports":[{"id":"layout/view-a","...":"...","scale":0.25,"ratio":"1:4","locked":false,"view":[[-60.0,-120.0],[660.0,880.0]]},"..."]}]}
```

## Extents

`agentdrafter query extents <source> [--sheet <sheet>] [--fonts <folder>]...` gives, for each sheet, the box round everything it draws, seen from above, as AutoCAD's extents are: `min` and `max` corners, `width` and `height`, in the declared unit, or `null` for a sheet that draws nothing. The extents follow the geometry, not the dashes a linetype lays along it: a circle of radius 10 drawn with a sparse dashed linetype spans 20 by 20. The sheet is drawn through the drawing pipeline that draws the PNG, with every layer continuous, so text counts as its font draws it, and the reply lists, under `font_substitutions`, each text style whose font was not in a font folder and the look-alike its text was measured with. Curves are drawn as chords within a millionth of the sheet's larger side, so how far a curve reaches is found to within that; straight lines are exact. A sheet drawn in a [layout](sheet-description.md#layouts-and-viewports) draws what it prints, its paper space and model space through its viewports, cut at their edges, so its extents are on the paper, its first layout's on a sheet drawn in several; [`layout`](#layout) gives model space's. The demo set's sheets draw within their trim line:

```json
{"ok":true,"result":{"source":"examples/demo-set","units":"in","sheets":["DEMO-003"],"message":"sheet `DEMO-003` draws within 17 by 11 in, from [0, 0] to [17, 11]","kind":"extents","count":1,"extents":[{"sheet":"DEMO-003","extents":{"min":[0.0,0.0],"max":[17.0,11.0],"width":17.0,"height":11.0}}],"font_substitutions":[{"style":"notes","font":"romans.shx","used":"Hershey Roman Simplex","why":"no font folder is set"},"..."]}}
```

## Snap

`agentdrafter query snap <source> (--id <id> | --name <name>) [--snap <snap>] [--which <which>] [--with <id> | --with-name <name>] [--from <id> | --from-name <name> --from-snap <snap> [--from-which <which>] [--from-with <id> | --from-with-name <name>]] [--sheet <sheet>] [--fonts <folder>]...` gives the points AutoCAD's [object snaps](../CONTEXT.md) find on an object, each in the declared unit: its `snap`, which of the object's points it is (`which`), and where it is (`at`). The object is named by its ID or its name, as an end of a [measure](#measure) is, with `--sheet` when its name is on more than one of the sheets looked at. Without `--snap`, every snap of the object by itself: endpoint, midpoint, center, quadrant, insertion and node. Everything is seen from above, as AutoCAD's snaps are in its top view, and every point keeps its height.

**A snapped point is named so it can be asked for again.** Its `which` is its place in the object, such as `points[2]`, `segments[1]` or `objects[3].to`, and the same object, `snap` and `which` give the same point. A point the object has once, with no place of its own, such as a line's midpoint or a circle's centre, has no `which` and needs none. In JSON a snap is written `{"kind": "snap", "id": "lugs/strip", "snap": "endpoint", "which": "objects[0].objects[1].to"}`, with `with` and `from` as below; the same mapping, without `kind`, is the [snapped point](change-sets.md#snapped-points) an operation takes wherever it takes a point.

| Object | `endpoint` | `midpoint` | `center` | `quadrant` | `insertion` | `node` |
|---|---|---|---|---|---|---|
| `line` | `from`, `to` | its middle | | | | |
| `polyline` | `points[0]`, ... | `segments[i]`, the segment from point `i`, along its arc when it is one; a closed polyline's last runs back to its first point | `segments[i]` of each arc segment | `segments[i].0`, `.90`, `.180`, `.270` of each arc segment, those on it | | |
| `arc` | `start`, `end` | along its arc | its centre | `0`, `90`, `180`, `270`, those on it | | |
| `circle` | | | its centre | `0`, `90`, `180`, `270` | | |
| `ellipse` | an elliptical arc's `start`, `end` | halfway along an elliptical arc | its centre | the ends of its axes, `0` and `180` on the major axis, `90` and `270` on the minor, those on it | | |
| `spline` | `fit_points[0]` and its last, or `control_points[0]` and its last | halfway along it | | | | |
| `donut` | `points[0]`, `points[1]`, the ends of its two half circles | `segments[0]`, `segments[1]` | its centre | `0`, `90`, `180`, `270` on its middle circle | | |
| `solid` | `corners[0]`, ... | `segments[i]`, round its outline, the last back to the first | | | | |
| `text`, `attdef` | | | | | its position | where its baseline starts, when it is justified other than left |
| `mtext` | | | | | its position | |
| `table` | `borders[i].from`, `borders[i].to` | `borders[i]` | | | its position | |
| `leader` | `points[0]`, ..., and `hook`, where AutoCAD's hook starts | `segments[i]` of what it draws, its hook among them | | | its annotation's | |
| `mleader` | `leaders[i][j]`, each point of each leader line as it is laid out, and `landing`, the landing's far end | | | | its content's | |
| `dimension` | `lines[i].from`, `lines[i].to`, the ends of each line of its block as it is laid out, and `arcs[i].from`, `arcs[i].to`, of each arc of an angular one's | `lines[i]`, `arcs[i]` | `arcs[i]`, an arc's centre, the dimension's vertex | `arcs[i].0`, `.90`, `.180`, `.270` of each arc, those on it | the middle of its text | `defpoints[i]`, its definition points, as AutoCAD's node snap finds them |
| `viewport` | `corners[0]`, ..., its frame's, from its lower left corner anticlockwise | `segments[i]`, the middles of its frame's sides | its middle | | | |
| clipped `viewport` | as a closed `polyline`'s, `points[i]`, round a [boundary](sheet-description.md#clipped-viewports) of points | as a closed `polyline`'s | as a closed `polyline`'s, or a circle's centre | as a closed `polyline`'s, or a circle's `0`, `90`, `180`, `270` | | |
| `hatch` | none: AutoCAD snaps to no hatch while `OSNAPHATCH` is 0, as it is unless set | | | | | |

A clipped viewport is snapped to on the boundary it is drawn as, not on the frame it is clipped from, as AutoCAD snaps to its clipping polyline or circle. A table's `borders` are its lines as AutoCAD draws them: each row's boundary from left to right, top to bottom, then each column's from top to bottom, left to right, each running whole until a merged cell breaks it. A multileader is snapped to only at the ends of its lines and its content, as AutoCAD 2026 found no midpoint and no nearest point along it.

**An insert snaps to its block's own geometry.** Every object of the block, and of each block nested in it, is snapped to where the insert places it, moved, scaled and turned, its quadrants turned with it. Its `which` is its place in the block, `objects[3].to`, and in a block nested in it, `objects[0].objects[1].to`; `blocks` lists the blocks that place passes into, the insert's first. The insert's own point is its insertion point `position`; each visible attribute's, and each text's of its blocks, is the insertion point of the definition's place, `objects[2]`. An insert nested in a block has no insertion point of its own: AutoCAD's insertion snap on it finds the point of the insert that holds it. The made-up [example](../examples/snaps/lugs.yaml) inserts a strip of two lugs, turned 90° and twice as large, and draws a wire to the end of the first lug's stub:

```json
{"ok":true,"result":{"source":"examples/snaps/lugs.yaml","units":"mm","sheets":["lugs"],"message":"the endpoint of the insert `lugs/strip`, on sheet `lugs`: [50, 52, 0]","kind":"snap","count":1,"object":{"sheet":"lugs","id":"lugs/strip","type":"insert"},"points":[{"snap":"endpoint","which":"objects[0].objects[1].to","blocks":["STRIP","LUG"],"at":[50.0,52.0,0.0]}]}}
```

**Between two objects.** `intersection` and `apparent_intersection` take the other object as `--with` (`with` in JSON). They list every point where the two cross, inserts' block geometry among them, in the order they lie along the first object, named `0`, `1`, ... when there are several; `with_part` is the place in the other object of what it crosses there, and `part` the place in the first. An intersection is where the two meet in space; an apparent intersection where they cross seen from above, whatever their heights, at the first object's height. A line along Z, seen end on, meets what passes through the point it is seen as. Collinear lines do not intersect, whether they overlap or only meet end to end, as AutoCAD 2026's intersection snap found none between them: where they meet is an endpoint of each. Where two curves touch rather than cross, they meet once. The wire crosses the strip's frame, then meets the lug's stub at its end:

```json
{"ok":true,"result":{"source":"examples/snaps/lugs.yaml","units":"mm","sheets":["lugs"],"message":"2 intersections of the line `lugs/wire` with the insert `lugs/strip`, on sheet `lugs`","kind":"snap","count":2,"object":{"sheet":"lugs","id":"lugs/wire","type":"line"},"with":{"sheet":"lugs","id":"lugs/strip","type":"insert"},"points":[{"snap":"intersection","which":"0","with_part":"objects[2].segments[2]","at":[28.0,57.86666666666667,0.0]},{"snap":"intersection","which":"1","with_part":"objects[0].objects[1]","at":[50.0,52.0,0.0]}]}}
```

`perpendicular`, `tangent` and `nearest` are taken from a point: `--from`, a snap of another object, given as a snap is (`from` in JSON), which must name one point; a `from` that is itself an intersection or an apparent intersection takes its own other object as `--from-with` or `--from-with-name` (its `with` in JSON). They list each point, nearest the point taken from first, named `0`, `1`, ... when there are several. A perpendicular to a line or a straight segment reaches past its ends, as AutoCAD's does, and one to an arc or a curve is on it. Every foot is listed: on a curve with several, such as a spline, AutoCAD 2026's perpendicular snap found only one of them wherever it was picked. A tangent touches an arc, a circle, an ellipse or a spline, from a point outside it. The nearest point is the one point of the object nearest it. The reply gives the point taken from under `from`:

```json
{"ok":true,"result":{"source":"examples/snaps/lugs.yaml","units":"mm","sheets":["lugs"],"message":"the nearest point of the line `lugs/wire`, on sheet `lugs`: [38.04979253112033, 55.18672199170125, 0]","kind":"snap","count":1,"object":{"sheet":"lugs","id":"lugs/wire","type":"line"},"from":{"sheet":"lugs","id":"lugs/strip","type":"insert","snap":"center","which":"objects[1].objects[0]","at":[34.0,40.0,0.0]},"points":[{"snap":"nearest","at":[38.04979253112033,55.18672199170125,0.0]}]}}
```

**A whole sheet's snaps at once.** `display-list --snaps` gives every snap of the objects a sheet draws itself, its own and each intersection between two of them, named as `query snap` names them, for the viewer to find the one nearest the cursor ([viewer.md](viewer.md#object-snaps)). It leaves out what the sheet does not draw: an object on a layer that is off or frozen, one in an insert's block among them, which `query snap`, asked of an object by its ID, still answers for.

**A snap that finds nothing is not an error.** An object, a `with` or a `from` that no object on the sheets looked at has the ID or name of, and a snap the object has none of, such as a line's centre, answer `ok`, with `count` 0 and a `message` saying so. The two objects of a snap are on one sheet.

## Geometry

`agentdrafter query geometry <source>` with the options of [`find`](#find) gives each matching object's length, area, extents and angles, as AutoCAD's LIST and AREA give them, in the declared unit and degrees, and under `without` what it has none of:

| Object | `length` | `area` | `extents` | Angles |
|---|---|---|---|---|
| `line` | in space | | its ends | `angle`, in the XY plane, anticlockwise from the X axis |
| `polyline` | along its segments and arcs | when it is closed | round its points and arcs, and round the band it fills when it has a width | |
| `arc` | along it | | | `start_angle`, `end_angle` and `total_angle`, the angle it sweeps |
| `circle` | its circumference | yes | yes | |
| `ellipse` | its perimeter, or an elliptical arc's length | a whole ellipse's | yes | |
| `spline` | along it | when its ends meet | yes | |
| `donut` | its middle circle's | its middle circle's | its outside circle's | |
| `solid` | | yes | its corners | |
| `hatch` | | what it fills: a loop inside an odd number of others is a hole | its boundary's | |
| `dimension` | | | round its extension lines, dimension line or arc, tails, landing and leader | `measurement`, what it measures, unrounded, as AutoCAD keeps it in the dimension: a length in the declared unit, or an angular dimension's angle in degrees, which AutoCAD keeps in radians; and `text`, what its text shows, a stacked fraction written `N/D` |
| anything else | | | | |

Lengths and areas are measured along a polyline's middle, whatever its width, as AutoCAD's AREA measures a wide polyline, and a donut is the polyline AutoCAD draws it as. Extents are the box round the object's own geometry, `min` and `max` `[x, y, z]`, worked out exactly from its curves; text, inserts, tables and leaders have none here, as their extents are their fonts', which [`extents`](#extents) gives for the sheet. An ellipse's and a spline's lengths and areas are integrated to within a ten-billionth.

```json
{"ok":true,"result":{"source":"examples/snaps/lugs.yaml","units":"mm","sheets":["lugs"],"message":"a polyline `lugs/plate`, on sheet `lugs`, is 135.70796326794897 mm long and encloses 1178.539816339745 square mm","kind":"geometry","count":1,"objects":[{"sheet":"lugs","id":"lugs/plate","type":"polyline","length":135.70796326794897,"area":1178.539816339745,"extents":{"min":[0.0,0.0,0.0],"max":[40.0,30.0,0.0]},"without":["angle"]}]}}
```

**What AutoCAD 2026 gave.** On a made-up sheet of every kind of object, with an insert scaled, turned and nested, AutoCAD 2026's own snaps (`osnap`) and measures (`vlax-curve`, AREA) gave every point and value above to within a millionth. Where AutoCAD approximates, the query does not: AutoCAD's own length of a spline by control points, and the midpoint it snaps to on a spline, which it places by that length, were up to 1.6e-5 of the length away from the curve's exact length, which the same curve summed along 50,000 of AutoCAD's own points matched; and its extents, which it takes from the curves as it draws them, fell short of an arc's reach by up to 2.4e-4 mm.

## Errors

Errors take the shape and codes a sheet description's do ([Errors](sheet-description.md#errors)); `field` is the place in the request, such as `query.from.id`.

| Code | In a query it means |
|---|---|
| `bad_value` | A measure's ends or a snap's objects are on two sheets, a name to measure from or to snap to is given on more than one of the sheets looked at, or the object has no point of its own, such as a hatch. A snap gives `which` without `snap`, `with` to a snap that is not an intersection or `from` to one that is not perpendicular, tangent or nearest, or leaves them out of one that is; a `from` has no `snap`, names no point of its object, or names several without `which`, the message listing them. |
| `unknown_name` | A sheet the set does not hold, a point the object does not have, or a snap's `which` the object has no point of; the message lists the choices. |
| `usage` | A measure's end or a snap's object names it by both ID and name, or by neither, a snap is not one of AutoCAD's, or the command was called wrongly. |
| any other | A sheet that cannot be read, with its own error in its file, such as `duplicate_id`. |

## How it works

`crates/agentdrafter-core/src/query.rs` reads the set's settings and rows and each sheet's record (`description/records.rs`, and `description/template.rs` for a sheet of a set), selects and measures, and draws a sheet's extents with `draw::draw_sheet`, and what each object draws for a box, in its own space, with `draw::draw_objects`. `query/layout.rs` answers `layout`. `query/snap.rs` works out a snap and an object's geometry on the records, from `geometry.rs`, which reads each object as the curves it is made of, an insert's block's placed by the one placement rule, and the points its own snaps find; `geometry/curve.rs` holds the curves' arithmetic, `geometry/snap.rs` the snaps between two objects and where two objects touch, and `geometry/measure.rs` the lengths, areas and extents. `query/snap.rs`'s `one_point` is the one point a change set's snapped point names. A polyline's arcs come from their bulges as the drawing pipeline's do (`draw/polyline.rs`), and a spline is the B-spline it draws (`draw/curves.rs`).

## What is not offered yet

- Querying DWGs, built or someone else's. Every DWG a build writes carries each object's ID, ready for it.
- Patterns other than a whole value or a part of it, such as wildcards.
- A distance from a point given by coordinates. A box is the only coordinates a query takes.
- AutoCAD's other selections by place, such as a fence or a polygon window, and AutoCAD's `OSNAPHATCH`, a hatch's boundary as geometry to touch.
- AutoCAD's extended intersections and apparent intersections, where two objects would cross if drawn on, and a snap picked by a cursor's place: every point of the kind is listed, named.
- A node on multiline text, which AutoCAD 2026 finds at a point of its box that moves with its attachment, and snaps into a multileader's block content or a table's text.
- The geometric extents of text, inserts, tables and leaders: [`extents`](#extents) gives what a sheet draws, text as its font draws it.
