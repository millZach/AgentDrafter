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

**Drawn object**:
What the judge counts when it compares AutoCAD's reading of an original with its reading of a revision: the records in the drawing's ENTITIES and BLOCKS sections, such as lines, text, block inserts, attributes, hatches and dimensions. One is identical when AutoCAD reads it with the same handle and the same content before and after.
_Avoid_: entity count, geometry

**Layer**:
A named group of objects in a drawing, as in AutoCAD. An object sits on one layer and draws with the layer's colour, linetype and lineweight, unless it gives its own. A layer may be off or frozen, so that its objects draw nothing, locked, or set not to plot, so that they are left out of a PDF.
_Avoid_: level, class

**ByLayer**:
An object's colour, linetype or lineweight taken from its layer, as AutoCAD's ByLayer is. Each of an object's is ByLayer unless it gives its own.
_Avoid_: default, inherit

**ByBlock**:
An object's colour, linetype or lineweight taken from the insert that draws the object's block, as AutoCAD's ByBlock is, so one block draws in different colours on different inserts. On the sheet itself, outside any block, it draws white, continuous and at the default lineweight.
_Avoid_: from parent

**Linetype scale**:
What a linetype's dashes and gaps are enlarged by where an object draws them: the sheet's `ltscale`, AutoCAD's LTSCALE, times the object's own `linetype_scale`. An insert's scale does not enlarge them.
_Avoid_: dash scale, pattern scale (a hatch's scale enlarges its pattern)

**Linetype**:
A named pattern of dashes, gaps and dots that a layer, or an object of its own, draws with, such as `HIDDEN`. Every linetype but `Continuous` comes from a definition in the sheet description.
_Avoid_: line style, dash pattern

**Lineweight**:
The pen width a layer, or an object of its own, plots with, one of AutoCAD's standard widths in millimetres. It is a width on paper, so it is not in the declared unit.
_Avoid_: line width, pen size, thickness (a thickness is AutoCAD's extrusion of an object)

**Bulge**:
How a polyline's segment curves, as AutoCAD keeps it: the tangent of a quarter of the included angle of its arc, positive anticlockwise; 0 is straight and 1 a half circle. A sheet description may give the arc's included angle instead, as `arc_angle`.
_Avoid_: curvature, arc factor

**Donut**:
A ring, or a filled dot when its inside diameter is 0, as AutoCAD's DONUT draws it: a closed polyline of two half circles, as wide as the ring, such as a wire's junction dot.
_Avoid_: dot, ring, filled circle

**2D solid**:
A filled triangle or quadrilateral, AutoCAD's SOLID object. A sheet description lists its corners round the outline.
_Avoid_: solid (alone it is a hatch's `SOLID` fill, or AutoCAD's 3D solid), filled polygon

**Hatch**:
An object that fills an area solid or with a hatch pattern, inside a boundary of closed loops, as in AutoCAD. It draws no outline of its own.
_Avoid_: fill, shading, region (a region is AutoCAD's 2D solid object)

**Hatch pattern**:
A named set of families of parallel lines that a hatch fills its area with, as in an AutoCAD `.pat` file. Every hatch pattern but `SOLID` comes from a definition in the sheet description.
_Avoid_: fill pattern, texture

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

**Attribute definition**:
An attribute as its block defines it, once: its attribute tag, its prompt, its default value and how its text draws. AutoCAD's ATTDEF.
_Avoid_: attribute template

**Constant attribute**:
An attribute whose value its block's definition holds, the same on every insert, as AutoCAD's Constant mode. An insert carries no value for it and cannot set one.
_Avoid_: fixed attribute, static attribute

**Attribute tag**:
The label an attribute is filled in by, such as `TAG` or `TERMINAL`, kept in capitals as AutoCAD keeps it. A block may define one tag on more than one attribute definition; each is an **occurrence** of the tag, counted from 1 in the order the block defines them. A plain tag is what an instrument or device is called on the drawing, which is often an attribute's value.
_Avoid_: attribute name, field name

**Insert**:
One placement of a block on a sheet, at a position, scale and rotation, carrying a value for each of the block's attributes. AutoCAD calls it a block reference.
_Avoid_: instance, block reference

**Nested block**:
A block that another block inserts, such as a terminal in a terminal strip. The insert that places it is one of the outer block's objects, and the outer block's definition fills in its attributes, once for every insert of the outer block.
_Avoid_: sub-block, child block

**Block library**:
The blocks a drawing set shares, each defined once for the whole set. It is a file of its own that sheet descriptions name.
_Avoid_: symbol library, catalog

**Redefinition**:
A block's whole definition replaced, its base point and every object, as AutoCAD's BEDIT replaces it. Every insert of the block draws as the new definition does, and is brought into line with it as it lands: a value an insert gives for a tag the block no longer carries is removed from its record, as AutoCAD's ATTSYNC removes it.
_Avoid_: block edit, block update

**Import**:
A drawing's blocks read into a block library, as AutoCAD's DesignCenter drags them from one drawing into another: each with the blocks it nests, and the layers, linetypes, hatch patterns and text styles they use. The drawing is only read. What the core has no model for is not carried, and the import counts it.
_Avoid_: conversion, WBLOCK

**Template**:
A sheet's border, title block and fixed content, waiting for data.
_Avoid_: prototype, master

**Row**:
One sheet's data in a drawing set: the sheet's name, the template it is drawn from, the values that template takes from it, and any objects of the sheet's own, which are on that sheet alone. A sheet can be a template plus one row.
_Avoid_: record (a record is the file that is the truth for a sheet), entry, line

**Set settings**:
The file at the top of a drawing set's folder, `set.yaml`, that declares the set's unit, where its outputs go and where its rows are.
_Avoid_: project file, config

**ID**:
An object's identifier, fixed for the object's life. It lives in the object's record: the sheet's name joined to the `id` the sheet description or template gives the object, such as `DEMO-001/border`, or to the object's place in the file when it gives none. It is unique in the drawing set, and the DWG carries it with the object.
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
A mark written inside every DWG that AgentDrafter builds, so that a later build knows the file is its own. A build replaces an existing DWG only when it carries the mark; any other file may be an original and is never overwritten. A sheet built from a drawing set records the set's folder in its mark, relative to the DWG and never in full, so that an output drawn from the DWG alone finds its set wherever the DWG is.
_Avoid_: stamp (a stamp records the revision an output came from), signature, watermark

**Declared unit**:
The unit a sheet description states for its numbers, such as `mm`. A number with no unit of its own is in the declared unit; a number that carries its own unit is converted to it. There is no default unit.
_Avoid_: default unit, base unit

**Bare null**:
An unquoted `null`, `Null`, `NULL` or `~`, or a value left empty, as in `name:`, in a sheet description or another file of a drawing set, and JSON's `null` on a key of an object a change set adds. On a field whose value is text or a path it means the field is not written: an optional field takes its default and a required one is an error. Quoted, `'null'` is text.
_Avoid_: null value, empty value

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
A named, typed action on a drawing, such as `insert_block`. Where AutoCAD has the same command, the operation carries the AutoCAD name as an alias. An operation names the object it edits, its target, by ID or name.
_Avoid_: command (the word for what a person types), tool, call

**Change set**:
Operations sent together as one request. Each sheet takes its part of a change set fully or not at all.
_Avoid_: batch, transaction, commit

**Journal**:
The on-disk record of every change set and every failed attempt. A later session reads it to see what was done and to undo it. Undo is a stack: undoing a change set also undoes every later one that is not issued.
_Avoid_: history, log, undo stack

**Failed attempt**:
A change set that changed nothing: one refused whole, one that failed on every sheet it touched, or one whose files could not be written. It stays in the journal and is never undone.
_Avoid_: failed change set, error entry

**Undo state**:
The saved bytes a change set's undo needs: each file it wrote, as it was before. Only the newest change sets keep theirs; an older one's is pruned, and that change set can no longer be undone.
_Avoid_: snapshot, backup, checkpoint

**Pending entry**:
A journal entry of a change set, an undo or an accept, written before any of its files is put in place and not yet marked done. One a killed process left is **rolled back** by the next command that reads the set: every file goes back as it was before, from the undo state, and the journal marks the entry rolled back. A rolled-back entry changed nothing.
_Avoid_: transaction, write-ahead log, dirty entry

**Inconsistent set**:
A drawing set in which a change set landed on some sheets and failed on others. It stays marked, in a file in the set's folder, until the failed sheets are fixed or the change set is undone as one.

**Working revision**:
The state an agent's changes land on at once: the drawing set's own files, edited in place. It is not issued.
_Avoid_: draft, staging

**Issued**:
Accepted by the reviewer. Nothing is issued until the reviewer accepts the working revision. The reviewer cannot accept while the set is inconsistent, and an issued change set cannot be undone.
_Avoid_: released, approved, final

**Accept**:
The command that issues the working revision: every change set still in place, and the set's files as they stand. The reviewer accepts in the viewer, or asks an agent to accept for them through any interface; the journal records each accept with the interface it came through. It is refused while the set is inconsistent, and warns, listing them, when checks fail.
_Avoid_: approve, sign off, release

**Query**:
A question an agent asks a drawing set about what is on its sheets, such as which objects carry a tag, which blocks a sheet inserts, or how far apart two objects are. It reads the records, changes nothing, and selects objects by ID, name, block, attribute or text, never by coordinates. A query that matches nothing says so.
_Avoid_: search, lookup, select

**Extents**:
The box round everything a sheet draws, seen from above, as AutoCAD's extents are: the geometry itself, not the width of its pens.
_Avoid_: bounds, bounding box, limits (AutoCAD's limits are a setting, not what is drawn)

**Check**:
A rule a sheet is tested against, such as nothing off the sheet or no text overflowing its title-block cell.
_Avoid_: lint, validation rule

**Checker**:
The part of AgentDrafter that re-reads saved files and reports failed checks. It is separate from whatever made the change.
_Avoid_: validator, linter

**Finding**:
One failed check: the sheet, the rule, the object by ID and name, and what is wrong in numbers in the declared unit.
_Avoid_: violation, error (an error stops a request; a finding does not), issue

**Paper**:
What a sheet is drawn on, as its template or sheet description declares it: a width and a height, or a standard size such as `ANSI B landscape`, with its lower left corner at the origin. The checker holds the sheet to it, and a plot and a render are drawn on it.
_Avoid_: page, sheet size, limits (AutoCAD's limits are a setting of its own)

**Cell**:
The box in a block that one of its texts or attributes draws within, such as a title-block cell, declared in the block library in the block's own coordinates and placed with each insert. The checker holds the text to it.
_Avoid_: field, box (a cell is not a block)

**Output**:
A PDF, PNG or DWG produced from a revision and stamped with the revision it came from. An output whose revision has moved on is stale.
_Avoid_: export, artifact, plot

**Stamp**:
What an output records about the files it came from: the record and the files it draws on, such as a block library, each by where it was and a fingerprint of its bytes, and the revision once revisions are built. An output any of whose files no longer matches its stamp, or is gone, is stale.
_Avoid_: watermark, signature, build mark (a build mark tells that AgentDrafter built a DWG, not what from)

**Render**:
Drawing a sheet to a PNG, through the display list, so an agent can look at its own work.
_Avoid_: screenshot, export, build (a build produces a DWG)

**Plot**:
Drawing a sheet to a PDF at its sheet size, through a plot style, as AutoCAD plots the same DWG, for issue. The PDF is an output; its text can be searched.
_Avoid_: print, export (and a plot is the act, not the PDF: the PDF is an output)

**Plot style**:
The pens a plot draws with: for each AutoCAD colour index, the colour and the lineweight it plots in, read from a CTB file such as AutoCAD's `monochrome.ctb`. With none named, every pen plots black at its object's own lineweight.
_Avoid_: pen table, pen settings, CTB (the CTB is the file a plot style is read from)

**Plot style folder**:
A folder AgentDrafter reads CTB files from, named by a setting, such as AutoCAD's `Plot Styles` folder on a machine that has AutoCAD. There is no default.
_Avoid_: plot style path, CTB directory

**Display list**:
What the drawing pipeline turns a sheet into: lines drawn with a colour, a lineweight and their dashes laid out, and filled areas, text among them as strokes and outlines, in drawing units, with each line of text also as the characters it reads, so an output can keep it searchable. The PNG, the PDF and the viewer all draw from it, so they agree.
_Avoid_: scene, render tree, drawn object (the judge's count of records)

**Script**:
Code an agent sends to run inside AgentDrafter for an edit no operation covers. It runs sandboxed as one change set and only when the reviewer has switched scripts on.
_Avoid_: macro, plugin

### Reaching AgentDrafter

**Core**:
The Rust library that knows drawings. Every interface is a thin layer over it, and it reads and writes DWG through acadrust behind one seam.
_Avoid_: backend, server

**Interface**:
One of the four ways in: the command line, the MCP server, the Python library and the local HTTP API. All four expose the same operations with the same behaviour. The journal records which one an accept came through, and counts the viewer's page apart from the HTTP API it reaches the core through.
_Avoid_: door, adapter, front end

**Command**:
One thing an interface asks the core to do, such as build, query, change or undo, with one name through every interface and taken from one registry in the core. It takes its input as plain data and gives the same reply through each interface. Applying a change set is the command `change`; the change set's operations are operations, not commands.
_Avoid_: endpoint, route, tool (the MCP server's word for a command), subcommand

**Token**:
The secret every request to the HTTP API carries when it listens beyond the local machine, made new each time it starts and printed when it does. On the local machine none is needed.
_Avoid_: password, API key

**Viewer**:
The browser window where the reviewer watches a drawing set change, steps through its journal, and accepts or undoes. It learns of every change set from the set's journal, whichever interface sent it.
_Avoid_: app, editor, UI

### Measuring

**Speed baseline**:
The fixed set of numbers later work reports its speed against: one command from a cold start, a 100-sheet set built to DWG and PDF, and a change showing in the viewer, measured by one script on the laptop and on forge and recorded with the machines. A pull request that could move them reports them; none is a gate.
_Avoid_: benchmark score, performance budget

### Material

**Client material**:
Any plant's drawing, block, tag or name. It never enters this repository, including in tests, fixtures, screenshots and notes.

**Demo plant**:
The made-up plant whose data, blocks and sheets live in this repository for tests and examples.
_Avoid_: sample project, fixture plant
