# AgentDrafter: the plan

Agreed in a grilling session on 2026-11-10. Nothing is built yet. Each line below is a decision Zach made in that session unless it is marked **open**. Terms are defined in [`CONTEXT.md`](../CONTEXT.md); the decisions that are hard to reverse are recorded in [`docs/adr/`](adr/).

## What AgentDrafter is

An open-source CAD engine built for agents first. An agent creates, revises and checks technical drawings with no window open, and a reviewer watches, accepts or undoes. It is its own engine, not a layer on another CAD (ADR-0001). It runs on Windows, Linux and macOS, does 2D first and 3D after, and reads and writes DWG without AutoCAD.

It is a general engine: it knows lines, blocks, attributes, layers and sheets, and knows nothing about any trade (ADR-0002). Trade knowledge lives in generators that call it. Its first two callers are a private controls drawing generator, which will write loop sheets through it, and Vellum's drawing jobs, which will run on it in AutoCAD's place.

The drafting app for people, where someone uses AgentDrafter the way they use AutoCAD, is a later phase on the same core.

## The first usable version

**The job:** generate a set of loop sheets from data. Project data, templates and a block library go in; a drawing set comes out as DWG and PDF, with no AutoCAD in the loop. It is pure generation: nothing is lifted from an existing drawing.

**What it can author:** lines, polylines, arcs, circles, ellipses, splines, text and multiline text, hatch, blocks with attributes, layers, linetypes, and TrueType and SHX text styles, plotted with a CTB plot style. The sheet layout follows the template; the usual case is one DWG per sheet, drawn in model space with its border and plotted to extents.

**What an agent can do in it:**

- draw and edit with no window, through all four interfaces: command line with JSON output, MCP server, Python library, local HTTP API (ADR-0007);
- check its own work: render any sheet to PNG or PDF and query the set (find a tag, list blocks, read attributes, measure);
- read and compare text: sheet descriptions in a strict subset of YAML (ADR-0003);
- work in the reviewer's live session.

**The viewer:** a browser window that opens on the laptop, on forge and on a phone over the tailnet. Its look was picked from a prototype: the drawing fills the screen and updates as change sets land, sheets are tabs across the top, the journal drags up from the bottom, and undo and accept are always in reach. The reviewer pans, zooms, picks and inspects, watches each change land, steps through the journal, and accepts or undoes. It has no drawing tools. Changes land at once on a working revision; nothing is issued until the reviewer accepts.

**Outputs:** DWG in the 2018 format, which is what a new sheet is written in (ADR-0006), and PDF at issue quality: same sheet size, lineweights, monochrome plot style and text widths as AutoCAD's plot of the same DWG, with searchable text.

**Proof that it is done:**

1. A demo plant, made up for the purpose, generates in CI on Windows, Linux and macOS.
2. On Zach's laptop, AgentDrafter regenerates one real past job's loop sheets from its data. Every DWG passes AutoCAD's audit with no errors, each PDF overlays AutoCAD's plot of the same DWG, and Zach accepts the set against what he issued. That job never enters the repository (ADR-0008).

## After the first usable version

In this order:

1. **Revise a plant's DWG from markups.** Open their drawing, apply the marked-up change, cloud it, add the revision line, save a new revision in the version the drawing arrived in, and a PDF. This is the first real test of "untouched means unchanged" (ADR-0004).
2. **Paper-space layouts and viewports.**
3. **Dimensions and leaders.**
4. **Dynamic blocks placed in a chosen state.**

Zach decided on 2026-11-13 that the work after the first usable version follows the ranking in [`docs/capabilities.md`](capabilities.md), which a real controls job's notes shaped: the gaps generating a plant's sheets cost first, then tables, leaders, dimensions and layouts, then revising a plant's DWG. The four steps above are all in it, in that order's places.

Later, order not yet set: carrying part of an old sheet into a generated one; batch edit and query across existing sets; 3D solids in the AutoCAD style (boxes, extrudes, unions and cuts in the same file as the 2D work); the drafting app for people. Every coordinate is 3D from the start so that 3D is not a rewrite.

## Rules of the engine

| Rule | What it means |
|---|---|
| Untouched means unchanged | A revision returns everything AgentDrafter did not edit exactly as it was, including content it does not understand. |
| A revision keeps its DWG version | A revision is saved in the version its original arrived in, because that is the only way content the engine does not understand comes through unchanged. Converting a drawing is a separate step someone asks for. |
| A drawing that cannot be fully decoded is not revised | AgentDrafter stops and names the records it could not decode. It saves only when the reviewer allows it for that one drawing, and the loss is listed in the report saved with the revision. |
| Originals are never overwritten | An existing DWG is read-only. A change is saved as a new revision file beside it, with its change list and a report of anything that changed outside what was asked. |
| Two records | A new sheet's record is its sheet description. An existing drawing's record is its DWG, with a text mirror beside it for reading. |
| The drawing set is the unit of work | A change set can span sheets and is undone as one. Search and consistency run across the set. |
| Each sheet all-or-nothing | A sheet takes its part of a change set fully or not at all. If some sheets fail, the set is marked inconsistent until they are fixed or the change set is undone. Zach decided on 2026-11-11 that a template edit is all or nothing across its sheets: it lands only if every sheet built from the template takes its whole part, and otherwise the template, those sheets' rows and their DWGs stay byte for byte as they were, while sheets built from other templates still land ([`docs/change-sets.md`](change-sets.md)). |
| The working revision is the set's own files | Zach decided on 2026-11-11 that a change set writes the drawing set's records directly, its templates and its rows, and builds again only the DWGs it touched, replacing only DWGs AgentDrafter built, as `build --overwrite` does; an original is never touched. History is the journal's, and git's too when the set is in a repository. Revising existing DWGs is a later step. |
| An edit to a template's object changes the template | Zach decided on 2026-11-11 that every sheet built from a template follows an edit to one of its objects, and the reply lists the sheets it reached. |
| An object added to one sheet goes in its row | Zach decided on 2026-11-11 that a row may hold objects of its own, under `objects`, which are part of that sheet alone for build, render, plot and query; the template stays shared, and the rows' schema accepts exactly what the reader does ([`docs/drawing-set.md`](drawing-set.md#a-rows-own-objects)). |
| A value from the row is set in the row | Zach decided on 2026-11-11 that setting an attribute or a text whose template value is `{row: NAME}` writes that sheet's row value, and the reply names every place on the sheet the value fills. |
| Journal and undo across sessions | Every change set and failed attempt is recorded in the set's folder. A later session can read it and undo from it. Old undo states are pruned. Git is optional. Zach decided on 2026-11-11 that old undo states are pruned so that the 50 newest change sets keep theirs: the journal still records a pruned change set and says its undo state was pruned, and undoing one whose undo state is pruned, or one that needs a later pruned state, is refused with an error that says why. He decided the same day that undo refuses when a file was changed by hand since the change set: it compares each file it would put back with what the journal says the change set left behind, and if any differs, changes nothing and names the files that differ; a file the change set made that someone has since taken away counts as changed ([`docs/journal.md`](journal.md)). |
| Undo is a stack | Undoing a change set also undoes every later one that is not issued. A failed attempt stays in the journal. An issued change set cannot be undone. |
| Accept is blocked on an inconsistent set | The reviewer cannot accept while the set is inconsistent. Failed checks do not block: the reviewer can accept after a warning that lists them. |
| Accept is a command like every other | Zach decided on 2026-11-11 that accept is a command of the registry, reached through the command line, the HTTP API, the MCP server, the Python library and the viewer alike, so an agent can accept when the reviewer asks it to, such as "convert these drawings and accept all revisions automatically" (ADR-0007 stands). It is refused while the set is inconsistent. It checks every sheet and, when checks fail, lists them and issues nothing unless the caller says it has seen them: `despite_failed_checks` in its input, `--despite-failed-checks` on the command line; the viewer asks the reviewer first. It marks every change set in place issued, and an issued change set cannot be undone. The journal records each accept with the interface it came through, `viewer`, `cli`, `http`, `mcp` or `python`, so the reviewer can tell an agent's accept from their own ([`docs/journal.md`](journal.md#accept)). |
| Stable IDs and names | An object keeps one ID for life and can carry a name agents select by. Nothing is selected by coordinates. Zach decided on 2026-11-11 that an object's ID lives in its record: the `id` its sheet description or template gives it, which a generator can write, or else its place in the file; building never rewrites the record ([`docs/sheet-description.md`](sheet-description.md#ids-and-names)). Operations write `id` in as they edit, so an ID stays fixed once records are edited: every object added gets one, and deleting an object writes each later object's place as its `id`. |
| Outputs are stamped | Every PDF, PNG and DWG says which revision produced it. A stale output is flagged. Zach decided on 2026-11-11 that an output says whether it came from an issued revision in its stamp only: the PNG's and the PDF's stamp records whether the revision was issued and which accept issued it, and `agentdrafter stale` reports it; the drawing itself looks the same, with no watermark ([`docs/render.md`](render.md#the-stamp)). |
| Sheet-aware checks by default | Exact paper sizes, nothing off the sheet, no text overflowing its cell, sitting on other text, or with a line through its letters, reported after every change. |
| The checker is separate | It re-reads saved files from disk. Whatever made a change never marks its own work. |
| A sheet's paper is declared | Zach decided on 2026-11-11 that a template, or a sheet description, declares the paper its sheet is drawn on, under `paper`: a width and a height, or a standard ANSI or ISO size by name and the way it lies, such as `ANSI B landscape`. The exact paper size check holds the sheet's extents to it and nothing off the sheet holds every object to it, its lower left corner at the origin, within 0.01 mm on paper. A sheet with no paper declared gets one finding that says so. A DWG is unchanged by it ([`docs/check.md`](check.md)). Zach decided on 2026-11-13 that a plot and a render are drawn on it too: the page is the declared paper, what lies off it cut at its edge, and a sheet that declares none is drawn to its extents as before. A DWG plotted or rendered on its own takes its paper from the set its build mark names ([`docs/plot.md`](plot.md#the-sheet-size)). |
| A title-block cell is declared in the block library | Zach decided on 2026-11-11 that an attribute definition, or a text of a block, may give its cell as a box in the block's own coordinates, under `cell`. No text overflowing its cell holds the text as its font draws it, through the one drawing pipeline, to the cell placed by the insert's position, scale and rotation. Text with no cell is not tested for overflow. Only the checker reads it; a DWG is unchanged by it. |
| Explicit units | Numbers are in the set's declared unit. A value may carry its own unit and is converted. An unknown unit is an error. |
| Operations have their own names | Typed operations such as `insert_block`, with the AutoCAD command name as an alias where one exists: `INSERT`, `MOVE`, `ERASE`, `CHANGE`, `ATTEDIT` and one add per object type, each listed with its schema by `agentdrafter operations`. |
| Scripts are sandboxed and off by default | A script is an escape hatch for an edit no operation covers. It runs as one change set, and only when the reviewer has switched scripts on. |

## Formats

- **DWG:** read every version. A new sheet is written in the 2018 format. A revision is saved in the version it arrived in. A per-job setting converts a set to 2018 or saves it down. No AutoCAD and no converter in the loop.
- **DXF:** read and write.
- **Data rows:** a drawing set's rows are written in YAML, in the same strict subset as sheet descriptions (ADR-0003), with a schema that accepts exactly what the reader accepts. Not CSV. Zach decided this on 2026-11-11; [`docs/drawing-set.md`](drawing-set.md) sets the rows out.
- **PDF:** plot at issue quality with searchable text. **PNG:** render for checking.
- **Fonts:** AutoCAD's SHX fonts are read from a machine that has AutoCAD and are never in the repository; open look-alikes are used elsewhere.
- **3D solids inside DWG:** **open.** AutoCAD stores solids in a format no open-source library writes for curved shapes. This is decided when 3D is designed.

## Platforms and shipping

- Windows, Linux and macOS. Every change builds and passes its tests on GitHub's runners for all three. Hand-proving happens on the Windows laptop and on forge; there is no Mac to hand-prove on.
- The repository is public from the first commit and holds no client material (ADR-0008).
- CI runs on GitHub, never on the laptop.
- **Speed is baselined.** The first build measures a fixed set of numbers on the laptop: one command from a cold start, a 100-sheet set built to DWG and PDF, and a change showing in the viewer. Those become the bars every later pull request reports against. Zach decided on 2026-11-11 that the baseline is measured on the laptop, in one short scripted run of a few minutes, under the judge's lock. He decided the same day that all three numbers are measured there with a Windows `agentdrafter.exe` that CI builds with the viewer's page in, by an optional workflow, `Windows binary`, that runs when asked and is not on the merge bar, since CI's wheel is built without the page. The numbers, the machines they were measured on, forge's numbers beside them and how a pull request reports against them are in [`docs/speed.md`](speed.md).
- **The judge runs on the laptop, on DWG changes and before releases.** AutoCAD exists only there, so the real-drawing checks cannot run in CI. An agent starts them in the background when a pull request touches DWG reading or writing and before each release: one AutoCAD at a time, a memory check first, results as counts. Zach is told when a run starts and what it found.

## The stack

Decided on 2026-11-10, after the DWG bake-off.

| Part | Decision |
|---|---|
| Engine language | Rust (ADR-0009) |
| DWG reading and writing | acadrust, pinned to a commit; fixes go upstream (ADR-0009) |
| Licence | Apache-2.0 for AgentDrafter's own code (ADR-0010) |
| Interfaces | One Rust core. The command line, the MCP server and the HTTP API are thin layers over it, and the Python library is a binding to it (ADR-0007). They take their commands from one registry in the core, so a command reaches every interface at once. The HTTP API listens on the local machine unless told otherwise. Zach decided on 2026-11-11 that a token is needed only when it listens beyond the local machine: on loopback no token is needed; on any other address every request carries a token, made new each time the server starts and printed in the line it starts with, compared in constant time, and a request without it is refused with a JSON error ([`docs/servers.md`](servers.md)) |
| Rendering | One Rust drawing pipeline for the screen, the PNG and the PDF (ADR-0011) |
| Front end | That pipeline compiled to WebAssembly in the page, with TypeScript and React around it. The engine stays on the machine that holds the drawings and streams changes to the page (ADR-0011) |
| Install | One pip package per operating system carries the Rust core, the command, both servers and the Python library: `pip install agentdrafter`, or `uvx agentdrafter` with no install. Plain binaries go on GitHub Releases too. No code signing until the drafting app ships. Nothing is published until Zach says so at the first release |
| Script language | Chosen when scripts are built, by running Starlark, JavaScript and Luau on the same 20 made-up edits and measuring how often agents get them right first time and whether time and memory limits hold |

### The bake-off

Three open DWG libraries each read 53 drawings (made-up ones and real ones under neutral IDs) and wrote them back. AutoCAD 2026 audited every result, and its own reading of each original was compared with its reading of the result. "Drawn objects" are lines, text, block inserts, attributes, hatches and dimensions.

| Run over 53 drawings | Opened clean in AutoCAD | Drawn objects identical | Drawings gaining audit errors |
|---|---|---|---|
| AutoCAD alone, plain save (baseline) | 50 | 99.6% | 0 |
| acadrust, newest code, same version | 52 | 98.4% | 5 |
| acadrust, newest code, converted to 2018 | 44, 8 refused | 95.4% | 5 |
| acadrust, last release, same version | 43 | 97.6% | 6 |
| ACadSharp, converted to 2018 | 40 | 93.8% | 18 |
| LibreDWG | 0 | not scorable | not scorable |

- An acadrust edit stayed in scope: with one attribute or text changed and one line added across 22 drawings, no other drawn object changed.
- A sheet acadrust wrote from scratch opened with no audit errors.
- acadrust's known faults, to fix and send upstream: one real drawing did not reopen, four lost a layout and gained audit errors, one lost its tables, and old-style polylines had vertices renumbered and a few dropped.
- Not covered: an R12 drawing, acadrust built on Windows or macOS (its own CI covers both), and the WebAssembly build in a real browser (it ran under Node with byte-identical output).

## Order of work once Zach says build

1. A prototype of the viewer that opens on his phone, for him to pick from, before any viewer is built for real.
2. Tickets in dependency order, built on branches by agents, each proven on the real surface before it merges.
3. CI from the first test, on Windows, Linux and macOS.
4. The judge moves into the repository as a tool with no drawings in it, so a change to DWG reading or writing can be scored the way the bake-off was.

## What the session found

- **Prior art.** Agent-first CAD exists (AgentCAD and others, mostly for 3D parts), and an open-source AutoCAD-like exists (Open CAD Studio). ADR-0001 says why this is still its own engine.
- **A trial** of AgentCAD on a made-up five-sheet loop set. Worth copying: a failed change left the file byte-identical and the error named the bad step and listed the valid choices; an on-disk journal let a later process undo and restore the exact bytes; named objects made a rename one operation per sheet. What it lacked for this job: DWG without AutoCAD, templates, a shared block library, undo across a set, a default check for objects off the sheet, an exact 11x17 sheet, searchable PDF text, and search by text or attribute value.
- **Research.** Every AutoCAD from 2018 through 2027 saves the same DWG format. The only vendor-grade DWG writers are paid and proprietary. Lessons on what makes a CAD tool usable by agents: render after every edit, offer a dry run, use stable IDs, expose the schema, make units explicit, keep the model as text, check that an edit stayed in scope, and expose undo.
- **A bake-off** of three open DWG libraries with AutoCAD as the judge (see The stack).
- **A census** of a real archive of about 22,000 drawings: nine DWG versions in circulation, a tenth of the files in the current one; loop-sheet content is lines, polylines, arcs, circles, splines, text, hatch and blocks with attributes, nearly all in model space.

## Still open

| Question | Waits on |
|---|---|
| The language scripts are written in, and their sandbox | the comparison, when scripts are built |
| Which desktop shell wraps the page for the drafting app | the drafting app's design |
| Code signing for Windows and macOS | the drafting app shipping |
| Real AutoCAD solids inside DWG | the 3D design |
| The drafting app for people | its own grilling session and prototypes |
