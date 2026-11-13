# AutoCAD's capabilities against AgentDrafter's

This page maps what AutoCAD lets a drafter do against what AgentDrafter does, and ranks the gap by what an agent drafting controls drawings needs most: loop sheets, PLC interconnects, panel layouts and network one-lines. Each capability has one issue. Zach decided on 2026-11-13 that the work follows this ranking ([`docs/plan.md`](plan.md)). It was written on 2026-11-13, at `main` 53ff845, and folds in the notes of two real controls jobs drawn with AgentDrafter that day, told here in made-up terms only (ADR-0008).

Every capability is done the same way, and each issue says so: the schema and the reader agree on it, an operation covers it with its AutoCAD command as an alias, all four interfaces reach it from the core's registry (ADR-0007), the PNG, the PDF and the viewer draw it through the one drawing pipeline (ADR-0011) as AutoCAD plots it, every DWG it writes passes the judge, and its page and glossary words land with it. An entry's AutoCAD words, such as layout or viewport, join [`CONTEXT.md`](../CONTEXT.md) when the entry is built, not before.

## What AgentDrafter does today

- **Objects:** lines, polylines of straight segments, arcs, circles, ellipses, splines, hatches, text and mtext in SHX and TrueType styles, and inserts of blocks with attributes from a block library, on layers with linetypes. An object draws in its layer's colour, linetype and lineweight; it has none of its own.
- **New sheets:** built from sheet descriptions, or from a drawing set's template and rows, into DWG 2018; rendered to PNG and plotted to PDF through a CTB at 1:1 to extents.
- **Operations:** add each kind of object, `change`, `move`, `delete` and `set_attribute`, in change sets that land on a set's own files, with a journal, undo and accept.
- **Queries:** find by ID, name, block, attribute or text; a sheet's blocks; an insert's attributes; the distance between two objects' own points; a sheet's extents.
- **Checks:** the exact paper size, nothing off the sheet, no text overflowing its cell, no text on other text.
- **Existing DWGs:** `render` and `plot` read a DWG's model space, drawing what the core models and counting the rest. Nothing revises one yet.

acadrust, behind the seam in `src/dwg.rs`, already models most of what the gap needs: dimensions, leaders and multileaders, tables, layouts and viewports, wipeouts, images and underlays, fields, groups, dynamic blocks, multilines, points, construction lines and 3D solids, as well as transforms and explode. Most of the gap is AgentDrafter's own model, operations and drawing pipeline, not the DWG library.

## What the real jobs showed

A real controls job drew about twenty sheets with AgentDrafter: interconnects, network one-lines and panel layouts, generated from data, with no change sets used. Every sheet passed the checker and AutoCAD 2026's audit with no errors, AutoCAD's plot looked the same as AgentDrafter's PDF, a rebuild was byte for byte the same, and the set built, plotted and checked in about a second. The checker caught three real faults. What it cost, most costly first, is in the ranking below as issues [#100](https://github.com/millZach/AgentDrafter/issues/100), [#97](https://github.com/millZach/AgentDrafter/issues/97), [#64](https://github.com/millZach/AgentDrafter/issues/64), [#99](https://github.com/millZach/AgentDrafter/issues/99), [#98](https://github.com/millZach/AgentDrafter/issues/98), [#77](https://github.com/millZach/AgentDrafter/issues/77), [#65](https://github.com/millZach/AgentDrafter/issues/65), [#101](https://github.com/millZach/AgentDrafter/issues/101), [#103](https://github.com/millZach/AgentDrafter/issues/103), [#102](https://github.com/millZach/AgentDrafter/issues/102), [#104](https://github.com/millZach/AgentDrafter/issues/104), [#73](https://github.com/millZach/AgentDrafter/issues/73) and [#76](https://github.com/millZach/AgentDrafter/issues/76); its one known gap was no dimensions on panel sheets, [#75](https://github.com/millZach/AgentDrafter/issues/75).

A second real controls job, 25 sheets on 2026-11-13, met the same gaps in nearly the same order, which is recorded on each issue. It asked for one more: a build that reports, or cleans up, outputs no row writes any more, [#109](https://github.com/millZach/AgentDrafter/issues/109), ranked 18th below. Its note on attribute tags was checked against the reader: `#` and `/` already pass, and a repeated tag and a lowercase one do not ([#99](https://github.com/millZach/AgentDrafter/issues/99)).

## How it is ranked

1. **What the real job paid for.** A gap a job worked round, with a script, a derived layer or a hand-drawn table, ranks above one it did not meet.
2. **How often controls drawings use it.** Loop sheets and interconnects are lines, text, blocks with attributes and tables, 1:1 in model space; panel layouts add dimensions, leaders and scale; one-lines add tables and callouts.
3. **What it unblocks.** A capability others stand on ranks before them: an object's own colour and arc polylines before importing a plant's blocks, object snaps before dimensions, trim and fillet.
4. **Size.** Between two of equal need, the smaller lands first.

## The ranking

| Rank | Capability | AutoCAD | Issue |
|---|---|---|---|
| 1 | An object's own colour, linetype, lineweight and linetype scale (built, #108) | PROPERTIES, CHPROP, LTSCALE | [#64](https://github.com/millZach/AgentDrafter/issues/64) |
| 2 | Polylines with arc segments and widths, donuts and 2D solids | PLINE, DONUT, SOLID | [#65](https://github.com/millZach/AgentDrafter/issues/65) |
| 3 | Plot a sheet onto its declared paper, not only its extents | PLOT window or limits | [#97](https://github.com/millZach/AgentDrafter/issues/97) |
| 4 | Layer control: layers, linetypes and text styles as operations; on, frozen, locked and plot states | LAYER, LINETYPE, STYLE | [#68](https://github.com/millZach/AgentDrafter/issues/68) |
| 5 | Check: text on a line | none: AgentDrafter's checker | [#98](https://github.com/millZach/AgentDrafter/issues/98) |
| 6 | Blocks: nested blocks, attribute options, defining and redefining a block | BLOCK, ATTDEF, ATTSYNC | [#69](https://github.com/millZach/AgentDrafter/issues/69) |
| 7 | A block may define the same attribute tag more than once | ATTDEF | [#99](https://github.com/millZach/AgentDrafter/issues/99) |
| 8 | Import blocks from a DWG or DXF into the block library | WBLOCK, INSERT of a drawing | [#100](https://github.com/millZach/AgentDrafter/issues/100) |
| 9 | Tables | TABLE, TABLESTYLE | [#73](https://github.com/millZach/AgentDrafter/issues/73) |
| 10 | Leaders and multileaders | MLEADER, LEADER, QLEADER | [#76](https://github.com/millZach/AgentDrafter/issues/76) |
| 11 | Object snaps and geometric queries | OSNAP, LIST, AREA, MEASUREGEOM, DIST | [#74](https://github.com/millZach/AgentDrafter/issues/74) |
| 12 | Dimensions and dimension styles | DIMLINEAR and the rest, DIMSTYLE | [#75](https://github.com/millZach/AgentDrafter/issues/75) |
| 13 | Paper-space layouts and viewports | LAYOUT, MVIEW, VPLAYER | [#77](https://github.com/millZach/AgentDrafter/issues/77) |
| 14 | Font folders: the system's fonts, and a warning when a look-alike stands in | FONTALT | [#101](https://github.com/millZach/AgentDrafter/issues/101) |
| 15 | A row's value inside other text | none: drawing sets | [#102](https://github.com/millZach/AgentDrafter/issues/102) |
| 16 | More than one rows file in a drawing set | none: drawing sets | [#103](https://github.com/millZach/AgentDrafter/issues/103) |
| 17 | Render through a plot style, to show what prints | PLOT preview | [#104](https://github.com/millZach/AgentDrafter/issues/104) |
| 18 | A build reports, or cleans up, outputs that no row writes any more | none: drawing sets | [#109](https://github.com/millZach/AgentDrafter/issues/109) |
| 19 | Copy and array | COPY, ARRAY | [#66](https://github.com/millZach/AgentDrafter/issues/66) |
| 20 | Rotate, scale and mirror | ROTATE, SCALE, MIRROR | [#67](https://github.com/millZach/AgentDrafter/issues/67) |
| 21 | Read a plant's DWG whole: every object, a decode report and a text mirror | OPEN, AUDIT | [#70](https://github.com/millZach/AgentDrafter/issues/70) |
| 22 | Revise a plant's DWG: a change set on an original, saved as a revision in its version | SAVEAS | [#71](https://github.com/millZach/AgentDrafter/issues/71) |
| 23 | Revision clouds | REVCLOUD | [#72](https://github.com/millZach/AgentDrafter/issues/72) |
| 24 | Wipeouts and draw order | WIPEOUT, DRAWORDER | [#78](https://github.com/millZach/AgentDrafter/issues/78) |
| 25 | Dynamic blocks placed in a chosen state | BEDIT states, Properties | [#79](https://github.com/millZach/AgentDrafter/issues/79) |
| 26 | Find and replace across a drawing set | FIND | [#86](https://github.com/millZach/AgentDrafter/issues/86) |
| 27 | Offset | OFFSET | [#80](https://github.com/millZach/AgentDrafter/issues/80) |
| 28 | Trim and extend | TRIM, EXTEND | [#81](https://github.com/millZach/AgentDrafter/issues/81) |
| 29 | Fillet and chamfer | FILLET, CHAMFER | [#82](https://github.com/millZach/AgentDrafter/issues/82) |
| 30 | Break and join | BREAK, JOIN | [#83](https://github.com/millZach/AgentDrafter/issues/83) |
| 31 | Stretch | STRETCH | [#84](https://github.com/millZach/AgentDrafter/issues/84) |
| 32 | Explode | EXPLODE | [#85](https://github.com/millZach/AgentDrafter/issues/85) |
| 33 | Fields | FIELD | [#87](https://github.com/millZach/AgentDrafter/issues/87) |
| 34 | Plot-style control: named plot styles, page setups and plot scale | PAGESETUP, STB | [#88](https://github.com/millZach/AgentDrafter/issues/88) |
| 35 | DXF read and write | DXFIN, DXFOUT | [#89](https://github.com/millZach/AgentDrafter/issues/89) |
| 36 | External references | XREF, XATTACH, XBIND | [#90](https://github.com/millZach/AgentDrafter/issues/90) |
| 37 | Raster images and PDF underlays | IMAGEATTACH, PDFATTACH | [#91](https://github.com/millZach/AgentDrafter/issues/91) |
| 38 | Groups | GROUP | [#92](https://github.com/millZach/AgentDrafter/issues/92) |
| 39 | Annotation scale | ANNOTATIVE, CANNOSCALE | [#93](https://github.com/millZach/AgentDrafter/issues/93) |
| 40 | Points, construction lines, rays, multilines and tolerances | POINT, XLINE, RAY, MLINE, TOLERANCE | [#94](https://github.com/millZach/AgentDrafter/issues/94) |
| 41 | Convert a drawing to another DWG version | SAVEAS to a version | [#95](https://github.com/millZach/AgentDrafter/issues/95) |
| 42 | 3D solids in the AutoCAD style | BOX, EXTRUDE, UNION | [#96](https://github.com/millZach/AgentDrafter/issues/96) |

Already filed before this map, and not ranked again here: text underline and overline, `%%u` and `%%o` ([#60](https://github.com/millZach/AgentDrafter/issues/60)); MTEXT's inline formatting codes ([#61](https://github.com/millZach/AgentDrafter/issues/61)); a middle-justified TrueType text's place ([#59](https://github.com/millZach/AgentDrafter/issues/59)). Filed while building, as follow-ups: an object's true colour ([#106](https://github.com/millZach/AgentDrafter/issues/106)) and a quoted insert scale the schema lets through ([#107](https://github.com/millZach/AgentDrafter/issues/107)).

## The plan's agreed order, against this ranking

`docs/plan.md` agreed four steps after the first usable version. Each is here:

| The plan's step | Issues |
|---|---|
| 1. Revise a plant's DWG from markups | [#70](https://github.com/millZach/AgentDrafter/issues/70), [#71](https://github.com/millZach/AgentDrafter/issues/71), [#72](https://github.com/millZach/AgentDrafter/issues/72); a plant's drawing needs [#64](https://github.com/millZach/AgentDrafter/issues/64) and [#65](https://github.com/millZach/AgentDrafter/issues/65) to draw as AutoCAD plots it |
| 2. Paper-space layouts and viewports | [#77](https://github.com/millZach/AgentDrafter/issues/77) |
| 3. Dimensions and leaders | [#75](https://github.com/millZach/AgentDrafter/issues/75), [#76](https://github.com/millZach/AgentDrafter/issues/76), with [#74](https://github.com/millZach/AgentDrafter/issues/74) under them |
| 4. Dynamic blocks placed in a chosen state | [#79](https://github.com/millZach/AgentDrafter/issues/79) |

The ranking differs from the plan's order in one way: the real job generated every sheet and revised none, and what it paid for was on new sheets. So this ranking puts first what generating a plant's sheets cost that day, most of it small, and the foundations a plant's drawing needs either way, an object's own colour and arc polylines, then the objects controls sheets are full of, tables, leaders and dimensions, then layouts, and revising a plant's DWG after them. Zach chose this ranking on 2026-11-13; either order builds the same issues.

## What the map leaves out

- **Trade knowledge.** AutoCAD Electrical's wire numbering, terminal and PLC I/O reports and its symbol libraries belong in a generator, not the engine (ADR-0002).
- **AutoCAD's programming interfaces.** AutoLISP, .NET and ObjectARX are replaced by operations and, later, sandboxed scripts (`docs/plan.md`).
- **The drafting app for people**, its commands typed at a prompt, grips and tool palettes: a later phase on the same core.
- **Rendering and visualisation**: materials, lights, cameras, walks and animation.
- **Sheet Set Manager and cloud collaboration**: the drawing set and the journal stand in for them.
