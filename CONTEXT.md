# AgentDrafter

AgentDrafter is an open-source CAD engine built for agents first: an agent creates, revises and checks technical drawings with no window open, and a person reviews the result. This file is the glossary. Write code, documents and messages in these terms.

## Language

### Who does what

**Agent**:
An AI coding agent, such as Claude Code or Codex, that calls AgentDrafter to make or change drawings.
_Avoid_: bot, AI, assistant

**Reviewer**:
The person who looks at what an agent did and accepts or undoes it.
_Avoid_: user, operator

**Generator**:
A program outside AgentDrafter that knows a trade, such as controls, and turns project data into sheet descriptions. AgentDrafter knows drawings; a generator knows the trade.
_Avoid_: plugin, toolkit

**Judge**:
AutoCAD run with no window to audit a DWG that AgentDrafter wrote. It is used in tests only and is never part of a job.
_Avoid_: oracle, reference CAD

### Drawings

**Drawing**:
One CAD file, DWG or DXF.
_Avoid_: document, model

**Sheet**:
One titled page that prints: a border, a title block and the content inside them.
_Avoid_: page, layout (a layout is AutoCAD's paper-space tab, which may hold a sheet)

**Drawing set**:
A job's sheets together with the block library, templates and data they share. It is the unit AgentDrafter works on: a change can span its sheets and is undone as one.
_Avoid_: project, package, workspace

**Object**:
Any single thing in a drawing: a line, a piece of text, a block insert.
_Avoid_: entity, element, shape

**Layer**:
A named group of objects in a drawing, as in AutoCAD. An object sits on one layer and draws with the layer's colour, linetype and lineweight.
_Avoid_: level, class

**Linetype**:
A named pattern of dashes, gaps and dots that a layer draws its objects with, such as `HIDDEN`. Every linetype but `Continuous` comes from a definition in the sheet description.
_Avoid_: line style, dash pattern

**Lineweight**:
The pen width a layer plots with, one of AutoCAD's standard widths in millimetres. It is a width on paper, so it is not in the declared unit.
_Avoid_: line width, pen size, thickness (a thickness is AutoCAD's extrusion of an object)

**Text style**:
A named font that text draws with, and how wide its characters are, as in AutoCAD. The font is an AutoCAD SHX font such as `romans.shx` or a TrueType font such as `arial.ttf`. Every drawing has the style `Standard`.
_Avoid_: font style, text format

**Font folder**:
A folder AgentDrafter reads fonts from when it measures text, named by a setting, such as AutoCAD's `Fonts` folder on a machine that has AutoCAD. There is no default.
_Avoid_: font path, font directory

**Look-alike**:
An open font AgentDrafter measures text with when a text style's font is not in any font folder, such as Hershey's Roman Simplex in place of `romans.shx`. The drawing still names the style's own font, and the build says which look-alike it used.
_Avoid_: fallback font, substitute font

**Block**:
A named group of objects defined once and inserted many times, as in AutoCAD.
_Avoid_: symbol, component, cell

**Attribute**:
A labelled text field on a block that is filled in at each insert, such as a tag or a terminal number.
_Avoid_: property, field, parameter

**Block library**:
The blocks a drawing set shares, each defined once for the whole set.
_Avoid_: symbol library, catalog

**Template**:
A sheet's border, title block and fixed content, waiting for data.
_Avoid_: prototype, master

**ID**:
An object's identifier, fixed for the object's life.
_Avoid_: handle (a handle is the DWG's own identifier), index

**Name**:
A label given to an object so an agent can select it later, such as `jb`. Objects are selected by ID or name, never by coordinates.
_Avoid_: tag (a tag is what an instrument or device is called on the drawing), label

### New sheets and existing drawings

**Record**:
The file that is the truth for a sheet. For a new sheet it is the sheet description; for an existing drawing it is the DWG.
_Avoid_: source of truth, master file

**Sheet description**:
The text file that states what is on a new sheet: its template, its blocks with their attribute values, its lines and its notes. A sheet can be a template plus one row of data.
_Avoid_: script, spec, source file

**Build**:
Producing a DWG from a sheet description.
_Avoid_: compile, render, export

**Build mark**:
A mark written inside every DWG that AgentDrafter builds, so that a later build knows the file is its own. A build replaces an existing DWG only when it carries the mark; any other file may be an original and is never overwritten.
_Avoid_: stamp (a stamp records the revision an output came from), signature, watermark

**Declared unit**:
The unit a sheet description states for its numbers, such as `mm`. A number with no unit of its own is in the declared unit; a number that carries its own unit is converted to it. There is no default unit.
_Avoid_: default unit, base unit

**Original**:
A DWG that someone else drew, such as a plant's drawing. It is read-only to AgentDrafter and is never overwritten.
_Avoid_: source drawing, input file

**Revision**:
A new DWG saved beside an original, holding the original plus the changes made to it. It is saved in the DWG version the original arrived in.
_Avoid_: version, copy, edit

**Change list**:
The recorded operations that turn an original into a revision.
_Avoid_: patch, diff, edit log

**Text mirror**:
A readable text rendering of an existing DWG, kept beside it for reading and comparing. It is never the record.
_Avoid_: dump, export

**Untouched means unchanged**:
The fidelity rule for a revision: everything AgentDrafter did not edit comes back exactly as it was in the original, including content AgentDrafter does not understand. An original that cannot be fully read is not revised unless the reviewer allows it for that drawing.
_Avoid_: round-trip safe, lossless

### Changing and checking

**Operation**:
A named, typed action on a drawing, such as `insert_block`. Where AutoCAD has the same command, the operation carries the AutoCAD name as an alias.
_Avoid_: command (the word for what a person types), tool, call

**Change set**:
Operations sent together as one request. Each sheet takes its part of a change set fully or not at all.
_Avoid_: batch, transaction, commit

**Journal**:
The on-disk record of every change set and every failed attempt. A later session reads it to see what was done and to undo it.
_Avoid_: history, log, undo stack

**Inconsistent set**:
A drawing set in which a change set landed on some sheets and failed on others. It stays marked until the failed sheets are fixed or the change set is undone as one.

**Working revision**:
The state an agent's changes land on at once. It is not issued.
_Avoid_: draft, staging

**Issued**:
Accepted by the reviewer. Nothing is issued until the reviewer accepts the working revision.
_Avoid_: released, approved, final

**Check**:
A rule a sheet is tested against, such as nothing off the sheet or no text overflowing its title-block cell.
_Avoid_: lint, validation rule

**Checker**:
The part of AgentDrafter that re-reads saved files and reports failed checks. It is separate from whatever made the change.
_Avoid_: validator, linter

**Output**:
A PDF, PNG or DWG produced from a revision and stamped with the revision it came from. An output whose revision has moved on is stale.
_Avoid_: export, artifact, plot

**Script**:
Code an agent sends to run inside AgentDrafter for an edit no operation covers. It runs sandboxed as one change set and only when the reviewer has switched scripts on.
_Avoid_: macro, plugin

### Reaching AgentDrafter

**Core**:
The Rust library that knows drawings. Every interface is a thin layer over it, and it reads and writes DWG through acadrust behind one seam.
_Avoid_: backend, server

**Interface**:
One of the four ways in: the command line, the MCP server, the Python library and the local HTTP API. All four expose the same operations with the same behaviour.
_Avoid_: door, adapter, front end

**Viewer**:
The browser window where the reviewer watches a drawing set change, steps through its journal, and accepts or undoes.
_Avoid_: app, editor, UI

### Material

**Client material**:
Any plant's drawing, block, tag or name. It never enters this repository, including in tests, fixtures, screenshots and notes.

**Demo plant**:
The made-up plant whose data, blocks and sheets live in this repository for tests and examples.
_Avoid_: sample project, fixture plant
