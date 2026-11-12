# Change sets

A change set is the operations an agent sends together to change a drawing set (see [`CONTEXT.md`](../CONTEXT.md)): typed operations such as `insert_block` or `set_attribute`, each with its AutoCAD command name as an alias. It lands at once on the working revision. Each sheet takes its part fully or not at all (ADR-0005), and a set where some sheets failed is marked inconsistent until they are fixed. An agent can list the operations with their schemas, and can ask for a dry run.

`agentdrafter change <drawing-set-folder> <change-set.json | -> [--dry-run] [--fonts <folder>]...` applies a change set written as JSON, from a file or from standard input, and prints one line of JSON. It is a thin layer over the core's `change_with`, which takes the change set and returns the reply as plain data (ADR-0007). `agentdrafter operations` lists the operations, and `agentdrafter schema change-set` prints the change set's JSON Schema.

## Where a change lands

Zach decided on 2026-11-11:

1. **The working revision is the set's own files, edited in place.** A change set writes the set's records, its templates and its rows file, directly, and builds again only the DWGs of the sheets it touched. History is the [journal](journal.md)'s, and git's too when the set is in a repository. A DWG is replaced only when it carries AgentDrafter's build mark, as `build --overwrite` replaces one; any other file at a sheet's path may be an original, so that sheet fails and the file is not touched. Revising an existing DWG, an original into a revision, is not part of change sets yet.
2. **An edit to an object that comes from a template changes the template.** Every sheet built from it follows, and the reply lists the sheets it reaches.
3. **An object added to one sheet goes in that sheet's row**, under the row's [`objects`](drawing-set.md#a-rows-own-objects). The template stays shared.
4. **A template edit is all or nothing across its sheets.** It lands only if every sheet built from that template takes its whole part. Otherwise the template, those sheets' rows and their DWGs stay as they were, byte for byte. Sheets built from other templates still land. ADR-0005 stands.
5. **A value the template takes from the row is set in that sheet's row.** Setting an attribute, or a text's contents, whose template value is `{row: NAME}` writes that sheet's row value `NAME`, on that sheet alone, and the reply names every place on the sheet the value fills.

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
| `insert_block` | `INSERT` | Adds an insert of a block from the block library. |
| `change` | `CHANGE` | Sets keys of an object. |
| `move` | `MOVE` | Moves an object. |
| `delete` | `ERASE` | Deletes an object. |
| `set_attribute` | `ATTEDIT` | Sets one attribute value of an insert. |

`agentdrafter operations` gives each with what it does and its JSON Schema (draft 2020-12), with the definitions it refers to, so an agent can check an operation before it sends it:

```json
{"ok":true,"result":{"how":"A change set is {\"operations\": [...]}: ...","operations":[{"name":"add_line","alias":"LINE","does":"Adds a line, written with the keys a line takes in a sheet description, ...","schema":{"type":"object","required":["op","from","to"],"...":"..."}},"..."]}}
```

**Adding.** Every object a sheet description holds has an operation that adds it. It takes the keys that object takes in a [sheet description](sheet-description.md#objects), `id`, `name` and `layer` among them, and where it goes: `sheet`, to add it to that sheet's row, or `template`, the template's path as the rows write it, to add it to the template and so to every sheet built from it. The object is drawn after the objects already in its list. An object added without an `id` is given one, its type and the first number free on the sheets it is added to, such as `line-1` or `insert-2`, so that its ID is fixed for its life; the reply gives its ID.

**`change`** takes `target` and `set`, a mapping from each key to set to its new value, written as a sheet description writes it; `null` takes an optional key away. It sets any key the object's type takes, and `name`. It does not set `id`, since an object's ID is fixed for its life, nor an insert's `attributes`, which `set_attribute` sets one at a time, nor the object's type. A text's or an mtext's `contents` whose template takes them from the row is set in the sheet's row (decision 5).

**`move`** takes `target` and `by`, `[dx, dy, dz]`. Every point the object is drawn through moves: a line's ends, a polyline's points, a circle's, an arc's or an ellipse's centre, a spline's fit or control points, the points and edges of each loop of a hatch's boundary, and the position of a text, an mtext or an insert. A coordinate along an axis it does not move along is left as the record writes it; one it moves along is written in the declared unit.

**`delete`** takes `target`.

**`set_attribute`** takes `target`, an insert, `tag` and `value`, one line of text. An attribute the insert leaves out is added. When the insert's template takes the attribute from the row, the sheet's row value is set instead (decision 5), and the reply's `row_value` names the value and every place on the sheet it fills: the demo set's `TITLE` fills both the title block's `TITLE` and the heading's contents.

## IDs stay put

Every object keeps its [ID](sheet-description.md#ids-and-names) for its life. An object without an `id` takes its place in its file as its ID, such as `objects[2]`, and that place would move when an object before it is deleted. So `delete` writes the place of each later object of the same list that has no `id` as its `id`, such as `id: objects[2]`, before it takes the object out, and the reply lists them under `ids_written`. The demo set's title block is `DEMO-001/objects[2]` before the trim line before it is deleted and after. An `id` may be written as a place for that reason, whole, as `objects[3]` or `rows[1].objects[0]`; an object without an `id` whose place is another object's written `id` is the `duplicate_id` error. An object added goes at the end of its list, so it moves nothing, and is always given an `id`.

## Each sheet fully or not at all

The operations are applied in order to the records in memory. Every sheet an operation reaches is then read from the edited records and built into a DWG in memory, as `agentdrafter build` would build it. A sheet that cannot take its part fails, and so does every operation that reaches it. An operation on a template reaches every sheet built from it, so it fails with any of them (decision 4), and takes those sheets' other operations with it, with the error `shared_edit_failed`. The operations left are applied again from the records as they were, until every sheet they reach takes its part. Only then is anything written, all of it or none of it: each record and each DWG is written whole beside its place first; then, one at a time, the file already there is moved aside and the new one moved into its place. If any step fails, every file already moved is put back as it was, byte for byte, every new file and every folder the change set made is taken away, and the error says nothing was changed.

A sheet that fails is left as it was, byte for byte: its row's text in the rows file, its template, and its DWG. The rows file holds every sheet's row, so its other rows can change while a failed sheet's row does not. A DWG that is there but cannot be written, such as one open in AutoCAD on Windows, fails its sheet before anything is written. A sheet whose rebuilt DWG is byte for byte the one already there is not written again.

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
| `operations` | Each operation, in the order sent: whether it landed, what it does in words, the sheets it reaches, the record it writes, the object's ID, the object as its record writes it `before` and `after`, the row value it sets (`row_value`), the IDs it wrote (`ids_written`), and why it did not land (`error`). |
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
| `unknown_name` | No operation, sheet, template, ID or name of that name; or a layer, text style, block or attribute tag the sheet does not have. |
| `unknown_key` | A key the change set, an operation or the object's type does not take. |
| `missing_key`, `wrong_type`, `bad_number`, `bad_value` | As in a sheet description, for an operation's keys or the object it writes. `bad_value` also: a name with no sheet on more than one sheet, a `change` of an `id`, `fixes` naming a sheet the mark does not hold. |
| `usage` | An add given both `sheet` and `template`, or a target given both an ID and a name. |
| `duplicate_id`, `duplicate_name` | An `id` or a name the sheet has already, the template's and the row's objects together. |
| `shared_edit_failed` | The sheet was left as it was because a template edit in its part could not land on another sheet built from the same template. |
| `not_a_build`, `write_failed` | The sheet's DWG is not one AgentDrafter built, or cannot be written. |
| `sheets_failed` | Some sheets failed and were left as they were; the others took their part. |
| `read_failed` | A file of the set, or the mark, cannot be read. |

## How the records are written

Only what an operation touched is rewritten. Each record is read into the strict YAML tree, which keeps where each node is written; an operation edits the tree, and the new text replaces just the lines of each object it touched, writes an object added after the last of its list, and takes a deleted object's lines away. An object rewritten keeps its style: an object written on one line in braces stays so, and one written a key to a line stays so, and every value it did not touch is written exactly as it was, quotes and all. Comments outside a rewritten object, and everything else in the file, stay byte for byte; a comment inside one is lost. A row that had no objects gains `objects:`, and a list written `[]` becomes a list of items. A new value is written plain unless YAML would read it as something else, a number as few digits as hold it, and text of several lines as a literal block. The text is read back before it is written, and must read as the edit made it; a text that does not is refused rather than written.

## How it works

`crates/agentdrafter-core/src/change/` holds the change set: `operations.rs` the operations, their schemas and reading a change set, `apply.rs` applying it sheet by sheet, `commit.rs` putting its records, DWGs and mark in place together or not at all, with its entry in the journal written ahead of them and marked done after, `journal.rs` and `undo.rs` the [journal and undo](journal.md), `recover.rs` rolling back a commit a killed process never finished ([journal.md](journal.md#a-commit-that-never-finished)), and `mod.rs` the reply and the mark. `src/description/edit.rs` edits a record in place and `emit.rs` writes nodes back as text; `mark.rs` reads and writes the mark. Every sheet is read by the same code that reads it for a build, and built by the same code that builds it.

## What is not offered yet

- Failed checks in a dry run: the [checker](check.md) reads saved files, and a dry run saves none.
- Revising an existing DWG: a change set edits a drawing set's records, not an original.
- Change sets on a sheet description outside a drawing set.
- Operations on the set's other parts: layers, linetypes, text styles, hatch patterns, blocks, rows and their values directly, and a sheet's template.
- Choosing where in its list an added object goes; it is drawn after the others.
- Moving an object from a template to a row, or the reverse.
- The MCP server, the Python library and the HTTP API ([#14](https://github.com/millZach/AgentDrafter/issues/14)), which will reach the same core entry point.
