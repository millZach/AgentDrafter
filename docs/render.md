# Rendering a sheet to PNG

`agentdrafter render` draws a sheet to a PNG so that an agent can look at its own work. It draws through the one drawing pipeline that the PDF ([plotting](plot.md)) and the viewer also draw through (ADR-0011): the sheet becomes a display list, and the display list becomes pixels. The PNG is stamped with every file it was drawn from, and `agentdrafter stale` says whether any of them has moved on since.

```
agentdrafter render <sheet-description-or-dwg> [--output <png>] [--overwrite] [--dpi <dpi>]
                    [--scale <scale> | --width <pixels> --height <pixels>] [--margin <mm>] [--fonts <folder>]...
agentdrafter stale <png-or-pdf> [--source <sheet-description-dwg-or-template>]
```

Both print one line of JSON, as `build` does. Rendering needs no window and no GPU.

## What it draws

What is drawn follows AutoCAD's plot of the same DWG, through a plot style that keeps each object's own colour. The rules below were read from AutoCAD 2026's plots of made-up sheets; the comparison is under [Measured against AutoCAD](#measured-against-autocad).

- **Paper and colour.** The PNG is white paper. Each object draws in its layer's colour, in the colour AutoCAD plots that colour index in; index 7, white on a dark screen, plots black.
- **Lineweights** are widths on paper: a 0.5 mm pen is 0.5 mm wide at the render's dots per inch, whatever the scale. A layer with no lineweight of its own draws with AutoCAD's default, 0.25 mm. Lineweight 0, and any pen thinner than a pixel, draws one pixel wide. Ends and corners are round.
- **Linetypes** repeat along each line, arc, circle, ellipse and spline, and along each segment of a flat polyline. Along an open path, both ends take the pattern's first element, a dash or a dot, and share what is left over after whole repeats, so an end dash may be shorter or longer than the pattern's; a path shorter than one repeat draws continuous. Round a circle or a whole ellipse, the pattern stretches or shrinks to the nearest whole number of repeats, from the curve's start. A 3D polyline draws continuous, as AutoCAD draws it.
- **Text** draws in its font: an SHX font's strokes with the layer's pen, and a TrueType font's outlines filled. A font that is in no font folder is drawn with its look-alike, and the reply lists it under `font_substitutions` as `build` does. Single-line text starts where the build records it starts. Multiline text breaks between words where a line would grow wider than its box, its lines sit 5/3 of the text's height apart (more for a line that draws higher than its capitals), and each line is aligned by what it draws.
- **Hatches** fill solid, or draw their pattern's lines inside their boundary, a point inside an odd number of loops being inside. Pattern lines draw with the layer's pen and are anchored at the drawing's origin; none is drawn along the boundary itself. One hatch draws at most 1,000,000 pattern lines and 2,000,000 dashes: a pattern that needs more stops there, partway along a line if need be, and the reply lists the hatch under `hatches_cut_short`, with its place among the sheet's objects (an insert's, for a hatch in a block, with the block's name), its pattern, its layer and why, as it lists `font_substitutions`. A single line whose own dashes would be more than 2,000,000 draws continuous, as a linetype too fine to see does.
- **Inserts** draw their block's objects moved so that the block's base point sits at the insert's position, scaled and turned with it. Hatch patterns and text are enlarged and turned with the block; a linetype keeps its own lengths and is laid out along each object where the insert puts it, as AutoCAD 2026 plotted a made-up probe of inserts doubled and turned. An object on layer `0` draws with the insert's layer, and so in its colour, linetype and lineweight; an object on any other layer draws with that layer. Each visible attribute draws as single-line text saying the insert's value, placed as the build writes it ([the block library](block-library.md#how-it-is-written)); an invisible attribute draws nothing, and nor do the block's attribute definitions. The extents take in each insert's block box, the box round the block's objects placed whole, as AutoCAD's do, so a turned insert's box can reach past what it draws.
- **Seen from above.** `z` is dropped: a line straight up draws as a dot.

A sheet description that names a block library is drawn with it, found from the sheet description's folder. A DWG is drawn from what the core reads of it (`read_dwg`), its block definitions and inserts among it. Objects the core has no model for yet are counted under `objects_not_drawn`.

## Size and scale

Until drawing sets and their paper sizes arrive (ticket #7), a render covers the drawing's extents, with a margin round them.

| Option | Default | What it is |
|---|---|---|
| `--dpi` | 150 | How many pixels an inch of paper spans. Lineweights draw at this. |
| `--scale` | | The plot scale: how long a length on the drawing is on paper, as a share of itself, such as `1` for 1:1 or `0.02` for 1:50. The drawing's unit counts: 1 m drawn at `0.02` is 20 mm on paper. A DWG with no unit is taken to be in millimetres. |
| `--width`, `--height` | | The image's size in pixels, the drawing fitted inside it and centred. With one, the other follows the drawing. Not given with `--scale`. |
| `--margin` | 5 | The margin round the extents, in millimetres of paper. |

Without a scale or a size, the image's longer side is 2000 pixels. An image is at most 32767 pixels on a side and 100 million in all.

The reply says where the drawing sits: the drawing's point `(x, y)` is at pixel `((x - left) * pixels_per_unit, (top - y) * pixels_per_unit)`, counted from the image's top left, where `[left, top]` is `top_left`. It also gives the extents, the scale, the dots per inch and the image's size.

The PNG goes beside the source with a `.png` extension unless `--output` names another path. An existing file is never replaced without `--overwrite`, and `--overwrite` replaces only a PNG AgentDrafter rendered, as its stamp shows; anything else may be someone's own file (`not_an_output`).

## The stamp

Every PNG carries a stamp, in a text chunk keyed `AgentDrafter`, as one line of JSON; a PDF carries the same stamp in its document information ([plotting](plot.md#the-stamp)). It lists the drawing set's files the sheet is drawn from: the record first, then the files it draws on. Fonts are not in it: they belong to the machine that renders (AutoCAD's `Fonts` folder on one, a look-alike on another), not to the drawing set, so the same PNG checked on another machine is not stale for that.

```json
{"stamp":1,"produced_by":"agentdrafter 0.0.0","sources":[{"kind":"sheet_description","path":"sheet-1.yaml","sha256":"…"},{"kind":"block_library","path":"blocks.yaml","sha256":"…"}],"revision":{"issued":false}}
```

- `sources` holds a sheet description and the block library it names, when it names one, or a DWG alone; a PDF of a drawing set's sheet holds its template, the rows file and the block library.
- Each entry's `kind` is `sheet_description`, `dwg`, `block_library`, `template` or `rows`.
- `path` is where the file was, from the PNG's folder when the two share a folder above them, with `/` between folders, so a PNG and its files moved together still find each other and the stamp names no folder above them. Otherwise it is the path in full.
- `sha256` is the SHA-256 of the file's bytes, as the render read them.
- `revision` says whether the files were read from an issued revision, and which accept issued it. Zach decided on 2026-11-11 that **an output says whether it came from an issued revision in its stamp only**: the drawing looks the same either way, with no watermark. The output is from an issued revision when it is drawn from a drawing set's files (the set is the nearest folder above the record that holds a set's settings, as a DWG in the set's output folder has) and every file the stamp lists is, byte for byte, a file an [accept](journal.md#accept) of the set issued. Then `revision` is `{"issued": true, "accept": 5, "accepted_at": "2026-11-11T21:40:02Z", "via": "viewer"}`: the accept's entry in the set's journal, when it was made and the interface it came through. Otherwise it is `{"issued": false}`: the working revision, or a sheet description of no set. A stamp made before revisions were issued has no `revision`. A DWG written to an output folder outside its set's folder, such as `output: ../issued`, has no set above it, so a PNG or a PDF made from it alone says it was not issued even when an accept issued it; the set's own plot, whose stamp starts from its template, says so ([#50](https://github.com/millZach/AgentDrafter/issues/50), filed for later).

`agentdrafter stale <png>` (or `<pdf>`) reads the stamp, finds each file it lists from the output's folder, and reports `stale: true` when any file's bytes no longer have their fingerprint or it cannot be read. `--source` says where the record, the first file, is now; the others are found as the stamp names them. The reply's `why` names each file that moved on, and `sources` gives each file as it was checked: its kind, where it was looked for, whether it is stale and why. It reports what the stamp says of the revision too: `issued`, true when the output was made from an issued revision, and `revision`, in words, which accept issued it, through which interface and when, or that it was the working revision. An output made from an issued revision that has since moved on is both `issued` and `stale`. A file with no stamp is `not_stamped`.

`stamp` is the stamp's format. A reader that meets a format it does not know says so rather than guessing; one that meets a key it does not know, as an older AgentDrafter meets `revision`, passes over it.

## The pipeline

- `crates/agentdrafter-core/src/draw/` turns a sheet into a display list: strokes with their colour index, lineweight and dashes already laid out, and fills, in drawing units, with each line of text also as a run of its characters along its baseline, which draws nothing and lets the PDF keep the text searchable. `curves.rs` turns arcs, circles, ellipses and splines into chords (a spline through fit points is fitted as AutoCAD fits it), `linetype.rs` lays out linetypes, `hatch.rs` hatches, `text.rs` text, and `color.rs` holds the colour each index plots in. `src/text/outline.rs` gives what each character of a font draws.
- `src/raster.rs` draws the display list on the CPU with [tiny-skia](https://crates.io/crates/tiny-skia) (BSD-3-Clause) and writes the PNG with [png](https://crates.io/crates/png) (MIT or Apache-2.0). `src/stamp.rs` makes and checks stamps, with [sha2](https://crates.io/crates/sha2) (MIT or Apache-2.0). `src/render.rs` is the entry point, and `src/output.rs` holds what it shares with the PDF: reading the record and writing the output safely.

- The viewer draws the same display list in the page ([viewer.md](viewer.md)): the `display-list` command gives it as JSON, and `crates/agentdrafter-viewer`, compiled to WebAssembly, draws it with WebGL2; held to the PNG pixel for pixel, its ink lies within one pixel of the PNG's.

The display list and the rasteriser are plain computation: no window, GPU, file system, thread or clock. A test keeps their source that way, and CI builds the core for `wasm32-unknown-unknown`, as the viewer will run it.

## Measured against AutoCAD

AutoCAD 2026 on the machine that has it plotted the DWGs `agentdrafter build` wrote from the examples to PDF through its headless console, at 1:1 through `acad.ctb` with lineweights, and the PDFs were turned into pixels at 300 dots per inch. AgentDrafter rendered the same sheets at 1:1 and 300 dots per inch, with AutoCAD's own fonts in its font folders. Each pair was lined up by the shift that overlaps their ink best, and for every inked pixel of each image the distance to the nearest inked pixel of the other was measured. One pixel is 0.085 mm on paper.

| Sheet | AutoCAD's ink within 1 px of AgentDrafter's | within 2 px | AgentDrafter's ink within 1 px of AutoCAD's | within 2 px |
|---|---|---|---|---|
| `examples/one-line.yaml` | 100% | 100% | 99.98% | 100% |
| `examples/geometry.yaml` | 100% | 100% | 99.99% | 100% |
| `examples/hatch.yaml` | 100% | 100% | 99.97% | 100% |
| `examples/text.yaml` | 99.86% | 100% | 99.77% | 99.99% |

What still differs: AutoCAD sometimes leaves off the last dot of a dotted linetype at the end of an arc, which AgentDrafter draws; and anti-aliasing and the PDF's own rounding of pen widths to its 600-dots-per-inch grid move edges by up to a pixel.
