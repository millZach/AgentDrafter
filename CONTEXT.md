# AgentDrafter

AgentDrafter is an open-source CAD engine built for agents first: an agent creates, revises and checks technical drawings with no window open, and a person reviews the result. This file is the glossary. Write code, documents and messages in these terms.

## Language

### Who does what

**Agent**:
An AI coding agent, such as Claude Code or Codex, that calls AgentDrafter to make or change drawings.
_Avoid_: bot, AI

**Reviewer**:
The person who looks at what an agent did and accepts or undoes it.
_Avoid_: user, operator

**Drafter**:
A person who draws and edits in the drafting app. The same person is the reviewer when they accept or undo.
_Avoid_: user, operator, designer

**Assistant**:
The agent that drafts with a drafter in the drafting app: it answers in the app's panel, proposes, guides and checks. It is an agent like any other and reaches the core through an interface. It runs on the drafter's own agent, Claude Code or Codex, which `agentdrafter serve` starts pointed at `agentdrafter mcp --assistant`, the MCP server that serves only the commands that read and `propose`: what it would change is always a proposal.
_Avoid_: copilot, bot, AI

**Agent adapter**:
The small part of `agentdrafter serve` that runs one agent's command line for the assistant: how it is started for a message, given the assistant's MCP server and kept from writing, and how what it prints is read back as its words and the commands it calls. Claude Code and Codex each have one.
_Avoid_: driver, plugin, connector, integration

**What the drafter is looking at**:
The sheet open in the drafting app, the layout shown and the objects selected on it, by their IDs. The page records it with `look-at` as the drafter moves about, beside the set's journal and outside its records, and an agent on any interface reads it with `looking-at`. Each message to the assistant carries it, shown as a chip above the input, unless the drafter leaves it off that message.
_Avoid_: context, focus, the selection (it is more than the selection)

**Generator**:
A program outside AgentDrafter that knows a trade, such as controls, and turns project data into sheet descriptions. AgentDrafter knows drawings; a generator knows the trade.
_Avoid_: plugin, toolkit

**Judge**:
AutoCAD run with no window to audit a DWG that AgentDrafter wrote. It is used in tests only and is never part of a job.
_Avoid_: oracle, reference CAD

### Drawings

**Drawing**:
One CAD file, DWG or DXF (ASCII or binary).
_Avoid_: document, model

**DXF**:
AutoCAD's Drawing Exchange Format, ASCII (the default when building DXF) or binary (`dxf_binary`), read everywhere AgentDrafter reads a DWG and written by `build` (via `format` or a `.dxf` output path) and drawing sets (`format: dxf` or `dxf_binary` in `set.yaml`) with the same build mark (`$CUSTOMPROPERTYTAG` / `$CUSTOMPROPERTY`) and object IDs as a DWG.
_Avoid_: text DWG, export file

**External reference (xref)**:
Another DWG or DXF file linked into a host drawing (`attach`, carried along when the host is itself referenced, or `overlay`, drawn in the host only) as a block record holding a file path rather than embedded geometry, placed by regular `insert` objects (`xrefs:` in sheet descriptions and templates, `attach_xref` / `XATTACH` / `XREF`, `detach_xref` / `XDETACH`, and `bind_xref` / `XBIND`). Resolved from disk for drawing, querying and checking without embedding geometry into built DWGs, with missing or cyclic references reported under `unresolved_xrefs` and top-level `warnings`.
_Avoid_: linked file, external block, underlay (an underlay is a PDF or raster image)

**Raster image**:
An external bitmap image (PNG, JPEG, BMP, GIF or TIFF) linked into a drawing (`image` object, AutoCAD's `IMAGE` / `IMAGEDEF` / `IMAGEDEF_REACTOR`, attached with `add_image` / `IMAGEATTACH` / `IMAGE`), placed on a sheet, template, row or inside a block at a position, width, optional height and rotation, with optional polygonal clip boundary (and inverted clip), brightness, contrast, fade and alpha transparency. Resolved from disk into the display list without embedding pixels in built DWGs, with missing or unreadable files reported under `unresolved_media` and top-level `warnings`.
_Avoid_: bitmap, picture, texture

**PDF underlay**:
One page of an external PDF file linked into a drawing (`pdf_underlay` object, AutoCAD's `PDFUNDERLAY` / `PDFDEFINITION`, attached with `add_pdf_underlay` / `PDFATTACH` / `PDFUNDERLAY`), placed at a position, scale and rotation, with optional polygonal clip boundary (and inverted clip), contrast, fade, monochrome and unloaded state. Rasterised from disk into the display list for PNG, PDF and WebGL2 drawing, with missing files or out-of-range pages reported under `unresolved_media` and top-level `warnings`.
_Avoid_: PDF import (importing converts PDF vectors into geometry; an underlay links the PDF page)

**Image frame**:
The drawing-wide setting (`image_frame`: `off`, `on`, or `display`; AutoCAD's `IMAGEFRAME` / `RASTERVARIABLES` in `ACAD_IMAGE_VARS`) that controls whether the boundary frames of raster images and PDF underlays draw and plot.
_Avoid_: image border, crop box

**Sheet**:
One titled page that prints: a border, a title block and the content inside them. It is drawn in model space alone, or in a layout: its border and title block in paper space, and its content in model space, shown through viewports. A sheet drawn in several layouts is one DWG that prints once for each layout, each on its own paper, all showing its one model space; it keeps one name, and its objects one set of IDs.
_Avoid_: page, layout (a layout is AutoCAD's paper-space tab, which may hold a sheet)

**Model space**:
Where a drawing is drawn at full size, in the declared unit, as in AutoCAD: every object of a sheet with no layout, and the objects of a sheet drawn in layouts that are in no layout's paper space. A sheet's layouts share its model space.
_Avoid_: model (alone it is AutoCAD's tab), drawing area

**Paper space**:
Where a layout draws at its paper's size, as in AutoCAD: its border, its title block, its notes and its viewports, its lower left corner at the paper's. Each layout of a sheet has its own, which an object names by the layout's name under `space`.
_Avoid_: sheet space, layout space

**Layout**:
AutoCAD's paper-space tab that holds a sheet: its name, its paper, its paper space, and its page setup, which plots its whole paper at full size, as AutoCAD's LAYOUT and PAGESETUP make it. A sheet drawn in one prints its layout: paper space as it is drawn, and model space through its viewports. A sheet may be drawn in several, in the order of their tabs, each printing on its own. A DWG is read with every layout it holds, whatever tab it was saved on; the tab says only which it opens on.
_Avoid_: sheet (a sheet is the page; a layout holds one), tab, paper space (the space a layout draws in)

**Viewport**:
A window on a layout's paper that shows model space at a scale, centred on a model point, its view centre, as AutoCAD's MVIEW makes one: seen from above, rectangular or clipped to a clipping boundary, cut at its edge, and locked or not against its view being changed, with layers frozen in it alone or none. It is an object of paper space, and its frame plots as AutoCAD's does: a rectangle as its thinnest line, a clipping boundary with the viewport's pen.
_Avoid_: view, window (a window is a selection box), viewport alone for AutoCAD's model-space tiles (VPORTS)

**Clipping boundary**:
The closed polyline, its segments straight or arcs, or the circle on a layout's paper that a viewport is clipped to instead of its rectangle, as AutoCAD's VPCLIP and MVIEW's Object and Polygonal options make it: model space shows inside it alone, its ink cut there, and it is the viewport's frame. The viewport's window is the box round it.
_Avoid_: clip path (the PDF's way of cutting ink), mask (the PNG's), polygonal viewport

**Frozen in a viewport**:
A layer frozen in one viewport alone, as AutoCAD's VPLAYER freezes it: its objects neither draw nor plot through that viewport, and are not checked through it, but draw through the sheet's other viewports. A layer frozen everywhere is frozen on the layer itself.
_Avoid_: VP freeze, hidden, off (a layer that is off draws nowhere and still counts)

**Annotation scale**:
A named paper-to-model scale ratio (such as `1:50` or `1/4" = 1'-0"`, AutoCAD's `ACAD_SCALELIST` and active `CANNOSCALE`) that sizes annotative objects on paper regardless of the viewport scale they are viewed through. Managed in sheet descriptions with `annotation_scales`, `annotation_scale`, `anno_all_visible` (`ANNOALLVISIBLE`) and per-viewport `annotation_scale`, queried with `query annotation-scales`, and edited with `set_annotation_scale` (`CANNOSCALE` / `SCALELISTEDIT`) and `set_object_scales` (`OBJECTSCALE` / `ANNOUPDATE`).
_Avoid_: text scale, viewport scale (a viewport's scale projects model geometry; its annotation scale sizes and filters annotative objects)

**Annotative**:
An object (`text`, `mtext`, `dimension`, `mleader`, `hatch`, or `insert`), style (`text_styles`, `dimension_styles`, `mleader_styles`), or block definition marked `annotative: true` (AutoCAD's `ANNOTATIVE` / `AcadAnnotative` XDATA and `ACDB_ANNOTATIONSCALES` context data) whose declared size is its paper size: it draws at that exact size on paper through every viewport whose scale is in its `annotation_scales`, and hides in viewports whose scale it does not support unless `anno_all_visible` is true.
_Avoid_: auto-scaling, scale-independent

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

**Group**:
A named set of objects on a sheet (`groups:` in sheet descriptions, templates and rows; AutoCAD's `GROUP` / `ACAD_GROUP`, including unnamed `*A...` groups read from DWGs) that operations can target as one by name (`{"group": "..."}`) and that AutoCAD selects as one when `selectable` is true, managed with `add_group` (`GROUP` / `G`), `ungroup` (`UNGROUP`) and `edit_group` (`GROUPEDIT`) and listed by `query groups`. Unlike a block, its objects remain ordinary top-level objects on the sheet, and an object may belong to more than one group.
_Avoid_: selection set (a selection set is transient; a group is saved in the drawing), collection

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

**Revision cloud**:
A closed lightweight polyline of outward-bowing (or reversed, inward-bowing) arcs drawn round a rectangle, a polygon or an existing object's extents to call out a revised region, as AutoCAD's REVCLOUD draws it, in `normal` or tapered `calligraphy` style and carrying AutoCAD's `RevcloudProps` extended data so AutoCAD's REVCLOUD can modify it.
_Avoid_: revcloud (alone it is AutoCAD's command name), markup bubble

**Wipeout**:
A closed polygonal mask, AutoCAD's WIPEOUT object, that hides whatever is drawn beneath it in the background colour (white on paper and in plotted PDFs and PNGs, the model-space background on screen) without trimming or deleting the geometry underneath; its boundary outline is controlled drawing-wide by `wipeout_frame` (`off`, `on`, or `display`, AutoCAD's `WIPEOUTFRAME` / `ACAD_WIPEOUT_VARS`). May appear on a sheet, in a template, in a row, or inside a block.
_Avoid_: mask (a mask is the display-list shape a wipeout draws), cover, blanking box

**Point style**:
How `point` (`POINT`) objects draw across a sheet, controlled by AutoCAD's `pdmode` (`PDMODE`: dot, blank, `+`, `×`, or `|`, optionally inside a circle and/or square) and `pdsize` (`PDSIZE`: absolute drawing units when positive, 5% of view height when 0, or percentage of view height when negative). Changed with `set_point_style` (`DDPTYPE`).
_Avoid_: marker style, node style

**Construction line**:
An infinite line (`xline`, AutoCAD's XLINE) defined by a point it passes through and a direction vector, clipped to the sheet or viewport view bounds when drawn so it never blows out extents.
_Avoid_: infinite line, guideline

**Ray**:
A semi-infinite line (`ray`, AutoCAD's RAY) starting at a base point and extending indefinitely along a direction vector, clipped to the sheet or viewport view bounds when drawn.
_Avoid_: half-line

**Multiline**:
A set of parallel lines (`mline`, AutoCAD's MLINE) drawn along a path of vertices with mitered corners, optional background fill, joints and end caps, governed by its multiline style, scale and justification (`top`, `zero` or `bottom`).
_Avoid_: double line, parallel polyline

**Multiline style**:
A named definition (`mline_styles`, AutoCAD's MLSTYLE / `ACAD_MLINESTYLE`) giving a multiline's parallel elements (offset, colour and linetype), optional fill colour, joint lines, start/end angles and end-cap flags. Every drawing has AutoCAD's `Standard` (`+0.5` / `-0.5`) without listing it.
_Avoid_: mline format

**Tolerance**:
A geometric dimensioning and tolerancing (GD&T) feature control frame (`tolerance`, AutoCAD's TOLERANCE) drawn as boxed compartments holding GD&T symbols (`{\Fgdt;...}`) and datum/tolerance text at a position and direction, sized by its dimension style or `text_height`.
_Avoid_: GD&T box, callout frame

**Centre mark**:
A crosshair with optional radial extension lines (`center_mark`, AutoCAD's CENTERMARK) marking the centre of a circle or arc, written into DWG/DXF as an anonymous block insert carrying AutoCAD's `OCS_CENTERMARK` extended data.
_Avoid_: crosshair, center tick

**Centre line**:
A centre line segment with independent start and end extensions (`center_line`, AutoCAD's CENTERLINE), written into DWG/DXF as a line carrying AutoCAD's `OCS_CENTERLINE` extended data.
_Avoid_: axis line

**Shape**:
A compiled SHX vector symbol placed on a sheet (`shape`, AutoCAD's SHAPE), naming its `.shx` shape file (looked up in font folders, never bundled from Autodesk), shape name or number, position, size, rotation, width factor and oblique angle.
_Avoid_: glyph (a glyph is a character in a font), symbol (a symbol is usually a block)

**Draw order**:
The back-to-front sequence in which objects draw: earlier objects draw behind later ones, so a wipeout hides objects drawn before it while objects drawn after it appear on top. Controlled in sheet descriptions by list order, in change sets by `draw_order` (`DRAWORDER` / `DR`), `text_to_front` (`TEXTTOFRONT`), and the optional `draw_order` key on every `add_*` operation, and in DWGs by each space's `ACAD_SORTENTS` (`SORTENTSTABLE`) extension dictionary.
_Avoid_: z-order (Z is the vertical coordinate axis), layer order (layers do not set draw order)

**Table**:
A grid of cells with text, AutoCAD's TABLE object, hung from its top left corner: rows of given heights running down, columns of given widths across, each cell's text laid out in its margins, and cells merged across rows and columns. Its first row may be a title row and the next a header row; the rest are data rows.
_Avoid_: grid, schedule (a schedule is one use of a table), spreadsheet

**Table style**:
What a table's cells draw with unless they give their own, as AutoCAD's TABLESTYLE: the text style, text height and justification of title, header and data rows, the cells' margins, and the borders' pen. Every drawing has the table style `Standard`.
_Avoid_: table format, cell style (AutoCAD's name for one kind of row's part of a table style)

**Leader**:
An arrow from a point on the sheet, a line through any bends, and the text it points from: what calls out a device or a part, or points a note at a wire. A sheet holds two kinds, as AutoCAD does: the multileader, and the older leader, AutoCAD's LEADER object, one line with an optional multiline text annotation beyond its end, hooked when its last segment is steep. Its arrow is placed at a point, which an operation may give as a snapped point on the object it points at; it does not follow the object afterwards.
_Avoid_: callout (a callout is one use of a leader), arrow (the arrow is a leader's arrowhead), pointer, QLEADER (AutoCAD's command that draws one)

**Multileader**:
A leader as AutoCAD's MULTILEADER object draws it: one or more leader lines, each from its arrowhead to where they all meet a landing, the short horizontal line beyond which its content, multiline text or block content, sits, laid out where its multileader style says. AutoCAD's command for one is MLEADER, and MLEADEREDIT adds and removes its leader lines.
_Avoid_: mleader alone in prose (it is the sheet description's key), dogleg (AutoCAD's other name for the landing)

**Block content**:
A block of the block library as a multileader's content, in place of its text, as AutoCAD's MULTILEADER holds one: placed at the landing's end where AutoCAD places it, by the centre of the block's extents or by its base point, and drawn as an insert of the block draws, with its attributes' values. A tag balloon or a tag hexagon at a leader's end is block content.
_Avoid_: block leader, balloon (a balloon is one use of block content), insert (block content is not an insert of its own; it draws as one)

**Multileader style**:
What a multileader draws with unless it gives its own, as AutoCAD's MLEADERSTYLE: the text style and height, the arrowhead and its size, the landing's length and its gap to the text, where the landing meets the text, and the leader lines' pen. Every drawing has the multileader style `Standard`.
_Avoid_: leader style (an older leader takes its arrowhead and sizes from a dimension style), callout style

**Dimension**:
A measurement drawn on the sheet, as AutoCAD's DIMENSION draws it: extension lines from the points it measures, a dimension line with an arrowhead at each end, and text that shows the measured value as its dimension style formats it, or its own text with `<>` standing for the value. A linear dimension measures along an angle, an aligned one between its points, a radius or diameter dimension a circle or an arc by its centre and a point on it, an angular dimension the angle between two lines or at a vertex between two points, with an arc for its dimension line, and an ordinate dimension a feature's X or Y from the origin, with a leader from the feature to its text. AutoCAD's commands for them are DIMLINEAR, DIMALIGNED, DIMRADIUS, DIMDIAMETER, DIMANGULAR and DIMORDINATE; it keeps what each draws in a block of its own, `*D1` and on, which it lays out again from the dimension.
_Avoid_: dim alone in prose, measurement (the measurement is what a dimension shows), callout

**Baseline dimension**:
A dimension added from another's first extension line, as AutoCAD's DIMBASELINE adds it: it measures from the same point, its dimension line further out by its style's baseline spacing, so that a row of them stacks. Once added it is an ordinary dimension of its kind.
_Avoid_: datum dimension, stacked dimension

**Continued dimension**:
A dimension added from another's second extension line, on the same dimension line, as AutoCAD's DIMCONTINUE adds it, so that a chain of them runs end to end. AutoCAD marks it by overriding its style's DIMTMOVE, which draws nothing different, and AgentDrafter keeps the mark as `continued`.
_Avoid_: chain dimension, string dimension

**Unit format**:
How a dimension style writes a length, as AutoCAD's DIMLUNIT: scientific, decimal, engineering (feet and decimal inches), architectural (feet and fractional inches), fractional or Windows desktop; and an angle, as its DIMAUNIT: decimal degrees, degrees, minutes and seconds, gradians or radians.
_Avoid_: units (the sheet's unit is what its lengths are in), number format

**Stacked fraction**:
A fraction in a dimension's text drawn as AutoCAD stacks it, its numerator over its denominator across a bar, or across a slash, both as high as the rest of the text, as an architectural or fractional length shows its part of an inch.
_Avoid_: stacked text, built-up fraction

**Dimension style**:
What a dimension draws with, as AutoCAD's DIMSTYLE: its arrowhead and their size, its text style and height, the gap round its text, how far its extension lines start from their points and reach past the dimension line, its centre mark, where its text sits, its baseline spacing, and how a value is written, its unit format, precision, fraction, decimal separator, zeros, prefix and suffix, all multiplied by its scale. A metric sheet has AutoCAD's `ISO-25` and an imperial one AutoCAD's `Standard` without listing it.
_Avoid_: dimstyle alone in prose, dimension format

**Hatch**:
An object that fills an area solid or with a hatch pattern, inside a boundary of closed loops, as in AutoCAD. It draws no outline of its own.
_Avoid_: fill, shading, region (a region is AutoCAD's 2D solid object)

**Hatch origin**:
The point a patterned hatch's pattern is laid from, AutoCAD's HPORIGIN: each line of the pattern runs through it, moved by the line's own origin. It moves and turns with the hatch's boundary, so a moved or copied hatch keeps its pattern where its boundary is.
_Avoid_: pattern base point (each line of a pattern has its own), snap base

**Hatch pattern**:
A named set of families of parallel lines that a hatch fills its area with, as in an AutoCAD `.pat` file. Every hatch pattern but `SOLID` comes from a definition in the sheet description.
_Avoid_: fill pattern, texture

**Text style**:
A named font that text draws with, and how wide its characters are, as in AutoCAD. The font is an AutoCAD SHX font such as `romans.shx` or a TrueType font such as `arial.ttf`. Every drawing has the style `Standard`.
_Avoid_: font style, text format

**Font folder**:
A folder AgentDrafter reads fonts from when it measures text, named by a setting, such as AutoCAD's `Fonts` folder on a machine that has AutoCAD. A TrueType font is then looked for in the system's own font folders, such as `C:\Windows\Fonts`, as AutoCAD finds it, unless they are left out; an SHX font is read only from the folders named.
_Avoid_: font path, font directory

**Look-alike**:
An open font AgentDrafter measures text with when a text style's font is not in any font folder, such as Hershey's Roman Simplex in place of `romans.shx`. The drawing still names the style's own font, and the build says which look-alike it used, in each sheet's result and in a warning at the top of the reply.
_Avoid_: fallback font, substitute font

**Warning**:
What a reply says at its top, beside `ok`, that an agent must not miss: so far, that a look-alike stood in for a font, naming the font, the look-alike and every sheet it was used on. Unlike a notice, which a sheet's own result carries about the files it was read from, a warning is about the whole reply.
_Avoid_: alert

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
One placement of a block on a sheet, at a position, scale and rotation, carrying a value for each of the block's attributes and, for a dynamic block, its chosen parameter values under `state`. AutoCAD calls it a block reference.
_Avoid_: instance, block reference

**Dynamic block**:
A block that declares named parameters — visibility states, lookup tables, flips, linear stretch distances, or rotations — so each insert can be placed in a chosen state without defining a separate block for every variant. Its chosen state draws through the one drawing pipeline, writes into a built DWG via an evaluated anonymous block (`*U<n>`), and is edited with `set_block_state` (`PROPERTIES`). Existing plant dynamic blocks (`ACAD_ENHANCEDBLOCK`) are read whole and preserved unchanged through revisions (ADR-0004).
_Avoid_: smart block, parametric symbol

**Visibility state**:
One named state of a dynamic block's visibility parameter, listing which of the block's objects draw when that state is active.
_Avoid_: layer state (layers are sheet-wide; visibility states belong to one dynamic block)

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

**Rows file**:
A file of a drawing set's rows, one row per sheet, under the key `rows`. A set has one or more, which its set settings name one by one or as a folder of them, and builds their rows in the order given; a sheet's name is given once across them all. A row's errors, its own objects' places and every edit to it are in the rows file it is written in.
_Avoid_: data file, CSV, sheet list

**Row value**:
One of a row's values: the text its template's `{row: NAME}` stands for, on that row's sheet. It may fill several places on the sheet, such as a wire number written four times; an edit to any of them sets the row value, on that sheet alone, and every place it fills changes.
_Avoid_: field, variable, placeholder (the template's `{row: NAME}` is where a row value goes, not the value)

**List of parts**:
A template's text or mtext contents, or an insert's attribute value, written as a list of plain text and `{row: NAME}`, joined in order, so that row values stand inside other text: `['PANEL ', {row: PANEL}, ' SLOT ', {row: SLOT}]`. Its plain text is its fixed parts; an edit of the joined text is matched to them to set each row value it holds, on that sheet alone.
_Avoid_: format string, interpolation, splice (nothing inside a string is read)

**Set settings**:
The file at the top of a drawing set's folder, `set.yaml`, that declares the set's unit, where its outputs go and which rows files its rows are in.
_Avoid_: project file, config

**ID**:
An object's identifier, fixed for the object's life. It lives in the object's record: the sheet's name joined to the `id` the sheet description or template gives the object, such as `DEMO-001/border`, or to the object's place in the file when it gives none. It is unique in the drawing set, and the DWG carries it with the object. An object of an original that carries no ID of AgentDrafter's takes the original's name joined to its handle, such as `P-101/handle:2F`, which stays while the object does.
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

**Decode report**:
What a read of a DWG says of the DWG library's reading of it: whether it decoded every record of the file whole, and each record it did not, by kind, count and handle. An undecoded custom, proxy or opaque record is kept as the bytes read, so a revision in the original's version carries it through, though nothing draws, queries or changes it; a damaged record skipped while reading past damage could not be read at all and is left out, and a record or section cut short is reported alongside any reader warnings and whether the library hit its diagnostic limit.
_Avoid_: audit (AutoCAD's AUDIT checks a drawing's integrity), parse log

**Untouched means unchanged**:
The fidelity rule for a revision: everything AgentDrafter did not edit comes back exactly as it was in the original, including content AgentDrafter does not understand. An original that cannot be fully read is not revised unless the reviewer allows it for that drawing.
_Avoid_: round-trip safe, lossless

### Changing and checking

**Operation**:
A named, typed action on a drawing, such as `insert_block`. Where AutoCAD has the same command, the operation carries the AutoCAD name as an alias. An operation names the object it edits, its target, by ID or name.
_Avoid_: command (the word for what a person types), tool, call

**Copy**:
An ordinary object an operation made from another, its source, as AutoCAD's COPY makes one: every key of its source, an insert's attribute values among them, but its name, moved to its place and, in an array, turned. It has an ID of its own, its source's `id` and the first number free on the sheets it reaches, such as `term-3`, and is written where its source is: a copy of a template's object in the template, so on every sheet built from it, and of a row's own object in the row. It names the object it was copied from under `copy_of`, which makes it one of a **copy family** with its source and the source's other copies. A value its source takes from the row it shares until the copy's is set: then that copy takes its own row value, and the others keep theirs. The first object of the copies sets the shared value where it is, so the copies not set since follow it. Nothing else ties it to its source.
_Avoid_: duplicate, clone, instance

**Array**:
Copies of objects placed in a pattern by one operation: in rows and columns, round a centre, or along a path, the objects themselves the first item. AgentDrafter's arrays land as plain copies, each an ordinary object; AutoCAD's associative array, which stays one object to edit later, is not written (Zach, 2026-11-21).
_Avoid_: associative array (AutoCAD's, which is not written), pattern, repeat

**Path** (of a path array):
The line, polyline, arc, circle, ellipse or spline a path array runs along, from its start as it is drawn; the copies ride along it as the objects sit at its start, turned with its direction unless they are not aligned.
_Avoid_: curve, guide

**Transform**:
A rotate, a scale or a mirror of objects, as AutoCAD's ROTATE, SCALE and MIRROR do them: every point an object is drawn through lands where it puts it, every direction it is drawn in turns with it, and, scaled, every length it is drawn with is scaled. A rotate and a scale work in place, and so does a mirror told to erase its source; a mirror otherwise keeps its source and makes mirrored copies. Text a mirror moves reads forwards, as AutoCAD's MIRRTEXT 0 keeps it. What the sheet description cannot hold, such as a table turned, is refused, never approximated.
_Avoid_: transformation, xform, flip

**Mirror line**:
The line a mirror reflects objects across, given by two points. The mirror is upright through it, so every point keeps its Z.
_Avoid_: axis, text mirror (a readable rendering of a DWG)

**Mirrored insert**:
An insert whose block's X axis is reversed before it is scaled and turned, as AutoCAD's MIRROR leaves one: `mirrored: true`, which a DWG keeps as a negative X scale. What its block draws is mirrored; its attributes read forwards. Mirrored back, it is no longer mirrored, and `readable_attributes: true` keeps its attributes reading forwards.
_Avoid_: negative scale, flipped block

**Fillet**:
A tangent arc of a declared radius rounding the corner between two objects (lines, arcs or circles) or every straight corner of a flat polyline, as AutoCAD's FILLET (`F`) draws it; a radius of `0` trims or extends two objects to a sharp corner or squares up existing arc corners on a polyline, and two parallel lines are joined with a semicircular cap.
_Avoid_: round (alone), corner arc

**Chamfer**:
A straight bevel cutting back the corner between two lines or every straight corner of a flat polyline by equal or asymmetric distances or by a length and an angle, as AutoCAD's CHAMFER (`CHA`) draws it; zero distances trim or extend two lines to a sharp corner.
_Avoid_: bevel (alone), miter (a mitre joins wide polyline edges)

**Break**:
Splitting a curve at a point (`BREAKATPOINT`), cutting a centered gap around a point, or removing the span between two points (`BREAK` / `BR`): `target`'s ID carries on, and when an open curve splits in two the second piece receives a new ID reported under `split_id`.
_Avoid_: split (alone), cut (a cut may refer to trimming)

**Join**:
Merging two or more lines, arcs, or open polylines into a single object (`JOIN` / `J`): collinear lines into one line (including across a gap), co-circular arcs into an arc or circle, and end-to-end curves into a polyline; `targets[0]`'s ID carries on and the absorbed objects' IDs are reported under `deleted`.
_Avoid_: merge (alone), weld, group (a group is AutoCAD's GROUP object)

**Stretch**:
Moving the endpoints, vertices and rigid objects lying inside a selection box by a displacement while leaving vertices outside the box fixed, as AutoCAD's STRETCH (`S`) does, so connected wires and enclosures follow; the one operation that selects by area, listing every object it touched under `transformed`.
_Avoid_: resize (alone), pull, drag

**Explode**:
Turning a compound object — an insert (into its evaluated block's objects and its visible attributes as single-line text), a polyline, donut or revision cloud (into lines and arcs), or a hatch, dimension, multileader, leader or table — into the simpler objects that draw it, as AutoCAD's EXPLODE (`X`) does; the source object is deleted (`deleted`) and each new object gets its own ID, listed under `created`.
_Avoid_: ungroup, unpack, flatten (alone)

**Field**:
A live text value attached to a single-line text, multiline text or block insert attribute (`FIELD`) — such as the drawing's file name, save/plot/create/current date, layout tab name, a custom drawing property (`DWGPROPS`), a property of another object on the sheet, or a raw AutoCAD field code — written as AutoCAD's `ACAD_FIELD` / `FIELD` / `FIELDLIST` records so AutoCAD updates it live, drawn with its current evaluated value through the one drawing pipeline, and kept unchanged when an existing DWG is revised.
_Avoid_: formula (for non-table fields), placeholder, macro

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
A question an agent asks a drawing set about what is on its sheets, such as which objects carry a tag, which blocks a sheet inserts, or how far apart two objects are. It reads the records, changes nothing, and selects objects by ID, name, block, attribute or text, or by where they are: within or crossing a box, as AutoCAD's selection windows take them, or touching another object. A box is the only coordinates a query takes. A query that matches nothing says so.
_Avoid_: search, lookup, select

**Object snap**:
A point AutoCAD's object snaps find on an object, an endpoint, a midpoint, a centre, a quadrant, an insertion point or a node, or between two objects, an intersection, an apparent intersection, or a perpendicular, a tangent or a nearest point from a snap of another, named by the object's ID or name, the snap and which of its points (`which`), so that it can be asked for again. An insert's snaps are those of its block's own geometry, where the insert places it, each named by its place in the block, such as `objects[3].to`.
_Avoid_: snap point by coordinates, pick point, osnap (AutoCAD's setting of which snaps are on)

**Running snaps**:
The object snaps the drafting app's cursor catches, which the drafter ticks in the Object snap menu and turns on and off together with Object snap (F3), as AutoCAD's OSMODE holds them: by default Endpoint, Midpoint, Centre and Intersection. The page remembers them in the browser.
_Avoid_: osnap modes, snap settings

**Snapped point**:
An object snap given where an operation takes a point, `{"snap": {...}}`, written as `query snap` takes it and naming one point. It is worked out when the operation applies, on the sheet as it stands then, and the record is written with the coordinates it names: a sheet description holds points, never snaps. A move takes one as its base point or its second point, as AutoCAD's MOVE does.
_Avoid_: reference point, linked point (it does not follow the object afterwards)

**Window**, **crossing** (selection):
A box on the sheet, by two corners, that a find selects by, as AutoCAD's selection windows do: a window takes the objects wholly within it, a crossing those within it or crossing its edge, each by what it draws, its text and its block's objects among it.
_Avoid_: region, area, fence

**Touching**:
Two objects whose geometry meets: one crosses the other, or an end of one lies on the other. How an agent checks that a wire reaches its terminal.
_Avoid_: connected (a trade's word for what the drawing shows), adjacent

**Geometry** (of an object):
An object's length, area, extents and angles, as AutoCAD's LIST and AREA give them, from the object's own curves: not text's, whose extents are its font's.
_Avoid_: properties, bounding box

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
What a sheet is drawn on, as its template or sheet description declares it: a width and a height, or a standard size such as `ANSI B landscape`, with its lower left corner at the origin. The checker holds the sheet to it, and a plot and a render are drawn on it. A sheet drawn in a layout declares a standard size, which its layout's page setup plots, and draws paper space on it.
_Avoid_: page, sheet size, limits (AutoCAD's limits are a setting of its own)

**Cell**:
The box in a block that one of its texts or attributes draws within, such as a title-block cell, declared in the block library in the block's own coordinates and placed with each insert. The checker holds the text to it.
_Avoid_: field, box (a cell is not a block)

**Output**:
A PDF, PNG, DWG or DXF produced from a revision and stamped with the revision it came from. An output whose revision has moved on is stale.
_Avoid_: export, artifact, plot

**Stamp**:
What an output records about the files it came from: the record and the files it draws on, such as a block library, each by where it was and a fingerprint of its bytes, and the revision once revisions are built. An output any of whose files no longer matches its stamp, or is gone, is stale.
_Avoid_: watermark, signature, build mark (a build mark tells that AgentDrafter built a DWG or DXF, not what from)

**Leftover output**:
A DWG, DXF or PDF in a drawing set's output folder that AgentDrafter made for that set, as its build mark or its stamp shows, and that no row writes any more, since its sheet was renamed, merged or taken out of the rows. Every build and plot of the set lists them; cleaning moves them into the superseded folder.
_Avoid_: orphan, stale output (a stale output's files have moved on; a leftover's row is gone), dead file

**Superseded folder**:
The folder `superseded` in a drawing set's output folder that cleaning moves leftover outputs into, keeping their names. Nothing is ever deleted from it or replaced in it.
_Avoid_: trash, archive, recycle bin

**Render**:
Drawing a sheet to a PNG, through the display list, so an agent can look at its own work: in each object's own colour, or through a plot style, or monochrome, to show what a plot will print.
_Avoid_: screenshot, export, build (a build produces a DWG)

**Plot**:
Drawing a sheet to a PDF at its sheet size, through a plot style, as AutoCAD plots the same DWG, for issue. The PDF is an output; its text can be searched.
_Avoid_: print, export (and a plot is the act, not the PDF: the PDF is an output)

**Plot style**:
The pens a plot draws with: for each AutoCAD colour index (in a CTB file such as `monochrome.ctb`) or each named plot style (in an STB file such as `plant.stb`, assigned on layers and objects when `PSTYLEMODE = 0`), the colour, screening percentage and lineweight it plots in. With none named, every pen plots black at its object's own lineweight.
_Avoid_: pen table, pen settings, CTB (CTB and STB are the files a plot style is read from)

**Page setup**:
How a sheet or paper-space layout plots, as AutoCAD's PAGESETUP sets it: its paper, plot area (`extents`, `window` or `layout`), plot scale (`fit` or a ratio such as `1:2`), centring or offset, plot style table (CTB or STB) and whether layer/object transparency plots. Read from an existing DWG's `Layout` `PlotSettings`, declared under `page_setup` in YAML, and edited with `set_page_setup` (`PAGESETUP`).
_Avoid_: print settings, plot config

**Plot style folder**:
A folder AgentDrafter reads CTB and STB files from, named by a setting, such as AutoCAD's `Plot Styles` folder on a machine that has AutoCAD. There is no default.
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

**Drafting app**:
The viewer with drawing tools and an assistant, in a browser or in its desktop shell, where a drafter draws and edits the way they use AutoCAD. Every edit made in it is a change set. Its frame, its first drawing tools and its assistant's panel are built round the viewer ([`docs/drafting-app.md`](docs/drafting-app.md)).
_Avoid_: editor, CAD program, GUI

**Drawing tool**:
What a drafter picks in the drafting app to make one operation by clicking and typing, such as Line or Move. It gathers the operation's input on the drawing and sends it as a change set. It runs in the viewer's renderer, and its AutoCAD aliases, such as `L` or `REC`, are its own, not an operation's.
_Avoid_: command (a command is the core's), tool on its own (the MCP server's word for a command), button

**Current layer**:
The layer a drawing tool puts each new object on, as AutoCAD's current layer is. The drafter sets it in the ribbon's Layer box; the page keeps it for each drawing set.
_Avoid_: active layer, default layer

**Rubber band**:
What a drawing tool draws from its last point to the cursor before the next click, such as the line it would draw or the circle's size, as AutoCAD's rubber band shows it. It is the page's alone: nothing is sent until the tool has what it needs.
_Avoid_: preview (a change set drawn before it lands), ghost

**Preview**:
A change set drawn before it lands: each sheet it reaches as it would leave it, applied in memory to the set as it is and written nowhere, the rest of the sheet faded, what it would add, change and delete marked, whether the sheet would take its part, and the checks it would then fail. `display-list` draws one when given a change set. The drafting app shows an edit to a template's object as one before it lands; a proposal is drawn as one.
_Avoid_: dry run (`change`'s, which draws nothing), ghost, rubber band, proposal (a proposal waits; a preview is how a change set is drawn)

**Template warning**:
What the drafting app says before an edit to an object that comes from a template lands: that every sheet built from the template changes, drawn as a preview on each of them, with what would become of each, and "Change all N sheets" or Cancel.
_Avoid_: template prompt, shared-edit warning

**Command bar**:
The line at the bottom of the drafting app's drawing where a drafter types a drawing tool's alias, a coordinate or words for the assistant, and reads the tool's prompt.
_Avoid_: command line (that is an interface), prompt, console

**Ribbon**:
The strip of the drafting app above the drawing that holds its drawing tools in panels, Draw, Modify and Layer, as AutoCAD's ribbon does, cut down to a few labelled tools, with the Layer box that sets the current layer.
_Avoid_: toolbar, menu

**Quick properties**:
What the selected object is, shown beside it in the drafting app: its type, layer, ID, name, block and attributes, the checks it fails and the change sets that changed it, as AutoCAD's Quick Properties palette shows an object's. Its layer, a text's contents, an insert's attribute values and a circle's radius or diameter, an arc's radius, are edited in them, each edit one change set, and each says before it is edited where its value is written: the template, a row value, or unset.
_Avoid_: inspector, properties panel, tooltip

**Screen colours**:
The colours a sheet shows in on the drafting app's dark model space, where colour index 7 is white, as on AutoCAD's dark screen; the PNG and the PDF plot it black on white paper.
_Avoid_: theme, dark mode

**Proposal**:
A change set that waits to be kept or discarded. It is shown on the sheet before it lands, changes nothing until it is kept, and lands as a change set when it is. `propose` records one, beside the journal, numbered on their own, with who proposed it; `keep` lands it on the set as it stands, as one journal entry that says who proposed it and who kept it; `discard` ends it, with a line in the journal that nothing undoes.
_Avoid_: suggestion, draft, preview (a preview is the drawing of it), pending change

**Guide**:
A list of steps an assistant gives the drafting app to walk a drafter through a task on their own drawing: what to say, which drawing tool, where to click, and the operation that completes each step. Each step the drafter does lands as their own change set, marked with the step in the journal.
_Avoid_: tutorial, wizard, walkthrough

**Setting step**:
A step of a guide that sets something on the page, such as the current layer, in place of an operation: no change set completes it.
_Avoid_: setup step

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
