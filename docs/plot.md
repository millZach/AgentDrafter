# Plotting a sheet to PDF

`agentdrafter plot` writes a PDF a plant can be issued: the sheet at its exact size, each pen's colour and lineweight from a CTB plot style, the text as AutoCAD's plot of the same DWG draws it, and that text searchable. It draws through the one drawing pipeline the PNG draws through (ADR-0011): the sheet becomes a display list, and the display list becomes a PDF. The PDF is stamped with every file it was drawn from, as the PNG is, and `agentdrafter stale` says whether any of them has moved on.

```
agentdrafter plot <sheet-description | dwg | drawing-set-folder> [--output <pdf>] [--overwrite]
                  [--plot-style <ctb>] [--plot-styles <folder>]... [--fonts <folder>]...
agentdrafter stale <png-or-pdf> [--source <sheet-description-dwg-or-template>]
```

Both print one line of JSON, as `build` does.

## The sheet size

Zach decided on 2026-11-13 that a sheet that declares its [paper](sheet-description.md#paper) plots on exactly that paper: the PDF's page is the paper at 1:1 in the drawing's unit, its lower left corner at the origin, whatever the sheet draws. What lies off the paper is cut at the page's edge, as AutoCAD cuts what lies outside the window it plots, and [the checker](check.md) still reports it as off the sheet. A sheet that declares none is as large as what it draws ([the drawing set](drawing-set.md#templates)): its page is the drawing's extents at 1:1, the box round everything it draws, as the PNG's extents are (AutoCAD's take the same box but for text, whose box they take where AgentDrafter takes what it draws). The reply's `page` says which: `paper` or `extents`. There is no scale to give.

The paper matters when what a sheet draws falls short of it, as a sheet whose trim marks sit on a layer that does not plot does. On the extents its page would come out smaller than its paper and fail the exact paper size check; on its paper it is the paper's size.

A DWG holds no paper (the DWG is unchanged by it). A DWG AgentDrafter built for a drawing set names the set in its build mark ([the build mark](drawing-set.md#the-build-mark-names-the-set)), so a plot of the DWG on its own reads the paper from that sheet's template filled in from its row, and the PDF is stamped with the template and the rows as well as the DWG. A DWG with no set, or whose set or row cannot be read, plots to its extents.

- The demo set's template draws its trim line from (0, 0) to (17, 11) inches, so each of its sheets plots on a page of exactly 17 by 11 inches: 431.8 by 279.4 mm, 1224 by 792 points. Inches are exact in points, so the page box is exact; a sheet drawn in millimetres lands on the same page to within a millionth of a point.
- The page's lower left corner is the paper's, the origin; or, for a sheet with no paper, the drawing's corner at its extents' lower left.
- What lies along the page's edge, such as the template's trim line, draws the half of its pen that falls on the page. Zach decided this on 2026-11-11, knowing AutoCAD leaves it out. AutoCAD 2026, plotting a sheet to extents on paper its own size, leaves out every segment that lies along the paper's edge (the trim line, and on a made-up probe a line on the edge and the parts of a polyline on it) and draws what only touches it. A plot does not leave them out: the page's edge is the box round what the sheet draws, so on a sheet with no trim line it runs along the sheet's own outermost lines, and along the strokes of text where text is outermost, which AutoCAD's extents, taking text's box, never do. Leaving them out would cut those away; drawing them costs a hairline, half the trim line's pen, on the very edge of the paper.
- A DWG with no unit is taken to be in millimetres, as `render` takes it.
- A sheet with no paper that draws nothing, or nothing with both a width and a height, has no size and is refused (`bad_value`); one with paper plots a blank page, and so is one larger than 200 inches (5080 mm) a side, the largest page PDF readers open: a sheet that size has objects drawn far outside its border.

The reply gives the page as `sheet_size` (`width_mm` and `height_mm` to the micrometre, `width_points` and `height_points` as the page box holds them) and `page`, with the `extents` in drawing units, the box round what the sheet draws, and `scale`, always 1.

### On its paper, measured against AutoCAD

AutoCAD 2026 plotted a made-up probe on 2026-11-13 through its headless console: DWG To PDF, model space, a window from (0, 0) to (17, 11) inches, 1:1, on ANSI full bleed B through `monochrome.ctb`. The probe declares `ANSI B landscape` and draws a border half an inch inside the paper, a line and a circle that run off it, and a note. AgentDrafter plotted the same sheet on its paper. Both pages are 1224 by 792 points; at 300 dots per inch every inked pixel of each lies within a pixel of the other's, and both cut the line and the circle at the paper's edge.

## Layers

A plot leaves out every object on a layer that is off, frozen or does not plot, as AutoCAD's plot does ([layers](sheet-description.md#layers)); objects on a layer that is off or does not plot still count towards the extents, so a sheet whose trim marks sit on a layer that does not plot, and that declares no paper, still plots on a page as large as its trim marks. AutoCAD 2026 plotted a made-up probe of such layers, and an insert on each, on 2026-11-13 through `acad.ctb`: its effective plotting area and extents, 160 by 75 mm from (-10, -10), were AgentDrafter's to the hundredth of a millimetre, and both drew the same two lines, every inked pixel of each within a pixel of the other's.

## The plot style

A CTB file, AutoCAD's colour-dependent plot style table, sets each pen: for each AutoCAD colour index, the colour and the lineweight that index plots in. AutoCAD's own CTB files, such as `monochrome.ctb` and `acad.ctb`, are Autodesk's and never enter this repository, not even in a test (ADR-0008). They are read from the plot style folders, as fonts are read from the font folders: the folders `--plot-styles` names, then those the `AGENTDRAFTER_PLOT_STYLES` environment variable lists (separated as the system separates paths), such as AutoCAD's `Plot Styles` folder. There is no default folder. `--plot-style` names the CTB by its file name, found ignoring case; a path is refused, so a plot style is never read from outside its folders.

**With no plot style named**, a plot draws with the built-in monochrome style: every pen black, at its object's own lineweight. That is what AutoCAD's `monochrome.ctb` sets for every one of its pens, and what a plant's issued sheets usually are, so a machine without AutoCAD plots the usual case exactly. The reply names it `monochrome (built in)`.

What a CTB's pen sets, and what the plot follows:

| A pen's setting | What the plot does |
|---|---|
| `color` | Followed. -1 (or -1006632961) plots the object's own colour, in the colour AutoCAD plots that index in ([`render`](render.md#what-it-draws)), index 7 black. Any other value means the pen plots in a colour of its own: red, green and blue in the low three bytes of the pen's `mode_color`, where CTB writers keep a custom colour while `color` may hold another value, or of `color` when the pen has no `mode_color`. A `mode_color` of -1006632961 (or -1) plots the object's own colour. A value whose top byte is 0xC3, the kind ezdxf calls ACI, is red, green and blue like any other, not a colour index: 0xC3000003 plots red 0, green 0, blue 3, not index 3's green. AutoCAD 2026 plotted all of this through a made-up CTB ([below](#how-autocad-plots-a-pens-colour)), except a `mode_color` of -1, which it was not shown and is read like -1006632961. |
| `lineweight` | Followed. 0 plots the object's own lineweight, its layer's or AutoCAD's default of 0.25 mm; any other value `n` plots with the lineweight at place `n - 1` of the file's `custom_lineweight_table`, counted from 0, so 1 is its first, 0 mm. AutoCAD 2026 plotted a made-up CTB's pens of `lineweight` 20 and 25 at the table's 0.9 and 1.58 mm, its places 19 and 24, not at 1 and 2 mm. |
| `screen`, grayscale in `color_policy`, `linetype`, `end_style`, `join_style`, `fill_style`, and the table's `apply_factor` | Not followed yet. The reply's `plot_style.not_followed` lists each one a CTB sets, with what it means, what the plot does instead, and the colour indexes whose pens set it, so nothing is left out in silence. AutoCAD's `monochrome.ctb` and `acad.ctb` set none of them. |

A NUL byte at the end of a CTB's text, which some writers such as ezdxf add, is ignored. A value followed by an appendix after a space, in brackets, as some files' lineweight tables carry one (`0.0076200000000 (+7.Z+"8V?S_LC )`, shown in ezdxf's issue #1069), is read as the value before it, as ezdxf reads it and as AutoCAD 2026 plotted it (below); what the appendix means is not known. Anything else after a value, not in brackets, is an error, as before. A CTB that cannot be found, or read, is an error (`read_failed`, at `plot_style`) and nothing is written. A named plot style table (STB) is refused: a sheet is plotted through a CTB.

## The text

Text draws as the display list draws it, so it looks as AutoCAD plots it: an SHX font's strokes with the layer's pen and a TrueType font's outlines filled, from the fonts in the font folders or their look-alikes ([`render`](render.md#what-it-draws)). Each line of text is also written on top as invisible text, so a PDF reader can search it, select it and copy it, SHX text as much as TrueType text:

- The display list carries each line of text it draws as a run of its characters along its baseline: where the line starts, the frame it is set in (one unit of the font's advances along the baseline, and the capitals' height up, turned, enlarged and mirrored with the text and any insert it is in), and how far the pen moves after each character. The run draws nothing, so the PNG is the same with it or without it.
- The PDF sets each run in that frame as one string of invisible text (text rendering mode 3) in Type 3 fonts whose glyphs draw nothing. Each character the sheet uses, at each advance its fonts give it, is a glyph of its own, exactly as wide as the character moves the pen, so each character lands where its pen starts and its box reaches where the next one starts: a reader joins each word whole, parts words at the spaces the text holds, and highlights the text where it draws. Each font's `ToUnicode` map says which character each glyph is, so any character a font can draw can be searched for and copied.
- Single-line text, each line of multiline text, the text in a block and each visible attribute of an insert are each a line. A character a font cannot draw is drawn as nothing and is not in the text.

Poppler's `pdftotext`, pdfium (the PDF reader in Chrome and Edge), `pypdf` and `pdfminer.six` all read the examples' text back as written, rotated lines among them, except that `pdfminer.six` reads a line turned a quarter one character at a time, and `pdftotext -raw` runs two words of a TrueType line together where the space between them is narrow. The reply's `text_lines` counts the lines the PDF holds.

## Where it goes

The PDF goes beside the source with a `.pdf` extension unless `--output` names another path. An existing file is never replaced without `--overwrite`, and `--overwrite` replaces only a PDF AgentDrafter plotted, as its stamp shows; anything else may be someone's own file (`not_an_output`). The PDF is written whole or not at all.

Given a drawing set's folder, `plot` plots every sheet of the set, each its template filled in from its row as `build` reads it, into the set's output folder as `<output>/<sheet>.pdf`, beside the DWG `build` writes there. It needs no build first: the PDF is drawn from the record, the template and the row, as the DWG is. As with `build`, a sheet that fails writes nothing and the others are still plotted; the reply gives each sheet's outcome under `sheets`, with its `notices`, and is not `ok`, with the error `sheets_failed`, when any sheet failed. Settings, rows or a plot style that cannot be read stop the set before anything is written. `--output` is refused for a set.

The same sheet plots to the same bytes: the PDF holds no date and no random identifier.

## The stamp

The PDF carries the same stamp as the PNG ([the stamp](render.md#the-stamp)), as one line of JSON in plain ASCII (any other character written as a `\u` escape) under the key `AgentDrafter` in the PDF's document information. Its `produced_by` is also the PDF's `Producer`, and its `Creator` names the plot style. The stamp is read only from there, the dictionary the PDF's last trailer names as `/Info`: the same text anywhere else, such as in a page's text, is not a stamp, so `stale` says such a PDF is not stamped and `--overwrite` never replaces it. That dictionary is found through the PDF's cross-reference tables or streams, so a plot saved again by a program that keeps them in streams, such as `qpdf --object-streams=generate`, which also moves the document information into an object stream, still reads as stamped: `stale` checks it and `--overwrite` replaces it. A stream packed any way but deflate, with or without a PNG predictor, is not read, and such a PDF reads as not stamped, the safe way round. So does a PDF whose streams do not hold the rows, fields or objects their dictionaries say they do, or that unpack to more than 64 MiB in all: a PDF made to break the reader is not ours, and `--overwrite` refuses it.

- A sheet description's PDF lists the sheet description, and the block library it names.
- A DWG's PDF lists the DWG. Its revision is read from the set the DWG's build mark names, wherever the DWG is ([the build mark](drawing-set.md#the-build-mark-names-the-set)).
- A drawing set's sheet lists its template (`kind` `template`), the rows file (`rows`) and the template's block library. A change to any row makes every sheet of the set stale, since a stamp fingerprints whole files.

Fonts and the plot style are not in it: they belong to the machine that plots, as fonts do for a render. Its `revision` says whether the files were read from an issued revision, and which accept issued it, as a PNG's does ([the stamp](render.md#the-stamp)): a set's sheet plotted after an accept, from the files it issued, says so; the page is drawn the same either way. `agentdrafter stale <pdf>` checks a PDF as it checks a PNG, and reports the revision too; `--source` says where the first file, the record, is now.

## The pipeline

- `crates/agentdrafter-core/src/draw/` makes the display list, with its runs of text (`TextRun`), as for the PNG.
- `src/pdf.rs` turns the display list into the PDF with [pdf-writer](https://crates.io/crates/pdf-writer) (MIT or Apache-2.0) and packs its streams with [miniz_oxide](https://crates.io/crates/miniz_oxide) (MIT, Zlib or Apache-2.0). Strokes have round ends and corners, as AutoCAD plots them; positions are written to a thousandth of a point.
- `src/plot_style.rs` reads a CTB and holds the built-in monochrome style; `src/plot.rs` is the entry point, which finds the CTB, works out the page and writes the PDF; `src/output.rs` holds what every output shares: reading its record and writing it safely.

The display list, the PDF writer and the CTB reader are plain computation: no window, GPU, file system, thread or clock, and they build for `wasm32-unknown-unknown`.

## Measured against AutoCAD

AutoCAD 2026 on the machine that has it plotted the DWGs `agentdrafter build` wrote to PDF through its headless console: DWG To PDF, model space, the window its own extents, 1:1, centred, through `monochrome.ctb` with lineweights. The demo set's three sheets, and a made-up probe of lines on and touching the paper's edge, were plotted on the sheet's paper, ANSI full bleed B, 17 by 11 inches. The examples are as large as their extents, which no stock paper is, so AutoCAD plotted them on A3 with its window 5 mm round their extents; a made-up probe of nine pens was plotted the same way through a made-up CTB. AgentDrafter plotted the same DWGs, with AutoCAD's own fonts in its font folders.

Both PDFs were turned into pixels at 300 dots per inch (one pixel is 0.085 mm). AgentDrafter's page was laid on AutoCAD's where the plot puts it, corner on corner for the demo sheets, and the best-overlapping shift within a few pixels taken; then for every inked pixel of each, the distance to the nearest inked pixel of the other was measured, over AgentDrafter's page.

The tolerance a plot is held to: every inked pixel within 2 pixels (0.17 mm) at 300 dots per inch of the other's ink, both ways, except that a line lying on the paper's edge is drawn by AgentDrafter and left out by AutoCAD (above). Every sheet below meets it. At 300 dots per inch a few pixels of `examples/text.yaml`, 0.006% of AgentDrafter's ink and 0.002% of AutoCAD's, lie 2.24 pixels (one pixel past 2 on the diagonal, 0.19 mm) from the other's ink. That is the overlay's rounding, not the text: the rasters fall on a grid of 0.085 mm and are laid on each other at a whole pixel's shift, and the same two PDFs measured at 600 dots per inch put every inked pixel of both within 0.095 mm of the other's, and at 1200 within 0.085 mm.

| Sheet | Page, AutoCAD's and AgentDrafter's | AutoCAD's ink within 1 px of AgentDrafter's | within 2 px | AgentDrafter's ink within 1 px of AutoCAD's | within 2 px |
|---|---|---|---|---|---|
| `DEMO-001`, `DEMO-002`, `DEMO-003` | 1224 by 792 points, both | 99.96% | 100% | 99.96% away from the page's edge; 93% in all | 100% away from the page's edge; 93% in all |
| `examples/one-line.yaml` | A3; 100 by 50 mm | 100% | 100% | 100% | 100% |
| `examples/geometry.yaml` | A3; 245 by 154 mm | 99.99% | 100% | 99.99% | 100% |
| `examples/hatch.yaml` | A3; 220 by 200 mm | 100% | 100% | 100% | 100% |
| `examples/text.yaml` | A3; 237.643 by 123.623 mm | 99.53% | 100% | 99.60% | 99.99% |
| `examples/blocks/sheet-1.yaml`, `sheet-2.yaml` | A3; 102.294 by 62.293 and 72.999 by 35.999 mm | 100% | 100% | 100% | 100% |
| made-up CTB probe | A3; 200 by 110 mm | 99.96% | 100% | 99.99% | 100% |

On the demo sheets, the 7% of AgentDrafter's ink that is not near AutoCAD's is the trim line's half pen on the page's edge, which AutoCAD leaves out; away from a band 5 pixels wide along the edge, both ways match. Through the made-up CTB, every pen plotted in the same colour in both, and every lineweight the CTB set came out the same to the pixel.

### How AutoCAD plots a pen's colour

A second made-up probe settled how AutoCAD reads a pen's `color` and `mode_color` when they differ, and a value of the 0xC3 kind ([#36](https://github.com/millZach/AgentDrafter/issues/36)). AutoCAD 2026 plotted a made-up sheet, a line and a solid swatch on each of ten layers of their own colour index, through a made-up CTB of ten such pens, as above, on 2026-11-12. Read at 600 dots per inch, each pen came out in the colour below. AgentDrafter's plot of the same DWG through the same CTB matches it on every pen.

| Pen | `color` | `mode_color` | AutoCAD plotted |
|---|---|---|---|
| 1 | red | blue | blue: `mode_color` |
| 30 | 0xC3000003 | blue | blue: `mode_color` |
| 2 | red | 0xC3000003 | red 0, green 0, blue 3: the low three bytes, not index 3's green |
| 3 | 0xC3000005 | none | red 0, green 0, blue 5 |
| 50 | 0xC3000001 | 0xC3000001 | red 0, green 0, blue 1 |
| 60 | 0xC3000002 | 0xC3000004 | red 0, green 0, blue 4 |
| 4 | red | -1006632961 | the object's own colour, index 4's cyan |
| 6 | -1 | green | the object's own colour, index 6's magenta |
| 40 | green | green | green |
| 250 | red | none | red |

Pen 4 was the one AgentDrafter plotted otherwise before: it read `mode_color`'s low three bytes as white. It now plots the object's own colour, as AutoCAD does. A `mode_color` of -1 is read the same way, though AutoCAD was shown only -1006632961. Every line plotted 0.51 mm wide in both, its layer's 0.5 mm, to within a pixel at 600 dots per inch (0.042 mm).

The same run plotted the sheet through a second made-up CTB, whose lineweight table carries the appendices of ezdxf's issue #1069 at its places 19 and 24. AutoCAD plotted the pens that use them at 0.889 and 1.566 mm as measured, the table's 0.9 and 1.58 mm, the same as AgentDrafter.

What still differs: the trim line on the page's edge (above); AutoCAD writes its PDF on a grid of 1/600 inch, one step down and to the left of where things are, and rounds each lineweight to it, so its sheets sit 0.04 mm off AgentDrafter's and its pens are up to 0.01 mm wider or narrower; TrueType text sits up to 2 pixels from AutoCAD's, as it does in the PNG; and AutoCAD writes its SHX text as lines, so none of it can be searched, where AgentDrafter's can.

## What is not offered yet

- Paper-space layouts and viewports, and a scale other than 1:1: a sheet is model space plotted at 1:1 onto its declared paper, or to its extents when it declares none.
- A plot style's screening, grayscale, linetypes, line ends and joins and fill styles (listed under `not_followed` when a CTB sets them), and named plot styles (STB).
- PDF layers from the drawing's layers, and PDF/A.
- More than one sheet in one PDF.
