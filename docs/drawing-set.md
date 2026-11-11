# The drawing set

A drawing set is a job's sheets together with the block library, templates and data they share (see [`CONTEXT.md`](../CONTEXT.md)). On disk it is a folder: the set's settings, its templates, its block library and its rows. `agentdrafter build <folder>` builds every sheet in it, one DWG per row, each a template plus that row of data. Building one sheet description on its own works as it always has.

Every file in a set is written in the same [strict subset of YAML](sheet-description.md#the-yaml-subset) as a sheet description, and each kind has its schema: `agentdrafter schema set` for the settings, `agentdrafter schema rows` for the rows and `agentdrafter schema template` for a template. Each schema accepts exactly what the reader accepts, and tests keep the two in step. The rows are YAML, not CSV: Zach decided this on 2026-11-11, so that a set's data is read by the same strict reader, with the same errors, as everything else in it.

## An example

`examples/demo-set/` is a made-up set of the demo plant: one template of an 11 x 17 inch sheet with a border and a title block, the block library the title block comes from, and three rows.

```
examples/demo-set/
  set.yaml                 the set's settings
  rows.yaml                one row per sheet
  blocks.yaml              the block library
  templates/ansi-b.yaml    the template
```

```yaml
# set.yaml
units: in
output: dwg
rows: rows.yaml
```

```yaml
# rows.yaml
rows:
  - sheet: DEMO-001
    template: templates/ansi-b.yaml
    values:
      TITLE: AREA 100 GENERAL ARRANGEMENT
      DWG_NO: DEMO-001
      SHEET: 1 OF 3
      REV: '0'
      DATE: 2026-11-11
```

```yaml
# templates/ansi-b.yaml, in part
units: in
block_library: ../blocks.yaml
objects:
  - polyline: {points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true, layer: trim}
  - insert:
      block: TITLE_BLOCK
      position: [16.5, 0.5, 0]
      name: title-block
      attributes:
        TITLE: {row: TITLE}
        DWG_NO: {row: DWG_NO}
        SHEET: {row: SHEET}
        REV: {row: REV}
        DATE: {row: DATE}
        DRAWN: AD
  - text: {contents: {row: TITLE}, position: [1, 10, 0], height: 0.25, style: notes, layer: notes}
```

`agentdrafter build examples/demo-set` writes `examples/demo-set/dwg/DEMO-001.dwg`, `DEMO-002.dwg` and `DEMO-003.dwg`, each with its row's values in its title block.

## The settings

The settings are the file `set.yaml` at the top of the set's folder. `agentdrafter build` finds the set by it.

| Key | Required | What it is |
|---|---|---|
| `units` | yes | The set's declared unit: `mm`, `cm`, `m`, `in` or `ft`. Every template, sheet description and block library in the set is in it, and every DWG the set builds is in it. There is no default. |
| `output` | yes | The folder the set's outputs go to, relative to the set's folder, such as `dwg` or `../issued`: its DWGs, and its PDFs when it is plotted. A build or a plot makes it if it is not there. |
| `rows` | yes | The file the set's rows are in, relative to the set's folder, such as `rows.yaml`. |

A path in a set is written as a sheet description writes its block library's: relative, with `/` between folders on every system, not starting with `/`, and holding none of `\ : < > " | ? *`.

## The rows

The rows file holds one key, `rows`: a list of rows, one per sheet, in the order the set builds them. A row takes:

| Key | Required | What it is |
|---|---|---|
| `sheet` | yes | The sheet's name, which names its DWG: `<output>/<sheet>.dwg`. |
| `template` | yes | The sheet's template: the path of its file, relative to the set's folder. |
| `values` | no | The values the template takes from this row: a mapping from each value's name to its text. One for each `{row: NAME}` the template writes; a value the template does not use is reported as a notice, and the sheet is still built (see [Building](#building)). |
| `objects` | no | The sheet's own objects, on this sheet alone: a list of objects as a sheet description writes them. See [A row's own objects](#a-rows-own-objects). |

A sheet's name is a file name every system allows: 1 to 251 characters, so that with `.dwg` it is at most 255; not starting or ending with a space and not ending with `.`; holding none of `/ \ : * ? " < > |` and no control characters; and not, up to its first `.`, a name Windows keeps for a device (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`), in any case. Each name is given once in a set, and names that differ only in case are one name, since some systems would write them to one file.

A value's name is one or more ASCII letters, digits, `_` and `-`, such as `TITLE` or `dwg-no`, and is matched exactly, case and all. A value is text, as every scalar is: `REV: 0` and `REV: '0'` are the same value. Whether a value is allowed where its template puts it is the sheet description's rule for that place: one line for an attribute, something that shows for a text's contents.

### A row's own objects

A row may hold objects of its own, which are on its sheet alone. Zach decided on 2026-11-11 that an object added to one sheet goes in that sheet's row, so the template stays shared: an operation that adds an object to a sheet writes it here ([Change sets](change-sets.md)), and a generator or a person may write them too.

```yaml
rows:
  - sheet: DEMO-002
    template: templates/ansi-b.yaml
    values:
      TITLE: AREA 200 GENERAL ARRANGEMENT
      ...
    objects:
      - circle: {center: [8, 5, 0], radius: 1, layer: notes, id: tank}
      - text: {contents: TANK, position: [8, 5, 0], height: 0.2, style: notes}
```

- **They are part of the sheet** for building, rendering, plotting and querying, drawn after the template's objects, in the order the row lists them.
- **They are written as a sheet description writes its objects**, with the same keys and rules, and the [schema](#the-rows) of the rows takes exactly what the reader takes. They name the layers, linetypes, text styles and hatch patterns the template defines and the blocks of the template's block library.
- **They write their values themselves.** A value from the row, `{row: NAME}`, belongs in a template; in a row's own object it is an error.
- **IDs.** Each `id` is given once on the sheet, the template's objects and the row's together, so `DEMO-002/tank` is unique in the set. An object without an `id` takes its place in the rows file, such as `DEMO-002/rows[1].objects[1]`.
- **Errors** in a row's own object are placed in the rows file, at the object's place, such as `rows[1].objects[0].circle.layer`.

## Templates

A template is a sheet's border, title block and fixed content, waiting for data. It is written with what a sheet description holds, and where a sheet's value comes from its row, it writes `{row: NAME}` in place of the value:

- as the value of an insert's attribute, such as a title block's `TITLE: {row: TITLE}`;
- as a `text`'s or an `mtext`'s `contents`.

A value from the row stands for the whole value; it is not spliced into other text. The same value can be used in more than one place. Each sheet is the template with every `{row: NAME}` replaced by its row's value `NAME`, and is built as a sheet description is.

- **Units.** A template states `units`, as every sheet description does, and it must be the set's. A block library it names must be in the same unit.
- **Block library.** A template names its block library under `block_library`, relative to the template's own folder, as a sheet description does.
- **Paper size.** There is no paper setting. A sheet is as large as what its template draws: the demo set's template draws its trim line at 17 by 11 inches, and the usual case is one DWG per sheet, drawn in model space with its border.
- **A sheet drawn by hand.** A template that writes no `{row: NAME}` is a sheet description. A set can build one as a row with no values.
- **IDs.** A template's objects are on every sheet built from it, so each sheet's object's [ID](sheet-description.md#ids-and-names) is the sheet's name joined to the `id` the template gives the object, or to its place in the template: the demo set's border is `DEMO-001/border` on its first sheet and `DEMO-002/border` on its second, and its trim line, which has no `id`, is `DEMO-001/objects[0]`. Each sheet's name is given once in a set and each `id` once in its template, so every ID is unique in the set.

A sheet description built on its own takes no value from a row: `{row: NAME}` in it is an error that says to build the set's folder or to write the value.

## Building

`agentdrafter build <folder> [--overwrite] [--fonts <folder>]...` reads the settings and the rows, makes the output folder, then builds each row's sheet in turn into `<output>/<sheet>.dwg`, in the 2018 format, with the build mark. `--output` is refused for a set: its settings say where its DWGs go.

- **Each sheet is built fully or not at all** (ADR-0005). A sheet that fails, because its row lacks a value, its template has a mistake or its DWG cannot be written, writes nothing; a DWG an earlier build left at its path stays as it was.
- **The other sheets are still built.** One bad row does not stop a long run. The reply says for each sheet what was built and what failed.
- **Settings or rows that cannot be read stop the set** before anything is written, since without them there is nothing to build.
- **An existing DWG is replaced only when asked.** Without `--overwrite`, a sheet whose DWG is already there fails with `output_exists`. With it, each sheet replaces only a DWG that carries AgentDrafter's build mark; any other file may be an original, so that sheet fails with `not_a_build` and the file is not touched.
- **A value the template does not use is reported, not refused.** Zach decided this on 2026-11-11. The sheet is built without it, and its outcome's `notices` lists each such value, so a stray or misspelt one stays in sight: `{"code":"unused_value","message":...,"sheet":"DEMO-003","template":"templates/ansi-b.yaml","field":"rows[2].values.CHECKED","line":...,"column":...,"file":...}`, where the message lists the values the template takes. A value the template needs and the row lacks is still the `missing_value` error. A sheet with nothing to report has `"notices":[]`.
- **An inconsistent set is reported.** While a change set has landed on some of the set's sheets and failed on others, the reply carries the set's mark under `inconsistent` ([Change sets](change-sets.md#an-inconsistent-set)). The build builds each sheet as its records are.
- **Building twice gives the same DWGs.** Nothing in a built DWG depends on when, where or on which machine it was built, so the same set builds byte-identical DWGs, in the same folder or another. AutoCAD 2026 read two builds of the demo set as identical.

The reply is one line of JSON. When every sheet was built:

```json
{"ok":true,"result":{"set":"examples/demo-set","units":"in","output":"examples/demo-set/dwg","built":3,"failed":0,"sheets":[{"sheet":"DEMO-001","template":"templates/ansi-b.yaml","ok":true,"result":{"dwg":"examples/demo-set/dwg/DEMO-001.dwg","...":"..."},"notices":[]}]}}
```

Each sheet's `result` is what building one sheet description reports, with its template as `description`. When some sheets failed, the reply is not `ok`, carries the same `result`, and adds an error; the command exits with 1:

```json
{"ok":false,"result":{"built":2,"failed":1,"sheets":[{"sheet":"DEMO-002","template":"templates/ansi-b.yaml","ok":false,"error":{"code":"missing_value","message":"sheet `DEMO-002` has no value `TITLE`, which its template `templates/ansi-b.yaml` fills in at `objects[2].insert.attributes.TITLE`, `objects[3].text.contents`; give it under the row's `values`","field":"rows[1].values.TITLE","line":15,"column":7,"file":"examples/demo-set/rows.yaml"},"notices":[]},"..."]},"error":{"code":"sheets_failed","message":"1 of the set's 3 sheets failed, and nothing was written for them; 2 built. Each sheet's outcome is under `sheets`"}}
```

When the settings or the rows cannot be read, the reply is an error alone, as for one sheet description.

## Errors

Errors take the shape and codes a sheet description's do ([Errors](sheet-description.md#errors)). An error about a row's value is placed at the value in the rows file and names the sheet; an error in a template is placed in the template; an error in the block library, in the library.

| Code | In a set it means |
|---|---|
| `missing_value` | A row lacks a value its template needs. The message names the sheet, the value and where the template puts it; `field` is where the value belongs, such as `rows[1].values.TITLE`. |
| `bad_value` | Besides a sheet description's meanings: a template in another unit than the set, a sheet's name no system allows, a value's name that is not ASCII letters, digits, `_` and `-`, a path that is not relative, or a row's value that the sheet description refuses where the template puts it. |
| `duplicate_name` | Two rows name the same sheet, or names that differ only in case. |
| `sheets_failed` | Some sheets failed and were not written; the others were built. Each sheet's error is under `sheets` in the result. |
| `read_failed` | The folder holds no `set.yaml`, or a file the set names cannot be read. |

## How it is read

`crates/agentdrafter-core/src/description/set.rs` reads the settings and the rows, `template.rs` fills a template in from a row and hands it to the sheet description's reader with the row's own objects, and `set_schema.rs` writes the three schemas. `crates/agentdrafter-core/src/set.rs` builds a set, sheet by sheet, through the same code that builds one sheet description.

## What is not offered yet

- Paper-space layouts and viewports: a sheet is drawn in model space.
- More than one sheet in one DWG.
- Stamping an output with its revision. A set's sheets plot to PDF with `agentdrafter plot <folder>` ([plotting](plot.md#where-it-goes)), each stamped with its template, its rows file and its block library.
- The journal and undo across the set. Stable IDs, [queries](query.md) and [change sets](change-sets.md) across the set are built.
- A value from the row spliced into other text, such as `SHEET {n} OF {m}`, or put anywhere but an attribute's value or a text's contents, such as a position or a layer; and a default for a value a row leaves out.
- More than one rows file in a set.
- One block library enforced for the whole set: each template names its own, in the set's unit.
- Checking a set without building it.
- Rendering a sheet from its template and row to PNG. A set's sheet renders as any DWG does, with `agentdrafter render <output>/<sheet>.dwg` ([Rendering](render.md)), and its PNG's stamp names the DWG; `render` refuses a template, since it holds values from a row.
