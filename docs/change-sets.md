# Change sets

A change set is the operations an agent sends together to change a drawing set (see [`CONTEXT.md`](../CONTEXT.md)): typed operations such as `insert_block` or `set_attribute`, each with its AutoCAD command name as an alias. It lands at once on the working revision. Each sheet takes its part fully or not at all (ADR-0005), and a set where some sheets failed is marked inconsistent until they are fixed. An agent can list the operations with their schemas, and can ask for a dry run.

`agentdrafter change <drawing-set-folder> <change-set.json | -> [--dry-run] [--fonts <folder>]...` applies a change set written as JSON, from a file or from standard input, and prints one line of JSON. It is a thin layer over the core's `change_with`, which takes the change set and returns the reply as plain data (ADR-0007). `agentdrafter operations` lists the operations, and `agentdrafter schema change-set` prints the change set's JSON Schema.

## Where a change lands

Zach decided on 2026-11-11:

1. **The working revision is the set's own files, edited in place.** A change set writes the set's records, its templates and its rows files, directly, and builds again only the DWGs of the sheets it touched. History is the [journal](journal.md)'s, and git's too when the set is in a repository. A DWG is replaced only when it carries AgentDrafter's build mark, as `build --overwrite` replaces one; any other file at a sheet's path may be an original, so that sheet fails and the file is not touched. Revising an existing DWG, an original into a revision, is not part of change sets yet.
2. **An edit to an object that comes from a template changes the template.** Every sheet built from it follows, and the reply lists the sheets it reaches.
3. **An object added to one sheet goes in that sheet's row**, under the row's [`objects`](drawing-set.md#a-rows-own-objects). The template stays shared.
4. **A template edit is all or nothing across its sheets.** It lands only if every sheet built from that template takes its whole part. Otherwise the template, those sheets' rows and their DWGs stay as they were, byte for byte. Sheets built from other templates still land. ADR-0005 stands.
5. **A value the template takes from the row is set in that sheet's row.** Setting an attribute, a text's contents, or the text of a leader's content or annotation, whose template value is `{row: NAME}` writes that sheet's row value `NAME`, on that sheet alone, and the reply names every place on the sheet the value fills.

Zach decided on 2026-11-19 (#102) how an edit lands on a value the template joins from a [list of parts](drawing-set.md#a-rows-value-inside-other-text), plain text and values from the row, such as `['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}]`:

6. **An edit is matched to the parts.** A `set_attribute`, or a `change` of a text's or an mtext's contents, on such a value takes the whole new text, which is matched against the parts' fixed text to find the text of each row value they hold. `PANEL P2 SLOT 4` sets that sheet's row values `PANEL` to `P2` and `SLOT` to `4`, on that sheet alone, as decision 5 sets one, and the reply's `row_values` names each, with every place on the sheet it fills. A value the parts hold twice must take the same text in both. An mtext's text is matched as it shows: the line breaks it ends with are dropped and CR LF is a line break, as LF is, in the parts and in the text sent, so the text quick properties show, sent back, matches.
   - It is refused with `bad_value`, naming the pattern, when the fixed text differs, or when the text splits more than one way, such as two row values side by side with no fixed text between them, or a value that holds a fixed part's own text; the error gives two of the splits. The fixed text is checked first, and each value ends only where the fixed text after it begins, so a text that cannot match is refused at once; one that puts many values side by side and splits too many ways to try is refused too, asking for `row_value`. The limit can refuse even a text that splits one way only, such as eight values side by side with one of them again after a fixed part; `row_value` sets each value whatever the text, so it always works. Nothing of the sheet is written.
   - `row_value: NAME` on the operation sets that one part alone: the text sent is that row value's, and nothing is matched. `NAME` is written as a row's value's name is, ASCII letters, digits, `_` and `-`. A name the parts do not hold is refused with `unknown_name`, listing those they do. On a whole value, `{row: NAME}`, it may name `NAME` and changes nothing; on a value the template does not take from the row it is refused with `bad_value`.
   - A change of nothing but contents the template takes from the row, whole or in parts, reaches only that sheet's row, so its refusal, such as contents sent as `null`, fails that sheet alone and the change set's other sheets still land.
   - When the joined text a sheet builds is refused, such as a one-line text given a line break, the error names the operation whose row value made it so: each operation that set a value the text joins is tried alone, and the one that fails alone is named. When none or more than one does, each that set a value the text joins is named.

A set whose rows are in [several rows files](drawing-set.md#several-rows-files) is changed as one: each edit to a row, a row value or a row's own object, is written to the rows file that row is in, the operation's `record` names that file, and a change set may land on sheets of several files at once, each sheet fully or not at all. An operation's `record` names one file, even when one edit writes both a row's rows file and a template, such as a `change` of a template text's `contents`, which sets the sheet's row value, and its `height`, which edits the template. It names the template when the template is built into more than one sheet, and the rows file when the template serves that one sheet alone: the same change on `SCHED-201/heading` in `examples/two-kinds` names `rows/schedules.yaml`. For every file a change set wrote, read the reply's top-level `records`, which lists each, every rows file among them. An undo puts back every rows file the change set wrote, byte for byte ([the journal](journal.md#undo)). No operation adds a row or a sheet, so none names a rows file.

A sheet's template is the file its row's path names, as the file system resolves it, so rows that reach one file by different paths, such as `templates/t.yaml` and `templates/T.yaml` where the file system ignores case, or a link and the file it links to, share one template, and rows whose paths name two files, such as `../t.yaml` and `../../../t.yaml`, do not. A change set writes a template through one path, so it does not edit a template the rows reach by more than one: such an edit refuses the change set, naming the paths, until every row writes one path to it. An edit to one sheet's row is no edit to its template, and lands.

## A change set

```json
{"operations": [
  {"op": "move", "target": {"id": "DEMO-001/border"}, "by": [0.1, 0, 0]},
  {"op": "set_attribute", "target": {"name": "title-block", "sheet": "DEMO-002"}, "tag": "REV", "value": "A"},
  {"op": "add_circle", "sheet": "DEMO-003", "center": [5, 5, 0], "radius": 0.5, "layer": "notes"}
]}
```

| Key | Required | What it is |
|---|---|---|
| `operations` | yes | The operations, applied in order. Each is a mapping that names its operation under `op`, by its name or its AutoCAD alias in any case: `move`, `MOVE` and `Move` are one operation. |
| `fixes` | no | The sheets of the set's [inconsistent mark](#an-inconsistent-set) this change set fixes, by name. |

**Naming the object.** An operation that edits an object names it under `target`, by its [ID](sheet-description.md#ids-and-names), `{"id": "DEMO-001/border"}`, or by its name, `{"name": "title-block", "sheet": "DEMO-002"}`, never by coordinates. An ID and a name are matched in any case. A name is given once on a sheet, and a template's names are on every sheet built from it, so `sheet` may be left out only when the name is on one sheet of the set. An object an earlier operation of the change set added is named by the ID it was given.

**Lengths** are in the set's declared unit and may carry their own, as in a sheet description: `"radius": "6 mm"` in a set in inches is converted. An object's keys are written as a sheet description writes them, as JSON: `[0, 0, 0]` for a point, `"0.25 in"` for a length with its unit.

### Snapped points

Wherever an operation takes a point, it may take a snapped point instead, `{"snap": {...}}`: one of AutoCAD's [object snaps](query.md#snap), written as `query snap` takes it, its object by `id` or `name`, its `snap`, and `which` of its points, with `with` or `from` where the snap takes them. An add takes one for each point of the object it adds, a line's `from` and `to`, a polyline's points, a centre, a text's or an insert's `position`, a hatch's boundary points, each point of a multileader's leader lines and an older leader's, a dimension's points, centre, point and location, and so does `add_leader_line` for each of its points, `move` and `copy` for their `from` and `to`, a polar `array` for its `center` and `base_point`, `rotate` and `scale` for their `base_point`, and `mirror` for its `first_point` and `second_point`. A direction, such as an ellipse's `major_axis`, is not a point and takes none, and neither does a block's definition, whose points are in the block's own coordinates.

```json
{"op": "add_line", "sheet": "SNAP-001", "layer": "wires",
 "from": {"snap": {"id": "SNAP-001/strip", "snap": "endpoint", "which": "objects[0].objects[1].to"}},
 "to": {"snap": {"id": "SNAP-001/ring", "snap": "quadrant", "which": "90"}}}
```

- **It names one point.** Its `snap` is given, and `which` when the snap finds more than one point; one that finds none, or several without `which`, is refused at its key, `bad_value` at `operations[0].to.snap` or `operations[0].to.snap.which`, the second listing each point with where it is, as `query snap`'s reply names them. An object the sheet does not hold is `unknown_name` at its `id` or `name`, listing the sheet's IDs or names. These fail the sheets the operation reaches, as a target that names nothing does. A snapped point that is wrong in itself, with no `snap`, a key a snap does not take, or `with` or `from` where its snap takes none, refuses the change set whole.
- **It is worked out when the operation applies**, on the sheet as it stands then: the operations before it in the change set have landed on it, so a line may run to a circle an earlier operation added or moved. The object an add adds is not on the sheet yet, and the object a move moves is where it was.
- **The record is written with the coordinates it names**, in the declared unit, as plain numbers: a sheet description holds points, never snaps. The point does not follow the object afterwards: moving the circle later leaves the line where it was.
- **On every sheet it reaches.** An operation on a template reaches every sheet built from it, and its snapped point is worked out on each. An ID names its object on the sheet it gives, which must be one the operation reaches, and the object of the same ID on each other sheet; a name, the object of that name on each. When the point is not the same on every sheet, as when it is on a row's own object, the operation is refused at its key, `bad_value`, naming two sheets and the point on each: a template's object is drawn through the same point on every sheet. A snap to an object only one of the sheets holds is `unknown_name` on another. Either holds back every sheet the template reaches, all or nothing (decision 4).

The operation's reply lists each snapped point under `snapped`: its key in the operation (`field`), the object (`id`, on the sheet its ID gives), the `snap`, `which` and where it is (`at`), the coordinates the record writes; its message says the same.

```json
{"index":0,"op":"add_line","ok":true,"message":"adds a line, `SNAP-001/line-1`, to sheet `SNAP-001`, in its row; `from` is the endpoint `objects[0].objects[1].to` of `SNAP-001/strip`, [50, 52, 0]; `to` is the quadrant `90` of `SNAP-001/ring`, [90, 68, 0]","after":"line: {from: [50, 52, 0], to: [90, 68, 0], layer: wires, id: line-1}","snapped":[{"field":"from","id":"SNAP-001/strip","snap":"endpoint","which":"objects[0].objects[1].to","at":[50.0,52.0,0.0]},{"field":"to","id":"SNAP-001/ring","snap":"quadrant","which":"90","at":[90.0,68.0,0.0]}],"...":"..."}
```

## The operations

| Operation | AutoCAD | What it does |
|---|---|---|
| `add_line` | `LINE` | Adds a line. |
| `add_polyline` | `PLINE` | Adds a polyline. |
| `add_arc` | `ARC` | Adds an arc. |
| `add_circle` | `CIRCLE` | Adds a circle. |
| `add_ellipse` | `ELLIPSE` | Adds an ellipse. |
| `add_spline` | `SPLINE` | Adds a spline. |
| `add_hatch` | `HATCH` | Adds a hatch. |
| `add_text` | `TEXT` | Adds a single line of text. |
| `add_mtext` | `MTEXT` | Adds multiline text. |
| `add_donut` | `DONUT` | Adds a donut. |
| `add_solid` | `SOLID` | Adds a 2D solid. |
| `insert_block` | `INSERT` | Adds an insert of a block from the block library. |
| `add_table` | `TABLE` | Adds a table. |
| `add_mleader` | `MLEADER` | Adds a multileader: leader lines from their arrows to a landing, and its text or block. |
| `add_leader` | `LEADER` | Adds an older leader, with its annotation if it has one. |
| `add_linear_dimension` | `DIMLINEAR` | Adds a linear dimension: written as a `dimension` with `kind: linear`, which the operation gives it. |
| `add_aligned_dimension` | `DIMALIGNED` | Adds an aligned dimension. |
| `add_radius_dimension` | `DIMRADIUS` | Adds a radius dimension. |
| `add_diameter_dimension` | `DIMDIAMETER` | Adds a diameter dimension. |
| `add_angular_dimension` | `DIMANGULAR` | Adds an angular dimension between two lines. |
| `add_three_point_angular_dimension` | none | Adds an angular dimension at a vertex between two points, as DIMANGULAR measures an arc, a circle or three points. |
| `add_ordinate_dimension` | `DIMORDINATE` | Adds an ordinate dimension. |
| `add_viewport` | `MVIEW` | Adds a viewport to a sheet's layout: a window on the paper onto model space, at a scale. |
| `add_baseline_dimension` | `DIMBASELINE` | Adds a dimension from another's first extension line, its line further out. |
| `add_continued_dimension` | `DIMCONTINUE` | Adds a dimension from another's second extension line, on its line. |
| `change` | `CHANGE` | Sets keys of an object. |
| `change_viewport` | none | Sets a viewport's scale, its view centre or its display lock. |
| `clip_viewport` | `VPCLIP` | Clips a viewport to a boundary, or takes its boundary away. |
| `set_frozen_layers` | `VPLAYER` | Sets the layers frozen in one viewport. |
| `move` | `MOVE` | Moves an object. |
| `delete` | `ERASE` | Deletes an object. |
| `copy` | `COPY` | Copies objects, once by an offset or to several. |
| `array` | `ARRAY`, `ARRAYRECT`, `ARRAYPOLAR`, `ARRAYPATH` | Copies objects in rows and columns, round a centre or along a path, as plain copies. |
| `rotate` | `ROTATE`, `RO` | Turns objects about a base point. |
| `scale` | `SCALE`, `SC` | Scales objects about a base point. |
| `mirror` | `MIRROR`, `MI` | Mirrors objects across a line, as mirrored copies or in place. |
| `set_attribute` | `ATTEDIT` | Sets one attribute value of an insert, naming one occurrence of a tag its block defines more than once. |
| `set_cell` | `TABLEDIT` | Sets keys of a table's cell: its text, style, height, justification or merge. |
| `add_row` | none | Adds a row to a table. |
| `delete_row` | none | Deletes a row of a table. |
| `add_column` | none | Adds a column to a table. |
| `delete_column` | none | Deletes a column of a table. |
| `add_leader_line` | `MLEADEREDIT` | Adds a leader line to a multileader. |
| `delete_leader_line` | none | Deletes a leader line of a multileader. |
| `add_layer`, `change_layer`, `rename_layer`, `delete_layer` | `LAYER`, none, `RENAME`, `LAYDEL` | Adds, changes, renames or deletes a layer of a template. |
| `add_linetype`, `change_linetype`, `rename_linetype`, `delete_linetype` | `LINETYPE`, none, none, none | The same for a linetype. |
| `add_text_style`, `change_text_style`, `rename_text_style`, `delete_text_style` | `STYLE`, none, none, none | The same for a text style. |
| `add_dimension_style`, `change_dimension_style`, `rename_dimension_style`, `delete_dimension_style` | `DIMSTYLE`, none, none, none | The same for a dimension style. |
| `add_block` | `BLOCK` | Adds a block to a block library. |
| `redefine_block` | `BEDIT` | Replaces a block's whole definition, and brings its inserts into line. |
| `attsync` | `ATTSYNC` | Brings the inserts of a block, or of every block of a library, into line with its definition. |
| `import_blocks` | `ADCENTER` | Imports a DWG's or a DXF's blocks into a block library. |

`agentdrafter operations` gives each with what it does and its JSON Schema (draft 2020-12), with the definitions it refers to, so an agent can check an operation before it sends it:

```json
{"ok":true,"result":{"how":"A change set is {\"operations\": [...]}: ...","operations":[{"name":"add_line","alias":"LINE","does":"Adds a line, written with the keys a line takes in a sheet description, ...","schema":{"type":"object","required":["op","from","to"],"...":"..."}},"..."]}}
```

**Adding.** Every object a sheet description holds has an operation that adds it. It takes the keys that object takes in a [sheet description](sheet-description.md#objects), `id`, `name`, `layer` and, on a sheet drawn in a layout, `space` among them, and where it goes: `sheet`, to add it to that sheet's row, or `template`, the template's path as the rows write it, to add it to the template and so to every sheet built from it. The object is drawn after the objects already in its list. An object added without an `id` is given one, its type and the first number free on the sheets it is added to, such as `line-1` or `insert-2`, so that its ID is fixed for its life; the reply gives its ID. A key sent as `null` is a [bare null](sheet-description.md#the-yaml-subset) (rule 8), JSON's form of `layer: null`: the record writes it so, and the sheet reads it as a sheet description does. On an optional text or path field it leaves the field out, so `"layer": null` puts the object on layer `0`, an attribute's value sent as `null` takes its definition's default, and an `id` sent as `null` is given one as above. On a required one, such as a text's `contents`, it is the `missing_key` error, and on any other key, such as `closed` or `radius`, it is an error as `closed: null` is in a sheet description; either fails the sheet, as a bad value does. An add to a template may give a text's or an mtext's `contents`, or the value of an attribute its block defines once, as a [list of parts](drawing-set.md#a-rows-value-inside-other-text), as the template writes it, and the add's schema takes it; an add to a sheet's row may not.

**`change`** takes `target` and `set`, a mapping from each key to set to its new value, written as a sheet description writes it; `null` takes an optional key away. It sets any key the object's type takes, and `name`. It does not set `id`, since an object's ID is fixed for its life, nor an insert's `attributes`, which `set_attribute` sets one at a time, nor the object's type. A text's or an mtext's `contents` whose template takes them from the row is set in the sheet's row (decision 5), matched to the parts when the template joins them from a list of parts (decision 6), and so is the text of a multileader's `content` or an older leader's `annotation` ([Leaders](#leaders)).

**`move`** takes `target` and `by`, `[dx, dy, dz]`, or `from`, a base point, and `to`, a second point, as AutoCAD's MOVE takes them, each a point or a snapped point: the object moves by `to` less `from`. Give `by`, or `from` and `to`; both is `usage`, and one of `from` and `to` alone `missing_key`. So a lug moves until its stub's end lands on a terminal:

```json
{"op": "move", "target": {"id": "SNAP-001/term"},
 "from": {"snap": {"id": "SNAP-001/term", "snap": "endpoint", "which": "objects[1].to"}},
 "to": {"snap": {"id": "SNAP-001/ring", "snap": "quadrant", "which": "0"}}}
```

Every point the object is drawn through moves: a line's ends, a polyline's points, a circle's, an arc's or an ellipse's centre, a spline's fit or control points, the points and edges of each loop of a hatch's boundary, and a patterned hatch's `origin`, the point its pattern is laid from, which a hatch that gave none takes as the distance moved, so its pattern moves with it, the position of a text, an mtext, an insert or a table, every point of each of a multileader's leader lines, an older leader's points, a dimension's points, centre, point and location, and a viewport's centre on the paper, its view moving with it; a leader's text moves with it, laid out again where its landing or end puts it. An arrow is placed at a point and does not follow an object it points at, and so is the arrow of a leader line `add_leader_line` adds; a [snapped point](#snapped-points) puts it on the object when the operation applies. A coordinate along an axis it does not move along is left as the record writes it; one it moves along is written in the declared unit.

**`delete`** takes `target`.

**Layers, linetypes and text styles.** Zach decided on 2026-11-13 how a change set edits a template's tables, which every sheet built from it shares. Each operation names its template by `template`, its path as the rows write it, or by `sheet`, a sheet built from it, and the entry by `name`, written exactly as the template writes it. Like any template edit it reaches every sheet built from the template, all or nothing, and the reply lists them. The block library's own layers, linetypes and text styles are not edited here, nor by the [block operations](#blocks).

- **`add_layer`** (`LAYER`), **`add_linetype`** (`LINETYPE`), **`add_text_style`** (`STYLE`) and **`add_dimension_style`** (`DIMSTYLE`) take `name` and the keys the entry takes in a [sheet description](sheet-description.md#layers), such as `color` and `plot` for a layer or `pattern` for a linetype. A name the template has already, in any case, or a drawing's own, is `duplicate_name`.
- **`change_layer`**, **`change_linetype`** and **`change_text_style`** take `name` and `set`, each key to its new value; `null` takes a key away. Layer `0` and `Standard` can be changed even where the template does not write them; `Standard` written for the first time takes the drawing's own font, `txt.shx`, unless `set` gives one. `Continuous`, `ByLayer` and `ByBlock` are AutoCAD's own linetypes and are not changed.
- **`rename_layer`** (`RENAME`), **`rename_linetype`** and **`rename_text_style`** take `name` and `to`, and rewrite everything that names the entry: every object's `layer`, `linetype` or `style`, in the template and in the rows of the sheets built from it, for a linetype, every layer's `linetype`, and for a layer, every viewport's `frozen_layers` that lists it, so that it stays frozen there. The reply counts the objects.
- **`delete_layer`** (`LAYDEL`), **`delete_linetype`**, **`delete_text_style`** and **`delete_dimension_style`** take `name`, and are refused with `in_use` while anything still uses the entry: the error lists the objects by ID, a viewport that freezes a layer among them, the layers that use a linetype and the dimension styles that use a text style. `ISO-25` and `Standard`, AutoCAD's own dimension styles, one of which every sheet draws its dimensions with unless they name another, are never renamed or deleted; `change_dimension_style` lists one in the template the first time it sets a key of it. Renaming a text style renames it in the dimension styles that draw with it, as renaming a linetype renames it in the layers. Move them, or delete them, first.
- Layer `0`, `Continuous` and `Standard`, which every drawing has, are never renamed or deleted (`bad_value`).

The table is written again whole, one entry to a line in braces, where the template wrote it; a table the template did not have goes just before its `objects`. A comment inside the table is lost; everything else in the file stays byte for byte. A table that only gains entries, written an entry to a line, keeps every line it had, comments among them, and takes each new entry after its last. A table written in braces with a comment among its entries is not written again: the operation fails `write_failed`, saying to write the table one entry to a line.

**A locked layer.** An object whose layer is `locked` is not edited: `change`, `move`, `delete` and `set_attribute` on it are refused with `locked`, naming the layer, as AutoCAD will not edit it. A row's value is not set either when any place on the sheet it fills, such as a heading, is on a locked layer; that refusal reaches only that sheet. An object can still be added onto a locked layer, as in AutoCAD. To edit one, unlock its layer first with `change_layer`, `set: {"locked": false}` or `null`, in the same change set or an earlier one.

**`set_attribute`** takes `target`, an insert, `tag` and `value`, one line of text, and `occurrence` when the insert's block defines the tag more than once. An attribute the insert leaves out is added. When the insert's template takes the attribute from the row, the sheet's row value is set instead (decision 5), and the reply's `row_value` names the value and every place on the sheet it fills: the demo set's `TITLE` fills both the title block's `TITLE` and the heading's contents. When the template joins it from a list of parts, the value sent is matched to the parts and each row value they hold is set, or `row_value` sets one alone (decision 6):

```json
{"op": "set_attribute", "target": {"id": "PNL-001/title-block"}, "tag": "TITLE", "value": "AREA 150 I/O PANEL P1"}
{"op": "set_attribute", "target": {"id": "PNL-001/title-block"}, "tag": "TITLE", "value": "P4", "row_value": "PANEL"}
```

A tag written in lowercase is taken in capitals, as AutoCAD keeps it, and the operation's message says so.

```json
{"op": "set_attribute", "target": {"id": "DEMO-001/revisions"}, "tag": "NOTE", "occurrence": 2, "value": "AS BUILT"}
```

Zach decided on 2026-11-13 (#99) how it names one of a tag the block defines more than once ([the block library](block-library.md#a-tag-defined-more-than-once)). `occurrence` counts from 1, in the order the block defines the tag, leaving out its constant definitions, which carry no value:

- When the tag repeats, `occurrence` is needed: without it the operation fails with `missing_key` at `occurrence`, listing each occurrence with its value, such as ``1 `ISSUED`, 2 `-` (its default)``. An occurrence past the last fails with `bad_value`, listing them too.
- When the tag does not repeat, `occurrence` is 1 or left out; any other is `bad_value`.
- It writes that item of the insert's list in its record, growing a shorter list with nulls, which take their defaults, and writing a list where the record gave none. An item the template takes from the row, `{row: NAME}`, sets the sheet's row value, as a whole attribute from the row does.

### Copy and array

Two operations copy objects, as AutoCAD's COPY and ARRAY do (#66). Zach decided on 2026-11-21 that **an array lands as plain copies**: each copy is an ordinary object with its own ID, and AutoCAD's associative array, one object to edit later, is not written; and that **a copied or arrayed insert keeps its source's attribute values**, as in AutoCAD, so the agent sets each copy's values afterwards, with `set_attribute` in the same change set.

Each names the objects it copies by `target`, one, or `targets`, a list of them, each by its ID or its name, all of one sheet; both is `usage`. Every object is copied to every place, in the order the targets are listed.

- **`copy`** (`COPY`) takes `by`, `[dx, dy, dz]`, for one copy; `offsets`, a list of them, a copy at each, as COPY's Multiple mode makes them; or `from`, a base point, and `to`, a second point, each a point or a [snapped point](#snapped-points), as `move` takes them. Give one of the three.
- **`array`** (`ARRAY`, `ARRAYRECT`, `ARRAYPOLAR`, `ARRAYPATH`) takes `kind`, `rectangular`, `polar` or `path`, and the keys of its kind. Sent as `ARRAYRECT`, `ARRAYPOLAR` or `ARRAYPATH`, in any case, `kind` may be left out, since the alias gives it; a `kind` that differs from the alias's is `usage`. A key of another kind is `unknown_key`.
  - **Rectangular:** `rows` and `columns`, each 1 when left out, at least one of them given, together at least 2; `row_spacing`, how far up each row is from the one before, and `column_spacing`, how far across each column is, each a length, negative to go down or left, needed when there is more than one row or column. The objects are row 0, column 0; the copies go row by row from there.
  - **Polar:** `center`, a point or a snapped point; `count`, the items, the objects among them, at least 2; `angle`, the angle they fill in degrees, a number, anticlockwise, negative clockwise, more than 0 and at most 360 either way, 360 when left out, so 360 is a whole turn and 0 fills nothing; and `rotate`, `true` when left out. A whole turn spaces the items evenly round it, 360 over `count`; less puts the last at its end, `angle` over `count` less 1, as ARRAYPOLAR fills an angle. Each copy turns about the centre with its place; with `rotate: false` each is moved, unturned, so that its `base_point`, a point or a snapped point, goes round, and that is the centre of the box round what the objects draw when it is left out, as `query extents` draws them, an insert's block and a text's letters among them, worked out on the sheet the operation names; an object that draws nothing gives no box, and is refused, asking for `base_point`. `base_point` is taken only with `rotate: false`.
  - **Path:** `path`, the object the array runs along, by its ID or its name, on the objects' sheet; `count`, at least 2; `spacing`, a length more than 0; and `align`, `true` when left out. A path is a **line, polyline, with or without arc segments, arc, circle, ellipse or spline**; any other object is `bad_value`. It runs as it is drawn: a line from its `from`, a polyline through its points in order, round its closing segment when closed, an arc from its start angle and a circle from angle 0, both anticlockwise, an ellipse anticlockwise from its start, and a spline from its first point. The objects stay where they are as item 0, wherever they are, at the path's start; each copy is moved by how far the path's point at its distance along is from the path's start, and, aligned, turned about that point by how far the path's direction there has turned from its direction at the start, so the objects ride along the path as they sit at its start, as ARRAYPATH aligns them to the first item. Without `spacing` the items are spaced evenly from the path's start to its end, or round a closed path, such as a circle, `count` to a turn; with it they are `spacing` apart along it, and a `count` whose items would run past the path's end is `bad_value`. A point where two segments meet takes the direction of the segment after it, and so does one within a billionth of the path's length of it, so that rounding in adding the segments' lengths up does not turn it the first segment's way. Distances along a path, `spacing` and the even spacing alike, are measured along it in space, its rises along Z among them; its direction, which an aligned copy turns with, is taken in the XY plane, since a copy turns about Z.

```json
{"op": "ARRAYRECT", "target": {"id": "TB-001/term"}, "columns": 40, "column_spacing": "6 mm"}
{"op": "array", "kind": "polar", "target": {"id": "GA-001/bolt"}, "center": {"snap": {"id": "GA-001/flange", "snap": "center"}}, "count": 8}
{"op": "arraypath", "targets": [{"id": "CB-001/tray"}, {"id": "CB-001/tag"}], "path": {"name": "route", "sheet": "CB-001"}, "count": 12, "spacing": 300}
{"op": "copy", "target": {"id": "LD-001/rung"}, "offsets": [[0, -20, 0], [0, -40, 0], [0, -60, 0]]}
```

**Each copy is an ordinary object.** It keeps every key its source has, written as its record writes it, an insert's attribute values among them, but its `name`, since a name is given once on a sheet; it is written with `copy_of`, the object it is a copy of (below); every point it is drawn through is moved, as `move` moves it, and, turned, every point turns about the turning point, and so does every direction it is drawn in: an arc's and a hatch's arc edges' angles, an ellipse's major axis, a text's, an mtext's and an insert's rotation, and a patterned hatch's angle and its `origin`, which a solid hatch does not take, so the pattern turns with the boundary. A copy turns as `rotate` turns an object ([Rotate, scale and mirror](#rotate-scale-and-mirror)), a multileader's, an older leader's and a dimension's text laid out again as its rules lay it; a table and a viewport are not turned, and an array that would turn one is `bad_value` at its `target`, asking for `rotate: false` or `align: false`. Unturned, every kind is copied. A coordinate the copy moves along, or any it turns, is written in the declared unit; one it neither moves along nor turns is left as its source writes it.

**Its ID.** A copy is given its source's `id` and the first number free on every sheet the copy reaches, counting from 1: `term-1`, `term-2` and so on, in the order the copies are made, skipping any taken, so a sheet that already has `ring-1` gives the template's ring's copy `ring-2` on every sheet. A source whose ID is its place, such as `objects[2]`, gives its copies its type, `circle-1`, as an add does, and so does one whose `id` is too long to take a number. The ID is fixed for the copy's life, as every ID is, and is in its record, so `query` finds it as it finds any object, and its DWG carries it.

**Where it is written.** A copy is written where its source is, as a [dimension added from another](#dimensions-from-others) is: a copy of a template's object in the template, so it is on every sheet built from it, all or nothing (decision 4), the reply's `sheets` lists them and its message says so, as `move`'s does, so a template warning can come before it, as it will in the drafting app's Copy tool; a copy of a row's own object in that row, on that sheet alone. Copies of a template's object and of a row's own object in one operation land in both, and the operation reaches every sheet of the template. A path of a row's own does not carry the objects of a template other sheets are built from too, since their copies would be on every one of those sheets and the path on one: that is `bad_value` at `path`. A template that one sheet alone is built from takes a path of that sheet's row. An object on a locked layer is not copied, as COPY does not select it (`locked`).

**Its values from the row.** Zach decided on 2026-11-21 that a copy gets a row value of its own only when one is set, not when it is copied. A copy of a template's object that takes a value from the row, `{row: NAME}`, whole or in a [list of parts](drawing-set.md#a-rows-value-inside-other-text), keeps its source's binding, so every copy shows what its source shows, as AutoCAD's copy keeps its source's values, and the rows gain nothing for copies nobody edits.

A copy is written with **`copy_of`**, the ID in its file of the first object of its copies: its source, or for a copy of a copy, its source's own `copy_of`. That is how the core knows which objects are copies of one another, its **copy family**: the first object, and the objects whose `copy_of` names it. It changes nothing drawn. The engine writes and keeps it: neither an add nor a `change` takes it, and the [sheet description](sheet-description.md#ids-and-names) reads it on any object.

When a `set_attribute`, a `change` of a text's or an mtext's contents, or another edit of a value from the row would change a row value a **copy**, an object written with `copy_of`, shares with the first object of its copies or another copy of it, that copy alone takes its own row value first: every place in it that takes the value takes a new one instead, and each row of the sheets built from the template is given it, set to what the copy showed on that sheet. The edit then sets it on the sheet edited, so the copy shows the new value there and what it showed before on the others, and the first object and the other copies are untouched. Of the values an edit matched to a [list of parts](#where-a-change-lands) sets, only those whose text changes are taken, for any object, a copy or not: the others are neither held to a locked layer, nor made a copy's own, nor written. Each is compared as the matcher reads it, so an mtext's last value written as a block of lines, which the text shows without its last line break, is unchanged when the text sent shows it the same. For example, a copied label made from `TITLE` and `TAG`, edited in its tag alone, takes its own `TAG` and still shares `TITLE`, so a later `TITLE` change reaches it. **The first object of the copies**, which has no `copy_of`, sets the shared value where it is, as every row value is set (decision 5): the copies not set since follow it, and so does every other place that shares it on purpose, as the demo set's `TITLE` fills the title block and the heading. So does a copy, for a value it shares with no object of its family. The new value's name is the value's and the copy's ID, joined by `-`, such as `TNUM-term-3`, with `_` for a character a value's name does not take and a number after it when the name is taken. Each row is written in its own style: in braces after its last value, or a value to a line at the end of the last value's line, after any comment on it, and before the first value written as a block of lines, such as `NOTES: |`, which a value after it would be read as part of. A row the new value cannot be written in fails the edit on the sheets it reaches, and nothing else. An edit that creates one writes the template and the rows of every sheet built from it, so it reaches all of them, all or nothing (decision 4); its reply lists the values it created under `created_row_values`, and its message says so; an edit held back lists none. `agentdrafter undo` takes the new value away with the rest.

**The reply** lists each copy under the operation's `copies`, in the order made: its `id`, on the sheet the operation names, the `source` it copies, where it went, `offset` for a copy, `row` and `column` for a rectangular array, `item`, counted from 0 with the objects item 0, for a polar or a path array, with `along`, how far along the path, for a path array; `by`, how far it was moved, and, when it was turned, `turned`, degrees anticlockwise, and `about`, the point, An operation held back, because a sheet it reaches failed, lists none: its copies never landed. Its `record` follows the rule for an edit that writes a row and a template together: the template when other sheets are built from it too, and the rows file when it serves that one sheet alone. The operation's message names the copies; the journal marks each added, on every sheet the copy is on, a template's object's on every sheet built from the template and a row's own object's on its sheet alone, where its ID is the first free too, so a [preview](#a-dry-run) and the viewer draw them as added.

```json
{"index":0,"op":"array","ok":true,"message":"arrays `TB-001/term` in a rectangular array of 1 row and 40 columns 6 apart (mm), making 39 copies, written in sheet `TB-001`'s row: `TB-001/term-1`, ...","sheets":["TB-001"],"record":"strip/rows.yaml","copies":[{"id":"TB-001/term-1","source":"TB-001/term","row":0,"column":1,"by":[6.0,0.0,0.0]},"..."]}
```

One operation makes at most 10,000 copies, its places times its objects; more is `bad_value`, and a `count`, `rows` or `columns` of 0, or 1 row and 1 column, which make no copy, is too. These, and the keys an operation does not take, refuse the change set whole; an object, a path or a snapped point the sheet does not hold fails the sheets the operation reaches, as for every edit.

### Rotate, scale and mirror

Three operations turn, scale and mirror objects, as AutoCAD's ROTATE, SCALE and MIRROR do (#67). Each names its objects by `target`, one, or `targets`, a list of them, each by its ID or its name, all of one sheet, as `copy` names them; an object named twice is `bad_value`, and objects of two sheets `bad_value` at the second. An object on a locked layer is not turned, scaled or mirrored (`locked`), as AutoCAD does not select it.

- **`rotate`** (`ROTATE`, `RO`) takes `base_point`, a point or a [snapped point](#snapped-points), and `angle`, degrees anticlockwise, negative clockwise, a number. It turns each object about the base point, in place.
- **`scale`** (`SCALE`, `SC`) takes `base_point` and `factor`, a number more than 0, less than 1 to make them smaller. It scales each object about the base point, in place, along Z too, as AutoCAD's SCALE does.
- **`mirror`** (`MIRROR`, `MI`) takes `first_point` and `second_point`, the two points of the mirror line, each a point or a snapped point, apart seen from above, and `erase_source`, `false` when left out. The mirror is upright through the line, so every point keeps its Z. As AutoCAD's MIRROR does by default, it keeps the objects and makes a mirrored copy of each, an ordinary object with an ID of its own and `copy_of`, written where its source is, exactly as [`copy`](#copy-and-array) gives them, listed under `copies` with `mirrored_across`, the line's two points. With `erase_source: true`, MIRROR's Erase source objects Yes, it mirrors the objects themselves instead, and each keeps its ID.

```json
{"op": "rotate", "targets": [{"id": "TB-001/strip"}, {"id": "TB-001/tag"}], "base_point": {"snap": {"id": "TB-001/strip", "snap": "insertion", "which": "position"}}, "angle": 90}
{"op": "SC", "target": {"id": "GA-001/detail"}, "base_point": [200, 150, 0], "factor": 2}
{"op": "mirror", "target": {"id": "JB-001/transmitter"}, "first_point": [300, 0, 0], "second_point": [300, 100, 0]}
```

**What moves with it.** Every point an object is drawn through lands where the transform puts it: a line's ends, a polyline's points, a centre, a spline's fit or control points, a solid's corners, each loop of a hatch's boundary, a text's, an mtext's, an insert's or a table's position, every point of a multileader's leader lines and an older leader's, and a dimension's points, lines, centre, point, feature and location. Every direction it is drawn in turns with it: an arc's angles and a hatch's arc edges', an ellipse's major axis, a text's, an mtext's and an insert's rotation, a patterned hatch's angle and `origin`, a linear dimension's `angle`, an ordinate dimension's `axis`, which a quarter turn or a mirror across a line at 45 degrees swaps between `x` and `y`, and a multileader's block content's rotation. An angular dimension whose location lies on one of its rays, which the reader takes as the sector anticlockwise from it, has its location put midway round that sector first, worked out with no sine or cosine, so that it measures the same sector transformed, which a mirror runs the other way. Scaled, every length it is drawn with is scaled: a radius, a polyline's widths, a donut's diameters, a text's and an mtext's height and an mtext's width, an insert's and a multileader's block content's `scale`, a patterned hatch's pattern `scale`, and a multileader's and an older leader's arrow size, gap, landing and text height, each written as its own when its style or the sheet's default gave it, so the leader scales whole. A key the object does not write, such as a text's `rotation` or an insert's `scale`, is written when the transform gives it a value other than its default. A coordinate, an angle or a length the transform leaves where it was is left as the record writes it; one it changes is written in the declared unit.

**What stays.** A multileader's and an older leader's text is laid out level again where its landing or end puts it, as every leader's is, and a dimension's text reads as its rules lay it out, from the bottom or the right. A dimension's text height and arrows stay as its style draws them, so a scaled dimension measures its new length at the same size, as AutoCAD's SCALE leaves one. A linetype's scale stays as it is.

**A mirror reads forwards.** A mirror keeps text reading forwards, as AutoCAD's MIRRTEXT 0 does: a text's or an mtext's position mirrors, and it reads along its mirrored direction, or that turned a half turn, whichever reads left to right or upwards, a rotation in [0, 90] or between 270 and 360, keeping its own `justify`. So text across a line up the sheet keeps its rotation's sense mirrored, 30 becoming 330, and horizontal text stays horizontal across any line up or across the sheet. An insert is mirrored, as AutoCAD leaves one: its `mirrored` key, which the DWG keeps as a negative X scale ([Inserts](sheet-description.md#inserts)), turns on, or off for one that was mirrored, which then takes `readable_attributes: true` so that its attributes keep reading forwards, and its rotation becomes twice the line's angle less its own, and a half turn, so a mirror across a line up the sheet leaves its rotation as it was mirrored and one across the sheet turns it a half turn. Its attributes sit where the mirror puts them and read forwards, by the same rule; what its block draws, a constant attribute among it, is mirrored, as in AutoCAD. A hatch's pattern keeps its direction, as AutoCAD's MIRRHATCH 0 keeps it: its boundary and its `origin` mirror and its `angle` stays. An arc's, an ellipse's and a hatch's arc edges' ends swap, since a mirror runs them the other way, and a polyline's `bulge` and `arc_angle` change sign.

**Exact for the common angles.** A turn by a multiple of 90 degrees, and a mirror across a line that runs across or up the sheet or at 45 degrees, land exactly, with no platform sine or cosine, so they write the same records on every system and add no difference of their own between systems' DWG bytes. A sheet that holds a patterned hatch at an angle can still write different DWG bytes on different systems, turned or not, since its pattern lines are laid out with the platform's sine and cosine ([#243](https://github.com/millZach/AgentDrafter/issues/243)). A mirror's line is worked out from its two points by arithmetic alone, at any angle.

**What the sheet description cannot hold is refused**, never approximated, as `bad_value` at the object's `target`: a table turned, since it hangs upright from its top left corner with no rotation of its own; a table scaled, since its table style and its cells size its text; a table mirrored across a line that is neither across nor up the sheet, which would not leave it upright (across or up the sheet, it hangs from the top left corner of its mirrored box and reads forwards); a multileader with block content mirrored, since its block is held upright; an ordinate dimension turned by other than a multiple of 90 degrees, or mirrored across a line that is neither across nor up the sheet nor at 45 degrees, since it measures along the sheet's X or Y; a multileader mirrored so that its landing, or an older leader with an annotation so that its hook and annotation, would go to the other side from its mirror image, since the side is not written but follows the last segment of its first leader line, or its line, to the right when that runs right or straight up or down, so that one whose last segment runs straight up or down is not mirrored across a line up the sheet, while one whose last segment runs to one side mirrors with the device it tags; and a viewport, a window on the paper, which `move` and `change_viewport` place instead. A polar or path `array` that would turn one is refused the same way, asking for `rotate: false` or `align: false`.

**Where it is written.** An object turned, scaled or mirrored in place is written where it is, as `move` writes it: a template's object in the template, so on every sheet built from it, all or nothing (decision 4), and a row's own object in the row; a mirrored copy where its source is. The reply's `sheets` and its message say where, and its `record` follows the rule for an edit that writes a row and a template together.

**The reply** lists the objects transformed in place under the operation's `transformed`, by ID on the sheet the operation names, and gives `before` and `after` when it names one; a mirror that keeps its source lists its copies under `copies`. The journal marks each transformed object changed and each copy added, on every sheet it is on, so a [preview](#a-dry-run), the viewer and undo work as for every edit.

```json
{"index":0,"op":"rotate","ok":true,"message":"turns `TB-001/strip` and `TB-001/tag` by 90 degrees about [40, 120, 0] (mm), in place, written in sheet `TB-001`'s row","sheets":["TB-001"],"record":"strip/rows.yaml","transformed":["TB-001/strip","TB-001/tag"]}
```

A `factor` of 0 or less, a mirror line whose two points are one seen from above, a missing key and a key an operation does not take refuse the change set whole; an object or a snapped point the sheet does not hold, a snapped mirror line that comes to one point, and what the sheet description cannot hold fail the sheets the operation reaches.

### Viewports

A sheet drawn in a [layout](sheet-description.md#layouts-and-viewports) is changed as any sheet is, in any of its spaces: an object added with `"space": "paper"` goes in paper space, such as a note in the title block, and one without it in model space. On a sheet drawn in [several layouts](sheet-description.md#several-layouts), an add names the layout whose paper space it goes in, `"space": "Wiring"`, and `paper` names none of them. An edit names its object by its ID or its name, whatever layout it is in: an ID does not name its layout. `change` sets `space` as any key, and moves the object to another space as it stands, unscaled, unlike AutoCAD's CHSPACE, which scales it by a viewport's.

**`add_viewport`** (`MVIEW`) adds a viewport with the keys a viewport takes, `center`, `width` and `height` on the paper, or `clip`, a boundary, `view_center` in model space and `scale`, `locked` and `frozen_layers`, to a sheet's row or to its template, as every add does: MVIEW's two corners are given as the window's centre and size, and its Object and Polygonal options as `clip`. A viewport goes in a sheet drawn in a layout; one added to a sheet without a layout fails the sheet with `bad_value`.

**`change_viewport`** takes `target`, a viewport by its ID or its name, and one or more of `scale`, a ratio such as `"1:5"` or a number, `view_center`, a point or a [snapped point](#snapped-points) on model space, and `locked`, `true` or `false`, and sets them, as AutoCAD's ZOOM with a scale, PAN and the display lock do in a viewport: the viewport stays where it is on the paper. AutoCAD has no one command for it. Its target is a viewport; anything else fails with `bad_value`. `move` moves a viewport on the paper, and its view moves with it.

**`clip_viewport`** (`VPCLIP`) takes `target`, a viewport, and `clip`: a boundary on the paper, as a viewport's [`clip`](sheet-description.md#clipped-viewports) is written, a closed polyline's `points` or a circle's `center` and `radius`, to clip the viewport to, or clip it again to; or `null`, to take its boundary away, as VPCLIP's Delete option does. Model space stays where it showed on the paper, as AutoCAD 2026's VPCLIP kept it: clipped, the viewport's window becomes the box round the boundary and its view centre the model point at the box's middle, its scale unchanged; unclipped, it keeps that box as its rectangle and its view centre, and its boundary goes, as VPCLIP's Delete erased it. It writes the keys that follow: `clip`, and `center`, `width` and `height` taken away, with `view_center` worked out, or the other way round. A viewport that is not clipped has no boundary to take away, and the operation fails with `bad_value`. Clipping leaves the view where it is, so a locked viewport may be clipped.

**`set_frozen_layers`** (`VPLAYER`) takes `target`, a viewport, and `frozen_layers`, the layers frozen in it alone, as AutoCAD's VPLAYER Freeze and Thaw leave them: the list replaces the viewport's own, and an empty list thaws every layer in it, taking the key away. Each is layer `0` or one of the sheet's, named exactly as it is defined; the sheet reads them in its layer table's order. `move` moves a clipped viewport's boundary with it.

**A locked viewport.** `change_viewport`, and `change`, do not change the scale or the view centre of a locked viewport, as AutoCAD's display lock keeps them: the operation fails with `locked` unless it sends `"locked": false` too, which unlocks it and changes them at once. A locked viewport can still be moved, resized, clipped, or put on another layer.

```json
{"op": "change_viewport", "target": {"id": "PANEL-001/view-a"}, "scale": "1:5",
 "view_center": {"snap": {"id": "PANEL-001/terminals", "snap": "midpoint", "which": "segments[0]"}}}
{"op": "clip_viewport", "target": {"id": "JB-201/view-a"}, "clip": {"center": [90, 150, 0], "radius": 60}}
{"op": "set_frozen_layers", "target": {"id": "JB-201/view-wired"}, "frozen_layers": ["dims", "wiring"]}
```

### Tables

Five operations edit a [table](sheet-description.md#tables)'s grid, as AutoCAD's TABLEDIT and its row and column commands do. Each names its table under `target`, by its ID or its name, as every edit names its object, and lands as any edit of an object does: a table of a template is edited in the template, so every sheet built from it follows, all or nothing, and a table of a row's own objects on that sheet alone. A table on a locked layer is not edited. Cells, rows and columns count from 0 from the top left, a cell `[row, column]`, as the record's `cells[r][c]` and the checker's `table_cell` do. Once the change set's operations are applied, each sheet they reach is read back and built, as for every operation, so a table they leave wrong fails the sheet, with the error placed in the operation that caused it, at its key, such as `operations[0].set.merge[0]` or `operations[1].height`.

- **`set_cell`** (`TABLEDIT`) takes `target`, `cell`, `[row, column]`, and `set`: the cell's keys, `text`, `style`, `height`, `justify` and `merge`, each to its new value as a sheet description writes a cell's; `null` takes a key away. In a template's table, `text` may be a value from the row, `{row: NAME}`, and so may a cell `add_row` and `add_column` take under `cells`, as the cell or its text; a row's own table takes none. A cell written as its text alone stays so when only its text is set, and becomes a mapping when another key is; one left with nothing is written `''`. A cell inside another cell's merge shows nothing of its own and is refused, naming the merge's cell, so set that cell, or take its merge away first. A `merge` keeps within the table and off other merges, and the cells it covers must be empty: one that is not is refused, and the error lists them, to be emptied first with `set: {"text": null}`. Text sent for a cell whose template takes its text from the row, `{row: NAME}`, as the cell or as its `text`, sets the sheet's row value, on that sheet alone, and the reply's `row_value` names it, as `set_attribute` does (decision 5); a value from the row, or `null`, sent there edits the template's cell. A `set_cell` that sets the row value and another key edits the template too, so it reaches every sheet built from it, and a refusal of it, such as a place the value fills on a locked layer, holds them all back.

  ```json
  {"op": "set_cell", "target": {"id": "DEMO-001/cables"}, "cell": [3, 0], "set": {"text": "C-104", "merge": [1, 2]}}
  ```

- **`add_row`** takes `target`, and `at`, the new row's place, the row there and those after it moving down; without it, after the last. `height` is its height; without it, the height of the row before it, or of the first row when it goes in at 0. `cells` is its cells, one for each column, each written as a sheet description writes a cell; without them, empty. A merge it falls inside, between its first and last rows, grows by a row, and the new row's cells under it must be empty. The title and header rows are the table's first: a row is not added above either, nor between them (`bad_value` at `at`).
- **`delete_row`** takes `target` and `row`. A merge it cuts through shrinks by a row, and one left spanning its own cell alone is taken away. A row holding a merged cell that spans more rows than it is refused, naming the cell, so take the merge away first with `set_cell`, `set: {"merge": null}`. Deleting the title row leaves the table without one, and the header row likewise. A table keeps at least one row.
- **`add_column`** and **`delete_column`** do the same across, with `at`, `width`, `cells`, one for each row, and `column`. A merge across the column grows or shrinks, a column holding a merged cell that spans more columns is refused, and a table keeps at least one column.

The reply's operation says what else it did to keep the table in line, such as `the merge of cell [0, 0] grows to [1, 5]`. The table is written again as an object an edit touched is ([How the records are written](#how-the-records-are-written)): each value it did not touch as it was, and a comment inside the table is lost.

### Leaders

Two operations edit a [multileader](sheet-description.md#leaders)'s leader lines, as AutoCAD's MLEADEREDIT adds and removes leaders. Each names its multileader under `target`, by its ID or its name, and lands as any edit of an object does: a multileader of a template is edited in the template, so every sheet built from it follows, all or nothing, and one of a row's own objects on that sheet alone. Once applied, each sheet they reach is read back and built, so a leader line the multileader cannot take fails the sheet, with the error at the operation's key.

- **`add_leader_line`** (`MLEADEREDIT`) takes `target` and `points`, each a point or a [snapped point](#snapped-points): the new leader line from its arrow's point through any bends, up to but not including the point where the multileader's leader lines meet its landing, which the operation appends, written as the multileader's first leader line writes it. At least one point, each apart from the one before and from that point, and the first segment at least as long as the arrow, unless the arrowhead is `none`; the error of one that is not is at `points`. It may come from either side of the landing: AutoCAD 2026 kept a second leader line from the landing's far side where it was when it laid the multileader out again.

  ```json
  {"op": "add_leader_line", "target": {"id": "DEMO-001/terminals"}, "points": [[4, 2, 0], [5, 2.5, 0]]}
  ```

- **`delete_leader_line`** takes `target` and `leader`, the leader line's place among the multileader's, counted from 0 in the order they are written. The last leader line is not deleted: delete the multileader instead. Deleting the first makes the next the first, whose last segment then sets the side the landing runs to.

**A leader's text from the row.** A `change` whose `set` gives a multileader's `content`, or an older leader's `annotation`, that differs from what the template writes in its `text` alone, where the template writes that text `{row: NAME}`, sets the sheet's row value `NAME` to the new text, on that sheet alone, as a text's contents from the row are set (decision 5), and the reply's `row_value` names it and every place it fills. Any other change of the content or the annotation, such as a new height, a value from the row, or a block, edits the template, so every sheet built from it follows.

```json
{"op": "change", "target": {"id": "DEMO-002/callout"}, "set": {"content": {"text": "AREA 200 RENAMED", "height": 0.2}}}
```

### Dimensions from others

Two operations add a [dimension](sheet-description.md#dimensions) from another, as AutoCAD 2026's DIMBASELINE and DIMCONTINUE added its own from made-up linear, aligned, angular and ordinate dimensions. Each names the dimension it is added from under `target`, by its ID or its name, and takes `point`, a point or a [snapped point](#snapped-points): the new dimension's second extension line's origin, or an ordinate dimension's new feature; and `id` and `name`, the new dimension's, when given. Without an `id` it is given one, `dimension-1` or the next number free. The new dimension takes the other's kind, style and layer, and is added where the other is written: beside a template's in the template, so it is on every sheet built from it, and beside a row's own in the row.

- **`add_baseline_dimension`** (`DIMBASELINE`) measures from the other's first extension line, its dimension line further out from the points than the other's by the style's `baseline_spacing`, AutoCAD's DIMDLI, times its `scale`; an angular dimension's arc that much further from the vertex.
- **`add_continued_dimension`** (`DIMCONTINUE`) measures from the other's second extension line, on the same dimension line or arc, and is written `continued: true`, as AutoCAD marks it.

A dimension from an aligned one is linear, at the aligned one's angle turned half round, as AutoCAD makes it. An angular one runs the same way round from the point it keeps as the other does, and keeps its location a third of the way round. An ordinate one, baseline and continued alike, measures the new feature on the same axis, its leader straight and ending level with the other's, and AutoCAD marks neither. A dimension is added from a linear, aligned, three-point angular or ordinate dimension: one from a radius or a diameter dimension, which AutoCAD's commands do not take, is refused, and so is one from an angular dimension between two lines, which AutoCAD 2026 was not measured adding from; add it from a three-point angular dimension at their vertex.

```json
{"op": "add_baseline_dimension", "target": {"id": "BRACKET-001/width"}, "point": [90, 20, 0], "id": "overall"}
{"op": "dimcontinue", "target": {"name": "first-hole"}, "point": {"snap": {"id": "BRACKET-001/hole-2", "snap": "center"}}}
```

### Blocks

Zach decided on 2026-11-13 how a change set defines and redefines the blocks of a [block library](block-library.md), as AutoCAD's BLOCK, BEDIT and ATTSYNC do.

- **`add_block`** (`BLOCK`) takes `name` and the block's definition as a block library writes it: `objects`, at least one, each a mapping with one key, its type, the objects a sheet holds, inserts of the library's other blocks and attribute definitions (`attdef`) among them, and `base_point`, `[0, 0, 0]` when left out. Lengths are in the set's declared unit, as the library is, or carry their own. A name the library has already, in any case, is `duplicate_name`, since AutoCAD ignores case in names.
- **`redefine_block`** (`BEDIT`) takes the same keys and replaces the block's whole definition, the base point and the full list of objects: what it leaves out is gone, and a `base_point` left out is `[0, 0, 0]`. A name the library does not have, written exactly as it writes it, is `unknown_name`, and the error lists the library's blocks. As it lands it brings every insert of the block into line, as `attsync` does.
- **`attsync`** (`ATTSYNC`) brings the inserts of the block `name`, or of every block of the library when `name` is left out, into line with the block's definition as the library writes it now, such as after the library was edited by hand. It writes nothing when every insert is in line already.
- **`import_blocks`** (`ADCENTER`, AutoCAD's DesignCenter) takes `from`, a DWG or a DXF by its path relative to the set's folder or absolute, and imports its blocks into the library, as `agentdrafter import-blocks` imports them into a library of its own: every named block, or those `blocks` names, each with every block it nests, with the layers, linetypes, hatch patterns and text styles they use that the library lacks. The drawing must be in the set's declared unit, or state none and take `units`. A block the library holds already is `duplicate_name` unless `replace`, which replaces its whole definition and brings its inserts into line as `redefine_block` does. What the core cannot carry is left out and counted under the operation's `imported`, with the blocks added and replaced, the table entries added and the notices. [Importing blocks](import-blocks.md) sets it out.

**Naming the library.** A block operation names its library by `library`, its path relative to the set's folder, as the rows write a template's path, such as `blocks.yaml`. It may be left out when every template of the set names the same library, as in the demo set. The set's libraries are the ones its templates name under `block_library`; two templates that name one file are one library. A `library` the templates do not name is `unknown_name`, and one left out while they name more than one is `missing_key`; each error lists the set's libraries by their paths in the set, and refuses the change set whole. A library the templates reach by more than one path to one file, such as a link and the file it links to, is refused whole as a template so reached is.

**Reach.** An edit to a library reaches every sheet whose template names it, all or nothing, as a template edit reaches every sheet built from the template: each is read back with the library as edited and built, a sheet that fails holds the operation back on every other sheet, with `shared_edit_failed`, and a sheet whose DWG comes out byte for byte the same is not written again. A block the library cannot read, such as one whose object names a layer the library does not define, fails every one of those sheets, and the error is placed in the operation that wrote the block, at the key within it, such as `operations[0].objects[0].line.layer`.

**Bringing inserts into line.** An insert carries a value for each tag its block carries: each of its attribute definitions' tags but a constant one's. A value an insert gives for a tag its block no longer carries, because the block no longer defines the tag or now defines it constant, is removed from the insert's record, as AutoCAD's ATTSYNC removes it: from the inserts of the templates that name the library, from the rows' own inserts of the sheets built from them, and from the inserts nested in the library's other blocks. A value the template takes from the row, `{row: REV}`, is removed from the template in the same way; the row's value stays in the row. A [list of parts](drawing-set.md#a-rows-value-inside-other-text), for a tag the block defined once in the library as the change set found it and still defines once, is one value and kept whole; a list for a tag it defined more than once is that tag's values, as above. An insert left with no value loses its `attributes`. A new tag needs nothing written: an insert that gives no value takes the definition's default. An insert stores no place for its attributes, so nothing else needs bringing into line. `agentdrafter undo` puts every value removed back, with the library, byte for byte.

The reply's operation lists the inserts it reached under `inserts`, by ID, on every sheet it reaches, an insert of a template on every sheet built from it: each insert of the block, and each insert of a block that nests it, however deep, which draws as the new definition does though it has no values of the block's to bring into line; and each value it removed under `attributes_removed`: the `record`, the insert's place in it under `field`, such as `objects[2]`, `rows[1].objects[0]` or, for an insert nested in another block, `blocks.STAMP.objects[0]`, its IDs on the sheets (none for an insert in a block), the `tag`, the `occurrence`, counted from 1, when only an item of a repeated tag's list was removed, and the `value` as the record wrote it, such as `B` or `{row: REV}`. A tag the block now defines fewer times than an insert's list gives values for loses the list's surplus items; one it now defines once takes the list's first item alone, and one it now defines more than once takes a single value as its list's first item. The journal entry marks each insert reached as changed, so the viewer shows it. The operation's `before` and `after` are the block as the library writes it, on one line.

The library is written as a template's objects are: the block an operation sets is written again whole, a key to a line, its base point in brackets and its objects an item to a line, in the place of its entry, and a block added goes after the library's last; a library whose blocks are written `{}` takes its first block a key to a line under `blocks:`. A block that only lost a nested insert's value is written again the same way. Every other block, comment and byte of the library stays as it was; a comment inside a block written again is lost.

## IDs stay put

Every object keeps its [ID](sheet-description.md#ids-and-names) for its life. An object without an `id` takes its place in its file as its ID, such as `objects[2]`, and that place would move when an object before it is deleted. So `delete` writes the place of each later object of the same list that has no `id` as its `id`, such as `id: objects[2]`, before it takes the object out, and the reply lists them under `ids_written`. The demo set's title block is `DEMO-001/objects[2]` before the trim line before it is deleted and after. An `id` may be written as a place for that reason, whole, as `objects[3]` or `rows[1].objects[0]`; an object without an `id` whose place is another object's written `id` is the `duplicate_id` error. An object added goes at the end of its list, so it moves nothing, and is always given an `id`.

## Each sheet fully or not at all

The operations are applied in order to the records in memory. Every sheet an operation reaches is then read from the edited records and built into a DWG in memory, as `agentdrafter build` would build it. A sheet that cannot take its part fails, and so does every operation that reaches it. An operation on a template reaches every sheet built from it, so it fails with any of them (decision 4), and takes those sheets' other operations with it, with the error `shared_edit_failed`. The operations left are applied again from the records as they were, until every sheet they reach takes its part. Only then is anything written, all of it or none of it: each record and each DWG is written whole beside its place first; then, one at a time, the file already there is moved aside and the new one moved into its place. If any step fails, every file already moved is put back as it was, byte for byte, every new file and every folder the change set made is taken away, and the error says nothing was changed.

A sheet that fails is left as it was, byte for byte: its row's text in its rows file, its template, and its DWG. A rows file holds the rows of many sheets, so its other rows can change while a failed sheet's row does not. A DWG that is there but cannot be written, such as one open in AutoCAD on Windows, fails its sheet before anything is written. A sheet whose rebuilt DWG is byte for byte the one already there is not written again.

**A change set that is wrong in itself is refused whole**, and nothing is done: an operation that does not exist, a sheet or a template the set does not hold, a key an operation does not take, an operation that leaves out a key it requires, such as an `add_line` with no `to`, a name with no sheet that is on no sheet or on more than one, an edit to a template the rows reach by more than one path. Each operation is checked for these before any is applied. There is no sheet to fail.

## An inconsistent set

When a change set lands on some sheets and fails on others, the set is inconsistent: the failed sheets did not take what the others did. It is marked in the file `.agentdrafter/inconsistent.yaml` in the set's folder, so the mark travels with the set and a later session sees it. The mark is written, or taken away when the last failure is fixed, in the same all-or-nothing step as the records and the DWGs: a change set whose mark cannot be written does not land at all. A change set that fails on every sheet it touches changes nothing and marks nothing.

The mark lists each failure: the sheet, the error's code and message, the sheets the change set landed on, and the operations that were the failed sheet's part, each as the JSON it was sent as, so an agent can send them again once it has fixed what failed. It is written in the same strict YAML as every file of a set:

```yaml
failures:
  - sheet: DEMO-002
    code: unknown_name
    message: 'operation 1 (`add_circle`) on sheet `DEMO-002`: unknown layer `note`; the sheet''s layers: 0, trim, border, notes'
    landed_on: [DEMO-001]
    operations:
      - '{"center":[5,5,0],"layer":"note","op":"add_circle","radius":0.5,"sheet":"DEMO-002"}'
```

**It is reported until fixed**, under `inconsistent`, by every reply that reads the set's folder: a change set's, a dry run's, and those of `agentdrafter build`, `plot` and `query` given the folder. The reviewer cannot accept a set while it is marked: [`accept`](journal.md#accept) is refused with the error `inconsistent`.

**A sheet is cleared by a change set that names it under `fixes` and lands on it.** The change set may hold operations for the sheet or none; either way the sheet is read and built again, and cleared only when it takes its part. A `fixes` naming a sheet the mark does not hold refuses the change set. When the last failure is cleared, the mark is taken away, and its folder when nothing else is in it. **Undoing the change set** puts the mark back as it was before it ([the journal](journal.md#undo)), which clears the failures that change set marked.

## A dry run

`--dry-run`, or `dry_run` in the request, does everything a change set does but write: it applies the operations in memory, reads and builds every sheet they reach, and reports what would land, what would fail, the records that would be written, each object as its record would write it before and after, the row values that would be set, and the mark as it would stand. It changes nothing on disk: no record, no DWG, no output folder and no mark.

**Drawn before it lands.** `agentdrafter display-list <set> --sheet <sheet> --change-set <change-set.json | ->`, or `change_set` in the request, runs the same dry run and draws the sheet as the change set would leave it, reading the set through the records it would write; nothing is written and nothing goes in the journal. Its reply's `preview` holds the dry run's reply under `change`, the checker's reply on the sheets it would land on under `checks`, read from those records, and the objects it would add, change and delete, with what the sheet drawn would draw otherwise ([viewer.md](viewer.md#a-change-set-drawn-before-it-lands)). The drafting app draws a template edit with it before it lands. A preview writes nothing, so it rolls nothing back: on a set whose journal holds an entry a commit has not finished, or a last line written in part, it is refused with `rollback_failed`, and any other command on the set rolls that back first. A `change_set` of `null` is refused as `change` refuses it, and one with `entry` is refused. With `all_or_none` (`--all-or-none`), it is applied as a proposal is kept, on every sheet it reaches or on none: a sheet that would take its part while another refuses is shown held back (`shared_edit_failed`), and nothing is drawn as changed. The viewer draws proposals so.

## Proposals

A change set can wait, to be looked at first: an agent that wants its work seen before it lands calls `propose` where it would call `change` (#155). A **proposal** is a change set that waits, drawn on its sheets for the drafter to keep or discard; nothing changes until it is kept. Four commands of the registry, reached by every interface:

```
agentdrafter propose <drawing-set-folder> <change-set.json | -> [--by <who>] [--fonts <folder>]... [--no-system-fonts]
agentdrafter proposals <drawing-set-folder>
agentdrafter keep <drawing-set-folder> <proposal> [--by <who>] [--fonts <folder>]... [--no-system-fonts]
agentdrafter discard <drawing-set-folder> <proposal> [--by <who>]
```

- **`propose`** (`set`, `change_set`, `by`) checks the change set as `change` checks it, by a [dry run](#a-dry-run) on the set as it is: a change set the core refuses whole, or one a sheet would refuse for its records, such as a name that does not exist, is not recorded, and the reply says why. What is checked is its records, not whether each output can be written now: a sheet whose DWG cannot be written just now, such as one open in AutoCAD, does not stop it, nor do the sheets of its template a template edit is held back on by it; the reply's message names them, and they must be writable by the time it is kept. Its dry run applies it as `keep` would, on every sheet it reaches or on none, so `would` shows a sheet that would take its part while another refuses held back, with no build and no operation landing. It records the proposal in the set's folder, beside the journal, in `.agentdrafter/proposals/`, one file each, `<number>.json`, numbered from 1 on their own: who proposed it, when, the journal's newest entry it was made against (`base_entry`, 0 for an empty journal), the change set, and the sheets it reaches. It writes nothing else: no record, no DWG, no line of the journal. The reply gives its number, and under `would` the dry run's reply, what keeping it would do now.
- **`proposals`** (`set`) lists those waiting, the oldest first: each one's `proposal`, `by`, `at`, `base_entry`, `changed_since`, the change sets and undos that landed after it was made, its `change_set` and its `sheets`. It is cheap: it reads the files and the journal, and draws nothing; the viewer draws each with [`display-list`'s `change_set`](#a-dry-run).
- **`keep`** (`set`, `proposal`, `by`) lands it through `change`'s own path, applied to the set as it stands then, each sheet fully or not at all (ADR-0005), and the proposal whole or not at all: when any sheet it reaches would refuse it, every other is left as it is too (`shared_edit_failed`, naming the sheet that refuses) and is not built, no operation lands (each that would have is held back with the same error), nothing is written, and the reply is not ok, with `sheets_failed`. Its reply is `change`'s, with the proposal's number, `proposed_by` and `kept_by`. A proposal made before the set changed is applied to it as it now is, as any change set sent then would be. Its journal entry is a change set's, which also names the proposal, who proposed it and who kept it ([journal.md](journal.md#what-it-records)); it is undone and accepted as any change set is, and undoing it does not bring the proposal back. When the set refuses it, the attempt is recorded as `change` records a [failed attempt](journal.md#what-it-records), naming the proposal too, and the proposal goes on waiting.
- **`discard`** (`set`, `proposal`, `by`) ends it without landing. The journal records it with a line of its own, `discard`, naming the proposal and who proposed and discarded it. It changes no file of the set, so there is nothing to undo, and an undo passes over it.
- **Ended.** A proposal kept or discarded has ended: the journal's entry that ended it says so, so it ends in the same step as that entry is written. `keep` and `discard` refuse it with `proposal_ended`, which says how it ended and by whom, and record nothing. An undo of the change set that kept it does not end the ending.
- **Who.** `by` is the caller's own words, up to 80 characters, spaces round them among them, with no control character and something besides spaces, such as `Assistant`; the input's schema says so too (`maxLength` and `pattern`), and `null` is refused as the schema refuses it. A proposal's number is counted from 1, and 0 is refused as `usage`. With none, it is the interface it came through: `agent (command line)`, `agent (HTTP API)`, `agent (MCP)`, `agent (Python)`, or `drafter` through the viewer, whose page sends `drafter` itself and shows it as "you"; a proposal made through the MCP server the drafter's assistant is given, `agentdrafter mcp --assistant`, is the assistant's, `Assistant` ([servers.md](servers.md#the-assistant)). It is recorded and shown; nothing else differs by it.
- **An accept** is unchanged: proposals waiting go on waiting through it, and it issues only the change sets in place.
- **Read-only.** `propose` and `proposals` write nothing to the set, so, as a preview, they roll nothing back: on a set whose journal holds a commit that has not finished, or a last line written in part, they are refused (`rollback_failed`) and change nothing; any other command rolls that back first.

The event stream reports a proposal made, kept and discarded as it reports a change set ([servers.md](servers.md#watching-a-set)).

## The reply

```json
{"ok":true,"result":{"set":"demo-set","units":"in","dry_run":true,"message":"a dry run, which wrote nothing: the change set would land on every sheet it touches: `DEMO-001`, `DEMO-002` and `DEMO-003`","landed":3,"failed":0,"sheets":[{"sheet":"DEMO-001","template":"templates/ansi-b.yaml","ok":true,"operations":[0],"fixed":false,"dwg_written":false,"result":{"dwg":"demo-set/dwg/DEMO-001.dwg","...":"..."},"notices":[]},"..."],"operations":[{"index":0,"op":"move","ok":true,"message":"moves `DEMO-001/border` by [0.1, 0, 0] (in), in the template `templates/ansi-b.yaml`, so on every sheet built from it: `DEMO-001`, `DEMO-002` and `DEMO-003`","sheets":["DEMO-001","DEMO-002","DEMO-003"],"record":"demo-set/templates/ansi-b.yaml","id":"DEMO-001/border","before":"polyline: {points: [[0.5, 0.5, 0], [16.5, 0.5, 0], [16.5, 10.5, 0], [0.5, 10.5, 0]], closed: true, layer: border, id: border}","after":"polyline: {points: [[0.6, 0.5, 0], [16.6, 0.5, 0], [16.6, 10.5, 0], [0.6, 10.5, 0]], closed: true, layer: border, id: border}"},{"index":1,"op":"set_attribute","ok":true,"message":"sets sheet `DEMO-002`'s row value REV to `A`, since its template takes the attribute REV of `DEMO-002/objects[2]` from the row; the value fills attributes.REV of `DEMO-002/objects[2]`","sheets":["DEMO-002"],"record":"demo-set/rows.yaml","id":"DEMO-002/objects[2]","before":"insert: {...}","row_value":{"sheet":"DEMO-002","value":"REV","was":"0","now":"A","fills":[{"id":"DEMO-002/objects[2]","at":"attributes.REV"}]}}],"records":["demo-set/rows.yaml","demo-set/templates/ansi-b.yaml"]}}
```

| Key | What it is |
|---|---|
| `dry_run` | Whether this was a dry run, which wrote nothing. |
| `journal_entry` | The [journal](journal.md) entry that records the change set, or the failed attempt; `agentdrafter undo` takes a change set by it. Absent for a dry run, which is not recorded. |
| `message` | What the change set did, in words. |
| `landed`, `failed` | How many sheets took their part, and how many did not. |
| `sheets` | Each sheet the change set touched, in the set's order: whether it took its part, the operations that were its part, whether it was cleared from the mark (`fixed`), whether its DWG was written, what its build wrote (`result`, as `build` reports a sheet), and why it failed (`error`). |
| `operations` | Each operation, in the order sent: whether it landed, what it does in words, the sheets it reaches, the record it writes, the object's ID, the object as its record writes it `before` and `after`, the row value it sets (`row_value`), or the row values an edit matched to a list of parts sets, each with the same keys (`row_values`), the IDs it wrote (`ids_written`), the inserts a block operation brought into line (`inserts`) and the attribute values it removed (`attributes_removed`, [Blocks](#blocks)), each snapped point it took and what it named (`snapped`, [Snapped points](#snapped-points)), each copy a copy, an array or a mirror made, with its source and its place (`copies`, [Copy and array](#copy-and-array)), the objects a rotate, a scale or a mirror transformed in place (`transformed`, [Rotate, scale and mirror](#rotate-scale-and-mirror)), the row values an edit created to give a copy its own (`created_row_values`), and why it did not land (`error`). |
| `records` | The records written, or that a dry run would write. |
| `inconsistent` | The set's mark as it stands after the change set, or would after a dry run; absent when the set is consistent. |
| `checks` | The [checker](check.md)'s reply on the sheets the change set landed on, read back from their saved files once they were written: `{"ok": true, "result": {...}}`, with each failed check under `findings`. A failed check does not stop a change set from landing, and does not make its reply not `ok`. Absent for a dry run, which saves nothing to read back, and when no sheet took its part. |

When some sheets failed, the reply is not `ok`, carries the same `result`, and adds a `sheets_failed` error; the command exits with 1:

```json
{"ok":false,"result":{"landed":1,"failed":1,"sheets":[{"sheet":"DEMO-001","ok":true,"...":"..."},{"sheet":"DEMO-002","ok":false,"error":{"code":"unknown_name","message":"operation 1 (`add_circle`) on sheet `DEMO-002`: unknown layer `note`; the sheet's layers: 0, trim, border, notes","field":"operations[1].layer"},"...":"..."}],"inconsistent":{"file":"demo-set/.agentdrafter/inconsistent.yaml","...":"..."},"...":"..."},"error":{"code":"sheets_failed","message":"the change set landed on `DEMO-001` and failed on `DEMO-002`, which was left as it was; ..."}}
```

## Errors

An error from an operation names it, `operation 1 (`add_circle`)`, and the sheet it failed on, and its `field` is in the change set, such as `operations[1].layer` or `operations[0].target.id`. An error about a choice lists the valid choices: the operations with their aliases, the set's sheets or templates, the IDs or names on the sheet, the keys an object takes, the sheet's layers, text styles and hatch patterns, the library's blocks or the block's attribute tags. The sheet's own rules are the sheet description's, so an added object is checked exactly as a built one is. An error a sheet had before the change set is reported as it is, placed in its file.

| Code | In a change set it means |
|---|---|
| `unknown_name` | No operation, sheet, template, ID or name of that name; or a layer, text style, block or attribute tag the sheet does not have; or a block library the set's templates do not name, or a block it does not have; or a snapped point's object, or its `which`, that the sheet does not have. |
| `unknown_key` | A key the change set, an operation or the object's type does not take. |
| `missing_key`, `wrong_type`, `bad_number`, `bad_value` | As in a sheet description, for an operation's keys or the object it writes. `missing_key` also: a block operation with no `library` when the set's templates name more than one. `bad_value` also: a name with no sheet on more than one sheet, a `change` of an `id`, `fixes` naming a sheet the mark does not hold, a snapped point that names no point, several without `which`, an object on a sheet the operation does not reach, or a different point on two sheets a template edit reaches, or one where a direction is taken. |
| `usage` | An add given both `sheet` and `template`, a target or a snapped point given both an ID and a name, or a move given `by` with `from` and `to`. |
| `duplicate_id`, `duplicate_name` | An `id` or a name the sheet has already, the template's and the row's objects together; or a layer, linetype or text style the template has already; or a block the block library has already. |
| `locked` | An edit to an object on a locked layer, or a copy, a turn, a scaling or a mirror of one. |
| `in_use` | A delete of a layer, linetype or text style that something still uses. |
| `shared_edit_failed` | The sheet was left as it was because a template edit in its part could not land on another sheet built from the same template, or an edit to a block library on another sheet whose template names it. |
| `not_a_build`, `write_failed` | The sheet's DWG is not one AgentDrafter built, or cannot be written. |
| `sheets_failed` | Some sheets failed and were left as they were; the others took their part. |
| `read_failed` | A file of the set, or the mark, cannot be read; or a proposal's file. |
| `proposal_ended` | A `keep` or a `discard` of a proposal kept or discarded already. |

## How the records are written

Only what an operation touched is rewritten. Each record is read into the strict YAML tree, which keeps where each node is written; an operation edits the tree, and the new text replaces just the lines of each object it touched, writes an object added after the last of its list, and takes a deleted object's lines away. An object rewritten keeps its style: an object written on one line in braces stays so, and one written a key to a line stays so, and every value it did not touch is written exactly as it was, quotes and all. Comments outside a rewritten object, and everything else in the file, stay byte for byte; a comment inside one is lost. A row that had no objects gains `objects:`, and a list written `[]` becomes a list of items. A new value is written plain unless YAML would read it as something else, a number as few digits as hold it, and text of several lines as a literal block. The text is read back before it is written, and must read as the edit made it; a text that does not is refused rather than written.

## How it works

`crates/agentdrafter-core/src/change/` holds the change set: `operations.rs` the operations, their schemas and reading a change set, `apply.rs` applying it sheet by sheet, `array.rs` where a copy or an array puts each copy, `src/description/edit/transform.rs` turning, scaling and mirroring an object's record, `commit.rs` putting its records, DWGs and mark in place together or not at all, with its entry in the journal written ahead of them and marked done after, `journal.rs` and `undo.rs` the [journal and undo](journal.md), `recover.rs` rolling back a commit a killed process never finished ([journal.md](journal.md#a-commit-that-never-finished)), and `mod.rs` the reply and the mark. `src/description/edit.rs` edits a record in place and `emit.rs` writes nodes back as text; `mark.rs` reads and writes the mark. Every sheet is read by the same code that reads it for a build, and built by the same code that builds it.

## What is not offered yet

- Failed checks in a dry run: the [checker](check.md) reads saved files, and a dry run saves none.
- Revising an existing DWG: a change set edits a drawing set's records, not an original.
- Change sets on a sheet description outside a drawing set.
- Operations on the set's other parts: hatch patterns, the block library's own layers, linetypes and text styles, renaming or deleting a block, rows and their values directly, and a sheet's template.
- Choosing where in its list an added object goes; it is drawn after the others.
- A column added after the last joining a merge that ends at the last, such as a title's: the merge stays as it was, and `set_cell` widens it. A merge grows only when the row or column goes in between its first and last.
- Moving an object from a template to a row, or the reverse.
- AutoCAD's associative array, which stays one object to edit; an array lands as plain copies (Zach, 2026-11-21). Nor an array's levels along Z, ARRAYPATH's measure along a path's Z or its tangent direction other than the path's own, or numbering a copied insert's attributes: set them with `set_attribute`.
- Turning a table, a multileader, an older leader, a dimension or a viewport, in a polar or a path array (#67).
- The MCP server, the Python library and the HTTP API ([#14](https://github.com/millZach/AgentDrafter/issues/14)), which will reach the same core entry point.
