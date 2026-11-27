# The drawing set

A drawing set is a job's sheets together with the block library, templates and data they share (see [`CONTEXT.md`](../CONTEXT.md)). On disk it is a folder: the set's settings, its templates, its block library and its rows, in one rows file or [several](#several-rows-files). `agentdrafter build <folder>` builds every sheet in it, one DWG per row, each a template plus that row of data. Building one sheet description on its own works as it always has.

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
paper: ANSI B landscape
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

`examples/demo-plant/` is the demo plant's loop sheets, a larger set built the same way: eight sheets from two templates, one for loops of two wires and one for three, where each row gives its loop's values and holds its own two objects, its field device and its I/O channel, since their blocks differ from loop to loop and a template names its blocks itself. CI builds, plots and checks it on Windows, Linux and macOS.

## The settings

The settings are the file `set.yaml` at the top of the set's folder. `agentdrafter build` finds the set by it.

| Key | Required | What it is |
|---|---|---|
| `id` | no | Optional stable identifier for this drawing set (1 to 128 ASCII letters, digits, `.`, `_` or `-`, starting with a letter or digit, such as `unit-4-loops`). Recorded in its DWGs' build marks and its PDFs' stamps so sets sharing an output folder (including through links or junctions) never claim each other's outputs (#234). |
| `units` | yes | The set's declared unit: `mm`, `cm`, `m`, `in` or `ft`. Every template, sheet description and block library in the set is in it, and every DWG the set builds is in it. There is no default. |
| `output` | yes | The folder the set's outputs go to, relative to the set's folder, such as `dwg` or `../issued`: its DWGs, and its PDFs when it is plotted. A build or a plot makes it if it is not there. Each DWG names the set in its build mark, so a render or a plot of it alone finds the set wherever this folder is ([below](#the-build-mark-names-the-set)). |
| `rows` | yes | The file the set's rows are in, relative to the set's folder, such as `rows.yaml`; or a folder of rows files, such as `rows`; or a list of such paths, such as `[rows/areas.yaml, rows/schedules.yaml]` ([Several rows files](#several-rows-files)). |
| `clean` | no | `true` or `false`: whether every build and plot of the set moves its [leftover outputs](#leftover-outputs) into `superseded/` in the output folder. Without it, or `false`, they are only listed. A run's own `--clean` or `--no-clean` overrides it for that run. |
| `plot_style` | no | The default CTB or STB plot style file name (such as `monochrome.ctb` or `plant.stb`) used when plotting the set, found in the plot style folders. A run's own `--plot-style` overrides it. |
| `format` | no | The drawing format the set builds: `dwg` (the default), `dxf` (ASCII DXF) or `dxf_binary` (binary DXF). A run's own `--format` overrides it. |
| `dwg_version` | no | The DWG version the set builds: `AC1032` (`2018`, the default), `AC1027` (`2013`), `AC1024` (`2010`), `AC1021` (`2007`), `AC1018` (`2004`), `AC1015` (`2000`), `AC1014` (`R14`), `AC1012` (`R13`), or `AC1009` (`R12`). A run's own `--dwg-version` overrides it. See [Converting DWG versions](convert.md). |

A path in a set is written as a sheet description writes its block library's: relative, with `/` between folders on every system, not starting with `/`, and holding none of `\ : < > " | ? *`.

## The rows

The rows file holds one key, `rows`: a list of rows, one per sheet, in the order the set builds them. A row takes:

| Key | Required | What it is |
|---|---|---|
| `sheet` | yes | The sheet's name, which names its DWG: `<output>/<sheet>.dwg`. |
| `template` | yes | The sheet's template: the path of its file, relative to the set's folder. |
| `values` | no | The values the template takes from this row: a mapping from each value's name to its text. One for each `{row: NAME}` the template writes; a value the template does not use is reported as a notice, and the sheet is still built (see [Building](#building)). A value that is a bare null, such as `REV:` or `REV: null`, is left out, as if the row did not give it ([the YAML subset](sheet-description.md#the-yaml-subset), rule 8); empty text is written `''`. |
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
- **IDs.** Each `id` is given once on the sheet, the template's objects and the row's together, so `DEMO-002/tank` is unique in the set. An object without an `id` takes its place in the rows file its row is in, such as `DEMO-002/rows[1].objects[1]`.
- **Errors** in a row's own object are placed in the rows file its row is in, at the object's place, such as `rows[1].objects[0].circle.layer`.

### Several rows files

A set's rows may be in more than one rows file, so that each kind of sheet keeps its own rows: the area sheets in one file and the schedules in another, say. This was decided on 2026-11-19 (#103). `examples/two-kinds/` is a made-up set of two kinds of sheet, drawn from two templates, with a rows file for each:

```
examples/two-kinds/
  set.yaml                   the set's settings
  blocks.yaml                the block library
  rows/areas.yaml            the area sheets' rows
  rows/schedules.yaml        the schedule sheets' rows
  templates/area.yaml        the area sheets' template
  templates/schedule.yaml    the schedules' template
```

```yaml
# set.yaml
units: in
output: dwg
rows:
  - rows/areas.yaml
  - rows/schedules.yaml
```

- **`rows` takes one path or a list.** One path, as before, is a rows file or a folder of them; a list holds one or more such paths, and an empty list is refused. Each rows file is written as one always has been, with [its schema](#the-rows), `agentdrafter schema rows`.
- **A folder gives its `.yaml` files**, not those of the folders in it and no other kind of file, in the byte order of their names, so that Windows, Linux and macOS list them alike: `Z.yaml` before `a-2.yaml` before `a_1.yaml`. `rows: rows` builds the two-kinds set as its list does. A folder that holds no `.yaml` file is refused.
- **The set's rows are each file's rows in turn**, in the order the settings give them, and that is the order the set builds, plots, checks and shows its sheets in.
- **A sheet's name is given once in the set**, across its rows files as in one. A name given in two files is the `duplicate_name` error, placed at the second and naming the first: its file, its line and its row.
- **No file is reached twice**, by two paths or by a path and a folder that holds it: that is the `duplicate_name` error at the second path.
- **A list's paths and a folder are plain files and folders in the set's folder**, since a change set writes a rows file where it is. One that leads outside it, such as `../shared/rows.yaml`, is refused with `bad_value`, as written or as the file system resolves it. So is a rows file a list or a folder reaches through a link, a symbolic link or a junction on Windows: the file itself, a `.yaml` link in a rows folder, or any folder on its way inside the set, the folder a list or `rows` names among them, whether the link leads outside the set or inside it. One path that names a file, as the settings of a set of one rows file write it, is read wherever it leads, as it always has been.
- **A path is listed once.** A list that names the same path twice, such as `[rows/areas.yaml, rows/areas.yaml]`, is the `duplicate_name` error at the second, and the schema refuses it too (`uniqueItems`).
- **Each row belongs to its rows file.** An error in a row names its file and its place there, such as `rows[0].values.REV` in `rows/schedules.yaml`; a row's own object without an `id` takes its place in its file, so the second area's label is `AREA-102/rows[1].objects[1]`; a query says which file an object is written in; a change set writes each edit to the rows file its row is in ([Change sets](change-sets.md#where-a-change-lands)); and a sheet's PNG and PDF are stamped with its own rows file. Its DWG's [build mark](#the-build-mark-names-the-set) names that rows file too, so a render or a plot of the DWG on its own reads that file alone.
- **A set of one rows file is as it was**, byte for byte: its settings, replies, errors, records, build marks, stamps and journal.

## Templates

A template is a sheet's border, title block and fixed content, waiting for data. It is written with what a sheet description holds, and where a sheet's value comes from its row, it writes `{row: NAME}` in place of the value:

- as the value of an insert's attribute, such as a title block's `TITLE: {row: TITLE}`;
- as a `text`'s or an `mtext`'s `contents`;
- as a table cell's text: the cell itself, `{row: NAME}`, or its `text`, `{text: {row: NAME}, height: 3}`;
- as a multileader's content's `text`, `content: {text: {row: NAME}}`, or an older leader's annotation's;
- as an attribute value of a multileader's block content, `content: {block: TAG_BALLOON, attributes: {TAG: {row: NAME}}}`, as an insert's.

The same value can be used in more than one place, and a text's contents or an attribute's value may join values from the row with other text ([below](#a-rows-value-inside-other-text)). Each sheet is the template with every `{row: NAME}` replaced by its row's value `NAME`, and is built as a sheet description is.

- **Units.** A template states `units`, as every sheet description does, and it must be the set's. A block library it names must be in the same unit.
- **Block library.** A template names its block library under `block_library`, relative to the template's own folder, as a sheet description does.
- **Paper size.** A template declares its sheet's paper under `paper`, as a sheet description does ([Paper](sheet-description.md#paper)), and [the checker](check.md) holds each sheet to it; the set's settings take none. A sheet is still as large as what its template draws, and a build, a render and a plot are the same with `paper` or without it: the demo set's template declares `paper: ANSI B landscape` and draws its trim line at 17 by 11 inches from the origin, and the usual case is one DWG per sheet, drawn in model space with its border.
- **A layout.** A template may be a sheet drawn in a [paper-space layout](sheet-description.md#layouts-and-viewports): its border, title block and viewports in paper space, `space: paper`, and the drawing in model space. Each sheet built from it is drawn in that layout, and a row's own objects are in model space unless they say `space: paper`, so a row can hold the drawing a template's viewports show. A template may be drawn in [several layouts](sheet-description.md#several-layouts) too, under `layouts`: each sheet built from it is one DWG holding every layout, a row's own objects naming the layout whose paper space they go in, and a plot of the set plots each sheet to one PDF of a page for each layout.
- **A sheet drawn by hand.** A template that writes no `{row: NAME}` is a sheet description. A set can build one as a row with no values.
- **IDs.** A template's objects are on every sheet built from it, so each sheet's object's [ID](sheet-description.md#ids-and-names) is the sheet's name joined to the `id` the template gives the object, or to its place in the template: the demo set's border is `DEMO-001/border` on its first sheet and `DEMO-002/border` on its second, and its trim line, which has no `id`, is `DEMO-001/objects[0]`. Each sheet's name is given once in a set and each `id` once in its template, so every ID is unique in the set.

A sheet description built on its own takes no value from a row: `{row: NAME}` in it is an error that says to build the set's folder or to write the value.

### A row's value inside other text

A text's or an mtext's `contents`, or the value of an insert's attribute, may be a **list of parts**: plain text and values from the row, joined in order. Zach decided on 2026-11-19 (#102) that this is how a value from the row stands inside other text:

```yaml
  - text: {contents: ['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}], position: [1, 7.4, 0], height: 0.25, id: heading}
  - insert:
      block: CARD_TITLE
      position: [10.5, 0.5, 0]
      attributes:
        TITLE: [{row: AREA}, ' I/O PANEL ', {row: PANEL}]
        DWG_NO: {row: DWG_NO}
```

A row with `PANEL: P1` and `SLOT: '3'` draws the heading `PANEL P1 SLOT 3`. `examples/panel-set/` is a made-up set built this way.

- **The whole-value form stays.** `{row: NAME}` alone is the whole value, as before.
- **Nothing inside a string is read for values.** A brace or a backslash in a part is text and shows as written, as in any `mtext` ([Text](sheet-description.md#text)), so nothing needs escaping.
- **A list holds one value from the row at least.** A list with none is refused: write the text as one string. Each item is plain text or `{row: NAME}`; a null, a list or any other mapping there is an error.
- **The joined text is held to the rules of its place**: a text's contents must show and are one line, an attribute's value is one line. A fixed part of a text's contents or an attribute's value that holds a line break, or another control character, is refused at that part, since no row could make the text build; an mtext's parts may hold line breaks. An error in the joined text is placed at the sheet's first value the list holds, in the rows file, and names the list.
- **An attribute's list depends on its block.** A tag the insert's block defines once takes a list of parts; a tag it defines more than once takes a list of its values, one for each occurrence, as before ([A tag defined more than once](sheet-description.md#a-tag-defined-more-than-once)), each of which may be `{row: NAME}`. A list of parts for one occurrence of a repeated tag is not offered yet.
- **Only these places take a list**: a text's and an mtext's contents, and an insert's attribute values. A table cell, a multileader's content, its block content's attributes and an older leader's annotation take `{row: NAME}` whole, as before.
- **A row lacking a value** one part needs is the `missing_value` error, naming each place the template puts it.
- **A row's own objects, and a sheet description built on its own**, take no value from a row, in a list or whole: a list of parts holding `{row: NAME}` there is the same error as `{row: NAME}`, placed at that item.

A change set edits such a value by matching the new text to the fixed parts, and sets each row value it holds in that sheet's row, or sets one part alone with `row_value` ([Change sets](change-sets.md#where-a-change-lands)). A query finds a sheet's text and attribute values as they are joined.

## Building

`agentdrafter build <folder> [--overwrite] [--clean | --no-clean] [--fonts <folder>]...` reads the settings and the rows, makes the output folder, then builds each row's sheet in turn into `<output>/<sheet>.dwg`, in the 2018 format, with the build mark, which names the set ([below](#the-build-mark-names-the-set)). `--output` is refused for a set: its settings say where its DWGs go.

- **Each sheet is built fully or not at all** (ADR-0005). A sheet that fails, because its row lacks a value, its template has a mistake or its DWG cannot be written, writes nothing; a DWG an earlier build left at its path stays as it was.
- **The other sheets are still built.** One bad row does not stop a long run. The reply says for each sheet what was built and what failed.
- **Settings or rows that cannot be read stop the set** before anything is written, since without them there is nothing to build.
- **An existing DWG is replaced only when asked.** Without `--overwrite`, a sheet whose DWG is already there fails with `output_exists`. With it, each sheet replaces only a DWG that carries AgentDrafter's build mark; any other file may be an original, so that sheet fails with `not_a_build` and the file is not touched.
- **A value the template does not use is reported, not refused.** Zach decided this on 2026-11-11. The sheet is built without it, and its outcome's `notices` lists each such value, so a stray or misspelt one stays in sight: `{"code":"unused_value","message":...,"sheet":"DEMO-003","template":"templates/ansi-b.yaml","field":"rows[2].values.CHECKED","line":...,"column":...,"file":...}`, where the message lists the values the template takes. A value the template needs and the row lacks is still the `missing_value` error. An attribute tag written in lowercase, in the block library or an insert, is taken in capitals and reported the same way, with the code `tag_in_capitals` and the file it is written in ([the block library](block-library.md#a-tag-defined-more-than-once)). A sheet with nothing to report has `"notices":[]`.
- **An inconsistent set is reported.** While a change set has landed on some of the set's sheets and failed on others, the reply carries the set's mark under `inconsistent` ([Change sets](change-sets.md#an-inconsistent-set)). The build builds each sheet as its records are.
- **Outputs no row writes any more are listed**, and moved aside when asked ([below](#leftover-outputs)).
- **Building twice gives the same DWGs.** Nothing in a built DWG depends on when, where or on which machine it was built, so the same set builds byte-identical DWGs, in the same folder or another. AutoCAD 2026 read two builds of the demo set as identical. The one thing a DWG holds of where it is, the way back to the set in its build mark, is worked out from the set's `output` as written, so it is the same wherever the set is built; for an output folder beside the set, such as `../issued`, it holds the set's folder's name too ([below](#the-build-mark-names-the-set)).

The reply is one line of JSON. When every sheet was built:

```json
{"ok":true,"result":{"set":"examples/demo-set","units":"in","output":"examples/demo-set/dwg","built":3,"failed":0,"sheets":[{"sheet":"DEMO-001","template":"templates/ansi-b.yaml","ok":true,"result":{"dwg":"examples/demo-set/dwg/DEMO-001.dwg","...":"..."},"notices":[]}],"leftovers":[]}}
```

Each sheet's `result` is what building one sheet description reports, with its template as `description`. When some sheets failed, the reply is not `ok`, carries the same `result`, and adds an error; the command exits with 1:

```json
{"ok":false,"result":{"built":2,"failed":1,"sheets":[{"sheet":"DEMO-002","template":"templates/ansi-b.yaml","ok":false,"error":{"code":"missing_value","message":"sheet `DEMO-002` has no value `TITLE`, which its template `templates/ansi-b.yaml` fills in at `objects[2].insert.attributes.TITLE`, `objects[3].text.contents`; give it under the row's `values`","field":"rows[1].values.TITLE","line":15,"column":7,"file":"examples/demo-set/rows.yaml"},"notices":[]},"..."],"leftovers":[]},"error":{"code":"sheets_failed","message":"1 of the set's 3 sheets failed, and nothing was written for them; 2 built. Each sheet's outcome is under `sheets`"}}
```

When the settings or the rows cannot be read, the reply is an error alone, as for one sheet description.

### Leftover outputs

When a set's rows change, a sheet can drop out of them: renamed, merged or taken out. Its DWG and PDF would stay in the output folder from the last build, look current, and could be issued by mistake. Zach decided on 2026-11-20 (#109) that every build and every plot of a set lists them, and moves them aside only when asked.

```
agentdrafter build <folder> [--clean | --no-clean] ...
agentdrafter plot <folder> [--clean | --no-clean] ...
```

- **A leftover output is a DWG or a PDF in the set's output folder that AgentDrafter made for this set, and that no row writes any more.** A build and a plot each list every one, DWGs and PDFs both, under `leftovers` in the reply, in the byte order of their names. A row writes `<output>/<sheet>.dwg` and `<output>/<sheet>.pdf`, and a file is the row's when it is the very file at that path, as the file system tells it (by its device and number on Unix, by its path as Windows resolves it), however its name compares: where the file system ignores letter case, or takes two spellings of an accented letter for one, as macOS does, a file left under the old spelling is the row's own and stays; where it tells them apart, as Linux does, the file under the old spelling is another file, and left over. Where a file cannot be told apart that way, a name that is a row's in any letter case is the row's, the safe way round. A row whose sheet failed in this run still writes it, so its file is not a leftover.
- **Made for this set** is told by what the file carries, never by its name alone (#234):
  - a DWG, by its [build mark](#the-build-mark-names-the-set): the mark records the sheet name (`sheet=<sheet>`) — so a copy saved under another file name in the output folder is never mistaken for a leftover — and either the set's explicit `id` (`id=<id>`) or the relative way back to the set (`set=...`). When a DWG carries no explicit `id`, its relative `set=` path must also resolve unambiguously from the physical output folder to this set's folder; outputs in a linked/shared folder without explicit `id`s are treated as ambiguous and left untouched.
  - a PDF, by its [stamp](plot.md#the-stamp): every set plot records `set` (`id`, relative `path`, and `sheet`) in its stamp, following the same sheet-name, explicit `id`, and unambiguous physical-path rules so sets sharing an output folder or an external rows file — or an outer set after a nested set moves — cannot claim each other's PDFs. Legacy PDFs plotted without `set` in their stamp are recognised only when their stamped rows file sits inside the set's own folder and the set has no explicit `id`.
- **Anything else is never listed or moved**: a file with no build mark or no stamp, an [original](../CONTEXT.md) among them; a DWG built, or a PDF plotted, from a sheet description on its own; another set's outputs; any other kind of file, such as a PNG; a link, whatever it leads to; and anything in a folder inside the output folder, `superseded/` among them (ADR-0004).
- **Without cleaning they are only listed**, and stay where they are, as they were.
- **Cleaning moves each one into `superseded/`** in the output folder, keeping its name, and never deletes it. The build or plot makes the folder when there is a leftover to move and it is not there. Each leftover's `moved_to` says where it went.
- **A move never replaces a file**, not even one another program makes at that name while the move is made: the leftover takes its new name as a hard link, which every system refuses to make over a file, and loses its old one after; where the file system has no hard links, its bytes are copied into a file made new, which is refused the same way, and the old one goes once the copy is whole. Just before it moves, each leftover is read again: one that is no longer a plain file, or no longer carries the set's mark or stamp, is not moved, and says so.
- **A `superseded` that is a link, or a junction on Windows, is refused**, since it would move the leftovers to another folder: nothing is moved, and each leftover carries the error. So is a `superseded` that is a file.
- **Cleaning is asked for by the run, or by the set.** `--clean` (`clean: true` through the HTTP API, the MCP server and the Python library) cleans for that run; `--no-clean` (`clean: false`) only lists for that run, whatever the settings say. Without either (or with `clean: null`), the run does as the set's settings say: `clean: true` cleans, and `clean: false` or no `clean` only lists. `--clean` and `--no-clean` together, or either twice, are refused. `clean` is for a set's folder alone: given for one sheet description or DWG, it is refused, since that run writes one file and leaves every other as it is.
- **A name already in `superseded/` is kept.** A sheet taken out, cleaned, put back and taken out again finds its first leftover there: that file is never replaced, and the newer one takes the first free name `<sheet> (2).dwg`, then `<sheet> (3).dwg`, and so on, so the plain name is the oldest. The same goes for PDFs.
- **A leftover that cannot be moved stays where it is**, with its `error`, and the reply is not `ok`, with the error `write_failed`. A sheet's leftover outputs move together: if one (such as a PDF held open on Windows) cannot be moved, its pair is kept with it rather than split into `superseded/` (#236). A sheet name near the longest allowed, 251 characters, that would be too long for the system with its number is such a case.
- **A look that cannot finish says so.** When the output folder cannot be listed, to its end, the reply has no `leftovers`, so that no one takes an empty list for a folder looked through, and says why under `leftovers_error`, an error `read_failed` naming the folder; the reply is not `ok`, with that error. A DWG or PDF no row writes that cannot be read to tell whether the set made it, its details or its bytes, is listed under `leftovers` with `"unreadable": true` and its `read_failed` error, and never moved; the reply is not `ok`, with a `read_failed` error that counts them. A file that can be read but carries no mark or stamp AgentDrafter can read is not the set's, and is left alone.
- **Which error a reply carries**: `sheets_failed` when sheets failed; else `read_failed` when the folder could not be listed or a file could not be read; else `write_failed` when a leftover could not be moved. Each leftover carries its own error whatever the reply's is.
- **When the set cannot be read**, its settings or any of its rows files, nothing is listed or moved, since without every row there is no telling which outputs no row writes.

```json
{"ok":true,"result":{"set":"examples/demo-set","...":"...","leftovers":[{"file":"examples/demo-set/dwg/DEMO-004.dwg","kind":"dwg","sheet":"DEMO-004","moved_to":"examples/demo-set/dwg/superseded/DEMO-004.dwg"},{"file":"examples/demo-set/dwg/DEMO-004.pdf","kind":"pdf","sheet":"DEMO-004","moved_to":"examples/demo-set/dwg/superseded/DEMO-004.pdf"}]}}
```

A reply with nothing left over has `"leftovers":[]`. One whose folder could not be listed:

```json
{"ok":false,"result":{"set":"examples/demo-set","...":"...","leftovers_error":{"code":"read_failed","message":"could not list the set's output folder to look for leftover outputs, so none was looked for or moved: ...","file":"examples/demo-set/dwg"}},"error":{"code":"read_failed","...":"..."}}
```

Moving a leftover is not a change set: it is not in the [journal](journal.md) and is not undone; the files are in `superseded/`, byte for byte, to be moved back by hand. A set's build or plot holds an exclusive lock on its output folder across writing and leftover cleanup, re-checks each leftover's file identity and current-output status before moving it, and uses an atomic no-replace rename so concurrent builds or plots of the same set cannot remove a freshly published output (#238).

### The build mark names the set

Zach decided on 2026-11-11 that **a DWG's build mark records the folder of the set it was built from, relative to the DWG, and never as a path in full**, since these DWGs go to plants. The mark is the custom drawing property `AgentDrafter` that every build writes ([the sheet description](sheet-description.md)), which AutoCAD shows under DWGPROPS; a sheet of a set records its optional `id=<id>` (when `set.yaml` gives `id`), its sheet name (`sheet=<sheet>`), its optional rows file (`rows=<rows>` for multi-file sets), and `set=<path>` last, such as `build; sheet=DEMO-001; set=..` for `examples/demo-set/dwg/DEMO-001.dwg`, or `build; id=unit-4; sheet=DEMO-001; set=../demo-plant`. Nothing else is added to the DWG: no registered application, no object.

A sheet of a set whose rows are in [more than one rows file](#several-rows-files) names its own rows file in the mark as well, relative to the set's folder, before `set=`: `build; sheet=SCHED-201; rows=rows/schedules.yaml; set=..` for `examples/two-kinds/dwg/SCHED-201.dwg`. Older DWGs carrying `build; set=...` or `build; rows=...; set=...` continue to be read as before. A render, a plot and `stale` of that DWG on its own read that rows file alone, so another of the set's rows files that cannot be read does not touch it. The file is found among the set's rows files by the file it leads to, however the settings spell it now, such as `./rows/schedules.yaml`; when it is no longer one of them, every rows file of the set is read, as for a set of one. A path that holds `;`, which would end it in the mark, is not named, and the DWG's set's rows files are read, as for a set of one.

- **The path is the way back from the output folder, as the set writes it**, with `/` between folders on every system: `..` for `output: dwg`, `../../..` for `output: out/a/b`, `.` for `output: .`, and for an output folder beside the set, such as `output: ../issued`, `..` and the set's folder's name. It is worked out from two things alone: `output` as the settings write it, each folder name in it a `..` back; and, for a `..` out of the set, the set's own folder's name, the real one, of the folder the set's path leads to through any link, since that is where `..` from the output folder leads. No other part of any path reaches it, whatever path the set was built by or wherever a link leads, so it holds nothing of where the set is on a machine, and a set's DWGs are the same bytes on Windows, Linux and macOS, whether or not the output folder is a link to a folder elsewhere.
- **A mark never names a folder above the set.** When the way back would, as from `output: ../../issued`, the DWGs name no set: their mark stays `build`, and a render or a plot of one on its own says it was not issued, as before the mark named the set. The build's reply says so under `no_set_in_marks`, with the set's `output`; the set's own plot, stamped from its template, still says whether it was issued. Since a set's paths are relative ([above](#the-settings)), this is the one case.
- **A sheet built on its own names no set**: its mark stays `build`, even when its sheet description is in a set's folder.
- **A change set** builds a sheet's DWG again with the same mark, since it builds it as `build` does.
- **A render, a plot and `stale`** of the DWG on its own find its set from the mark, wherever the DWG is, an output folder outside the set among them, and stamp `issued` as for the set's own files ([the stamp](render.md#the-stamp)). The mark is followed from the DWG's folder as it is given, and failing that from the folder as the file system resolves it, through links. The DWG is from an issued revision when it is the DWG the set writes, `<output>/<sheet>.dwg`, and an accept issued that file with the same bytes, however it is named: in other letter case where the file system ignores case, say. Every command that reads the set, such as rolling back a commit that never finished ([the journal](journal.md#a-commit-that-never-finished)), finds it the same way.
- **When the set cannot be found**, the stamp says `issued: false` and why, under `why`: the DWG names no set; the set its mark names is not there, since the DWG or the set has been moved apart, or the set taken away; or the folder holds a set that does not write this DWG, since the DWG was copied or moved out of the set's output folder, or the set's output folder changed. The words name the set as the mark writes it, never by its path in full. A set and its output folder moved together are still found.

## Errors

Errors take the shape and codes a sheet description's do ([Errors](sheet-description.md#errors)). An error about a row's value is placed at the value in the rows file and names the sheet; an error in a template is placed in the template; an error in the block library, in the library.

| Code | In a set it means |
|---|---|
| `missing_value` | A row lacks a value its template needs. The message names the sheet, the value and where the template puts it; `field` is where the value belongs, such as `rows[1].values.TITLE`. |
| `bad_value` | Besides a sheet description's meanings: a template in another unit than the set, a sheet's name no system allows, a value's name that is not ASCII letters, digits, `_` and `-`, a path that is not relative, a row's value that the sheet description refuses where the template puts it, a `rows` that lists no file, a folder of rows that holds none, or a list's path or a folder outside the set's folder. |
| `duplicate_name` | Two rows name the same sheet, or names that differ only in case, in one rows file or in two; or the settings reach one rows file twice. |
| `sheets_failed` | Some sheets failed and were not written; the others were built. Each sheet's error is under `sheets` in the result. |
| `read_failed` | The folder holds no `set.yaml`, or a file or folder the set names cannot be read. |

## How it is read

`crates/agentdrafter-core/src/description/set.rs` reads the settings, finds the rows files they name (`read_rows_files_in`) and reads their rows (`rows_of`), each row with its file, `template.rs` fills a template in from a row, joining each list of parts as `parts.rs` reads it, and hands it to the sheet description's reader with the row's own objects, and `set_schema.rs` writes the three schemas. `crates/agentdrafter-core/src/leftovers.rs` finds a set's leftover outputs and moves them into `superseded/`, for a build and a plot alike. `crates/agentdrafter-core/src/set.rs` builds a set, sheet by sheet, through the same code that builds one sheet description, works out the set's folder each DWG's build mark records (`set_in_mark`), and finds the set a DWG's mark names (`set_of_dwg`). `src/dwg.rs` writes the mark, and reads it from the DWG's drawing properties alone, without reading the drawing.

## What is not offered yet

- Stamping an output with its revision. A set's sheets plot to PDF with `agentdrafter plot <folder>` ([plotting](plot.md#where-it-goes)), each stamped with its template, its own rows file and its block library.
- The journal and undo across the set. Stable IDs, [queries](query.md) and [change sets](change-sets.md) across the set are built.
- Numbering worked out across the set, such as `SHEET {n} OF {m}`; a value from the row put anywhere but an attribute's value or a text's contents, such as a position or a layer; and a default for a value a row leaves out. A row's own values inside other text are [built](#a-rows-value-inside-other-text).
- One block library enforced for the whole set: each template names its own, in the set's unit.
- Rendering a sheet from its template and row to PNG. A set's sheet renders as any DWG does, with `agentdrafter render <output>/<sheet>.dwg` ([Rendering](render.md)), and its PNG's stamp names the DWG; `render` refuses a template, since it holds values from a row.
