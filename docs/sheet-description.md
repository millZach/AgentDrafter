# The sheet description

A sheet description is the text file that states what is on a new sheet. It is the sheet's record: the DWG is built from it (ADR-0003). It is written in a strict subset of YAML, set out below, so that an agent can write it, a person can read it, and a mistake is an error that names its field rather than a guess.

So far a sheet description holds lines, polylines, arcs, circles, ellipses, splines and hatches, on layers that carry a colour, a linetype and a lineweight, text and multiline text in text styles that name a TrueType or an SHX font, inserts of blocks with attributes from a [block library](block-library.md), tables in table styles, multileaders in multileader styles and older leaders, dimensions in dimension styles, and, on a sheet drawn in one or more paper-space layouts, viewports onto model space, rectangular or clipped to a boundary, with layers frozen in one viewport alone. A template is written with the same keys, and leaves some values to a row of a [drawing set](drawing-set.md), which builds one sheet per row.

`agentdrafter schema` prints the same keys as a JSON Schema (draft 2020-12), with what each means, for an agent to read before it writes a sheet description. `agentdrafter schema block-library` prints the block library's.

## An example

```yaml
# The smallest sheet description: one line, in millimetres.
units: mm
objects:
  - line:
      from: [0, 0, 0]
      to: [100, 50, 0]
```

`agentdrafter build examples/one-line.yaml` writes `examples/one-line.dwg` in the 2018 format and prints what it wrote as JSON. `examples/geometry.yaml` holds every kind of line and curve, on layers with linetypes, `examples/hatch.yaml` holds solid and patterned hatches, and `examples/text.yaml` every kind of text, in every justification. `examples/text-widths.yaml` holds the fixed strings AgentDrafter's measured widths are compared with AutoCAD's on. `examples/blocks/` holds a block library and two sheets that insert its blocks, `examples/table.yaml` two tables, `examples/leaders.yaml` callouts on a panel layout, multileaders, some ending in a block of `examples/leader-blocks.yaml`, and older leaders, `examples/layout.yaml` a made-up control panel's backplane drawn at full size in model space and sheeted in a paper-space layout through three viewports at 1:4, 1:1 and 2:1, and `examples/layouts.yaml` a made-up junction box's backplate sheeted in two layouts of one DWG: one with viewports clipped to a circle and to a polyline with a round end, the other with a layer frozen in one viewport.

Every DWG a build writes carries AgentDrafter's build mark: a custom drawing property, `AgentDrafter` with the value `build`, which AutoCAD shows under DWGPROPS. `--overwrite` replaces only a DWG that carries it. A sheet built from a drawing set records its set in the same property, as `build; set=` and the set's folder relative to the DWG ([the drawing set](drawing-set.md#the-build-mark-names-the-set)); a sheet description built on its own records none.

A layer and a linetype:

```yaml
units: mm
linetypes:
  HIDDEN:
    description: Hidden __ __ __ __
    pattern: [6.35, -3.175]
layers:
  outline: {color: white, lineweight: 0.5}
  hidden: {color: yellow, linetype: HIDDEN, lineweight: 0.25}
objects:
  - circle: {center: [60, 40, 0], radius: 20, layer: outline}
  - line: {from: [40, 0, 0], to: [40, 80, 0], layer: hidden}
```

## What it holds

Keys are spelled as AutoCAD spells them: `color`, `center`.

| Key | Required | What it is |
|---|---|---|
| `units` | yes | The sheet's declared unit: `mm`, `cm`, `m`, `in` or `ft`. Every length on the sheet is in this unit unless it carries its own. It becomes the DWG's insertion unit. |
| `block_library` | no | The block library the sheet's inserts draw their blocks from: the path of its file. See [Inserts](#inserts). |
| `ltscale` | no | What every linetype on the sheet is enlarged by, AutoCAD's LTSCALE: a number more than 0, with no unit. Without it, 1. See [An object's own colour, linetype and lineweight](#an-objects-own-colour-linetype-and-lineweight). |
| `wipeout_frame` | no | Whether [wipeout](wipeouts.md) frames draw: `off` (default, hidden everywhere), `on` (drawn on screen and plotted), or `display` (drawn on screen, hidden when plotted), matching AutoCAD's `WIPEOUTFRAME`. |
| `linetypes` | no | The sheet's linetypes: a mapping from each linetype's name to its definition. |
| `layers` | no | The sheet's layers: a mapping from each layer's name to its colour, linetype and lineweight. |
| `hatch_patterns` | no | The sheet's hatch patterns: a mapping from each pattern's name to its lines. |
| `text_styles` | no | The sheet's text styles: a mapping from each style's name to its font and width factor. |
| `table_styles` | no | The sheet's table styles: a mapping from each style's name to what its tables' cells draw with. See [Table styles](#table-styles). |
| `mleader_styles` | no | The sheet's multileader styles: a mapping from each style's name to what its multileaders draw with. See [Multileader styles](#multileader-styles). |
| `dimension_styles` | no | The sheet's dimension styles: a mapping from each style's name to what its dimensions draw with. See [Dimension styles](#dimension-styles). |
| `objects` | yes | A list of objects, which may be empty, in drawing order. Each object is a mapping with one key, its type. |
| `paper` | no | The paper the sheet is drawn on, which the checker holds it to. See [Paper](#paper). |
| `page_setup` | no | The sheet's [page setup](plot.md#page-setups-and-plot-scale-pagesetup) (`PAGESETUP`): `paper`, `area` (`extents`, `window`, or `layout` on a paper-space layout), `window`, `scale` (`fit` or ratio such as `1:2`), `center`, `offset`, `plot_style` (CTB or STB file name), and `plot_transparency`. |
| `layout` | no | The paper-space layout the sheet is drawn in, with its `name` and optional `page_setup`. See [Layouts and viewports](#layouts-and-viewports). |
| `layouts` | no | The paper-space layouts the sheet is drawn in, when it is drawn in more than one, each with its `name`, `paper`, and optional `page_setup`. Not with `layout`. See [Several layouts](#several-layouts). |

### Linetypes

A linetype is a pattern of dashes, gaps and dots that a layer draws its objects with. Every drawing already has `Continuous`, the solid line; any other linetype a layer uses is defined here. `Continuous`, `ByLayer` and `ByBlock` are AutoCAD's own and cannot be defined again.

| Key | Required | What it is |
|---|---|---|
| `pattern` | yes | 2 to 12 lengths, repeated along each object: a positive length is a dash, a negative one a gap, and 0 a dot. It starts with a dash or a dot, then a gap, as in an AutoCAD `.lin` file. The lengths are in the declared unit and may carry their own. |
| `description` | no | What AutoCAD's linetype manager shows beside the name, at most 47 characters. |

### Layers

A layer is a named group of objects that draw with its colour, linetype and lineweight, unless they give their own ([below](#an-objects-own-colour-linetype-and-lineweight)). Every drawing already has layer `0`, which is white, continuous and of the default lineweight; listing `0` under `layers` sets its properties.

| Key | Required | What it is |
|---|---|---|
| `color` | no | An AutoCAD colour index from 1 to 255, or one of `red` (1), `yellow` (2), `green` (3), `cyan` (4), `blue` (5), `magenta` (6) or `white` (7). Without it, 7. |
| `linetype` | no | `Continuous`, or a name under `linetypes` written exactly as it is there. Without it, `Continuous`. |
| `lineweight` | no | The pen width the layer plots with, in millimetres, with or without `mm`, written out as digits with a `.` and digits if any, such as `0.35` or `0.50`, with no sign or exponent: one of AutoCAD's standard lineweights, 0, 0.05, 0.09, 0.13, 0.15, 0.18, 0.2, 0.25, 0.3, 0.35, 0.4, 0.5, 0.53, 0.6, 0.7, 0.8, 0.9, 1, 1.06, 1.2, 1.4, 1.58, 2 or 2.11. Without it, AutoCAD's default lineweight. |
| `plot_style` | no | The named plot style this layer plots with in an STB drawing (such as `Normal` or `Heavy`, setting `PSTYLEMODE = 0` in the DWG). Leave out in a colour-dependent (CTB) drawing. Objects also accept `plot_style` (`ByLayer`, `ByBlock`, or a named style). |
| `transparency` | no | Layer transparency percentage, a whole number from `0` (opaque, default) to `90`, written in DWG via AutoCAD's `AcCmTransparency` extended data and drawn across PNG, PDF and viewer. Objects also accept `transparency` (`ByLayer`, `ByBlock`, or `0..=90`). |
| `on` | no | Whether the layer is on, `true` or `false`. An object on a layer that is off draws nothing, in the PNG, the PDF or the viewer, and still counts towards the extents, as in AutoCAD. Without it, `true`. |
| `frozen` | no | Whether the layer is frozen. An object on a frozen layer draws nothing and does not count towards the extents, as in AutoCAD. Without it, `false`. |
| `locked` | no | Whether the layer is locked against editing, as AutoCAD locks it. It changes nothing a sheet draws; a [change set](change-sets.md#the-operations) does not edit an object on it. Without it, `false`. |
| `plot` | no | Whether the layer plots. An object on a layer that does not plot, such as a sheet's trim marks, is left out of the PDF and drawn in the PNG and the viewer, as AutoCAD's screen shows it, and counts towards the extents. Without it, `true`. |
| `description` | no | What AutoCAD's layer manager shows beside the name: one line of text, at most 255 characters. Without it, none. |

Zach decided on 2026-11-13 how a layer's states draw. An insert on a frozen layer draws nothing at all; one on a layer that is off, or does not plot on paper, hides only its block's objects on layer `0`, which take the insert's layer, as AutoCAD does. A sheet's [extents](query.md#extents), and so a sheet with no paper's plotted page and the checker's exact paper size, count objects on layers that are off or do not plot, and not those on frozen layers: AutoCAD 2026's extents of a made-up probe did the same. The DWG holds each state, and [`query layers`](query.md#layers) lists them.

A lineweight is the one number on a sheet that is not in the declared unit. It is a width on paper, not a length on the drawing: a 0.35 mm pen is 0.35 mm on a sheet drawn in inches. Written in any unit but `mm`, it is an error.

A layer or linetype name is 1 to 255 characters, does not start or end with a space, and holds none of `< > / \ " : ; ? * | = , `` ` `` and no control characters. AutoCAD ignores case in names, so two names that differ only in case are an error, as the same key given twice would be. An object or a layer names its layer or linetype exactly as it is defined.

### Hatch patterns

A hatch pattern is one or more families of parallel lines that a hatch fills its area with, defined the way an AutoCAD `.pat` file defines one. `SOLID`, AutoCAD's solid hatch, needs no definition; every other pattern a hatch names is defined here. AutoCAD's own pattern files are not copied into AgentDrafter, so a pattern such as `ANSI31` is defined here too if a sheet wants it.

```yaml
hatch_patterns:
  # Blocks 20 long and 10 high, each course shifted by half a block.
  COURSES:
    lines:
      - {angle: 0, delta: [0, 10]}
      - {angle: 90, delta: [10, 10], dashes: [10, -10]}
```

A pattern takes one key, `lines`: a list of at least one line, each a family of parallel lines. A line takes:

| Key | Required | What it is |
|---|---|---|
| `angle` | no | The direction of the lines, an angle. Without it, 0. |
| `origin` | no | A point one of the lines passes through, as two lengths `[x, y]` in the hatch's plane. Without it, `[0, 0]`. |
| `delta` | yes | The step from one line to the next, as two lengths `[along, across]`: `along` runs in the lines' direction and shifts where the dashes start, and `across`, which is not 0, is the distance between neighbouring lines. These are a `.pat` file's delta-x and delta-y. |
| `dashes` | no | The dashes, gaps and dots each line repeats, as lengths: a positive length is a dash, a negative one a gap, and 0 a dot. It holds at least one dash or dot and at least one gap, in any order. Without it, the lines are continuous. |

The lengths are in the declared unit and may carry their own; a hatch's `scale` enlarges them all.

A hatch pattern name is 1 to 34 characters, written in capitals, as AutoCAD keeps it: AutoCAD reads a name written as `Fine` as `FINE`. It holds no spaces, none of the characters a layer name cannot hold, and no control characters. `SOLID` and `_USER`, AutoCAD's own, cannot be defined.

### Text styles

A text style is a font that text draws with, and how wide. Every drawing already has `Standard`, whose font is AutoCAD's `txt.shx`; listing `Standard` under `text_styles` sets its font and width factor. A text style's name follows the rules for layer and linetype names above, so `Standard` is written just so: `standard` or `STANDARD` under `text_styles` is an error, since AutoCAD would take it for `Standard`.

| Key | Required | What it is |
|---|---|---|
| `font` | yes | The font, by its file name alone: an AutoCAD SHX font such as `romans.shx`, or a TrueType font such as `arial.ttf`. The extension, `.shx` or `.ttf`, may be in any case. The name holds no folder and none of `/ \ < > : " \| ? *`. |
| `width_factor` | no | How wide characters are drawn, as a share of their normal width: from 0.01 to 100, with 1 normal and 0.8 narrower. Text in the style takes it unless it states its own. Without it, 1. |

The DWG names the font, and AutoCAD finds it on the machine that opens the drawing. AgentDrafter reads the font only to measure text, which it does to place justified text and for `agentdrafter measure-text`; see [Fonts](#fonts).

### Table styles

A table style is what a [table](#tables)'s cells draw with unless they give their own: AutoCAD's TABLESTYLE. Every drawing already has `Standard`: on a metric sheet its title rows are 6 mm high, its header and data rows 4.5 mm and its margins 1.5 mm, and on an imperial sheet 0.25, 0.18 and 0.06 in, all in the text style `Standard`, its titles and headers middle centre and its data top centre, and its borders `ByBlock`. Listing `Standard` under `table_styles` sets what it draws with; it is written `Standard` exactly, as a text style's name is.

```yaml
table_styles:
  schedule:
    text_style: notes
    text_height: 2.5
    horizontal_margin: 1
    vertical_margin: 1
    border_lineweight: 0.35
    title: {text_height: 3.5}
    data: {justify: middle_left}
```

| Key | Required | What it is |
|---|---|---|
| `text_height` | yes | The height of the capitals of every kind of row's text, unless the row gives its own: a length more than 0. |
| `text_style` | no | The text style every kind of row's text draws with, unless the row gives its own: `Standard` or a name under `text_styles`. Without it, `Standard`. |
| `horizontal_margin` | no | The space between a cell's sides and its text: a length of 0 or more. Without it, 1.5 mm on a metric sheet and 0.06 in on an imperial one. |
| `vertical_margin` | no | The space between a cell's top and bottom and its text, as `horizontal_margin` is. |
| `border_color` | no | The colour the borders draw in, as an [object's own colour](#an-objects-own-colour-linetype-and-lineweight) is written. Without it, `ByBlock`: the table's own colour, or its layer's where that is `ByLayer`. |
| `border_lineweight` | no | The lineweight the borders draw with, as an object's own lineweight is written. Without it, `ByBlock`. |
| `title`, `header`, `data` | no | What the cells of each kind of row draw with, where it is not the style's own: `text_style`, `text_height` and `justify`, one of the nine justifications of `mtext`. Without `justify`, titles and headers are `middle_center` and data `top_center`, as in AutoCAD. |

Every border of a table style draws in one pen, and every kind of row has the same margins; a cell is not filled. AutoCAD's TABLESTYLE offers more, such as a pen for each edge of each kind of row, a fill and a text colour; a DWG table style that uses them, in its rows or in any of the cell styles AutoCAD 2010 and later keep, such as turned text, is counted as not read, and so is a table in it.

### Multileader styles

A multileader style is what a [multileader](#leaders) draws with unless it gives its own: AutoCAD's MLEADERSTYLE. Every drawing already has `Standard`, as AutoCAD 2026's templates have it: on a metric sheet text and arrows 4 mm, a landing 8 mm and a gap of 2 mm, and on an imperial sheet 0.18, 0.36 and 0.09 in; text in the text style `Standard`, a closed filled arrowhead, the landing at the middle of the text's top line, and the leader lines `ByBlock`. Listing `Standard` under `mleader_styles` sets what it draws with; it is written `Standard` exactly, as a text style's name is. Every key of a style is optional: one left out is as the sheet's unit's `Standard` has it.

```yaml
mleader_styles:
  callout:
    text_style: notes
    text_height: 3
    arrow_size: 3
    landing: 6
    landing_gap: 1.5
    leader_lineweight: 0.25
```

| Key | What it is |
|---|---|
| `text_style` | The text style its text draws with: `Standard` or a name under `text_styles`. Without it, `Standard`. |
| `text_height` | The height of its text's capitals: a length more than 0. |
| `arrowhead` | The arrowhead: `closed_filled`, a filled triangle a third as wide as it is long; `dot`, a filled dot as wide as the arrow is long; `open`, the triangle's two sides; or `none`. These are AutoCAD's named arrowheads that AgentDrafter draws exactly; the others wait. |
| `arrow_size` | How long the arrowhead is: a length more than 0. |
| `landing` | How long the landing is, the short horizontal line, AutoCAD's dogleg, that the leader lines meet and the text sits beyond: a length of 0 or more, 0 for none. |
| `landing_gap` | The space between the landing's end and the text: a length of 0 or more. |
| `attachment` | Where the landing meets the text, up the text: `top_of_top_line`, `middle_of_top_line`, `middle_of_text`, `middle_of_bottom_line` or `bottom_of_bottom_line`. AutoCAD's attachments that underline the text wait. |
| `leader_color` | The colour the leader lines, landing and arrowheads draw in, as an [object's own colour](#an-objects-own-colour-linetype-and-lineweight) is written. Without it, `ByBlock`: the multileader's own colour, or its layer's where that is `ByLayer`. |
| `leader_lineweight` | The lineweight they draw with, as an object's own lineweight is written. Without it, `ByBlock`. |

A multileader style's content is multiline text and its leader lines are straight; its landing is horizontal and its text upright. A multileader may still take a block as its content ([Block content](#block-content)). AutoCAD's MLEADERSTYLE offers more, such as a block as the style's own content, spline leader lines, a frame round the text, text above or below a vertical landing, and annotative scaling; a DWG multileader style that uses them is counted as not read, and so is a multileader in it.

### Dimension styles

A dimension style is what a [dimension](#dimensions) draws with: AutoCAD's DIMSTYLE. A sheet has its unit's own without listing it, as AutoCAD 2026's templates have it: `ISO-25` on a metric sheet, of the `acadiso` template, and `Standard` on an imperial one, of the `acad` template. Listing it under `dimension_styles` sets what it draws with; AutoCAD's names are written exactly, as a text style's are. Every other style starts from the sheet's unit's own, and every key is optional: one left out is as that style has it.

```yaml
dimension_styles:
  cutout:
    arrowhead: open
    arrow_size: 3
    text_height: 3
    suffix: ' mm'
  reference:
    placement: centered
    precision: 1
    suppress_trailing_zeros: true
  feet-and-inches:
    unit_format: architectural
    precision: 4
    angular_unit: degrees_minutes_seconds
    angular_precision: 2
```

| Key | AutoCAD's | What it is | `ISO-25` | `Standard` |
|---|---|---|---|---|
| `arrowhead` | DIMBLK | The arrowhead at each end of the dimension line, of those a [multileader style](#multileader-styles) names: `closed_filled`, `dot`, `open` or `none`. | `closed_filled` | `closed_filled` |
| `arrow_size` | DIMASZ | How long an arrowhead is: a length more than 0. | 2.5 mm | 0.18 in |
| `text_style` | DIMTXSTY | The text style its text draws with: `Standard` or a name under `text_styles`. | `Standard` | `Standard` |
| `text_height` | DIMTXT | The height of its text's capitals: a length more than 0. | 2.5 mm | 0.18 in |
| `gap` | DIMGAP | The space round its text: a length of 0 or more. | 0.625 mm | 0.09 in |
| `extension_offset` | DIMEXO | How far an extension line starts from its point: a length of 0 or more. | 0.625 mm | 0.0625 in |
| `extension_beyond` | DIMEXE | How far an extension line reaches past the dimension line: a length of 0 or more. | 1.25 mm | 0.18 in |
| `line_extension` | DIMDLE | How far the dimension line reaches past the extension lines when its arrowhead is `none`: a length of 0 or more. | 0 | 0 |
| `center_mark` | DIMCEN | How far the arms of the cross that marks a circle's centre reach, 0 for none. It is drawn when a radius or diameter dimension's text is centred and outside the circle. | 2.5 mm | 0.09 in |
| `baseline_spacing` | DIMDLI | How much further out a [baseline dimension](change-sets.md#dimensions-from-others)'s line, or an angular one's arc, is than the one it is added from: a length of 0 or more. | 3.75 mm | 0.38 in |
| `placement` | DIMTAD, DIMTIH, DIMTOH, DIMTOFL | Where its text sits, one of AutoCAD's two defaults: `above`, aligned with the dimension line and above it, which then runs between the extension lines even when the arrowheads are outside them; or `centered`, horizontal in the middle of the dimension line, which breaks for it. | `above` | `centered` |
| `unit_format` | DIMLUNIT | How a length is written: `scientific`, `1.25E+01`; `decimal`, `12.50`; `engineering`, feet and decimal inches, `1'-0.50"`; `architectural`, feet and fractional inches, `1'-0½"`; `fractional`, `12½`; or `windows_desktop`, decimal with a period and its thousands grouped with commas. Engineering and architectural count the sheet's unit as an inch, so they are for a sheet in `in`; see [Unit formats](#unit-formats). | `decimal` | `decimal` |
| `precision` | DIMDEC | How many decimals a length shows, or for an architectural or fractional one how fine its fraction is, 1/2 at 1 to 1/256 at 8: 0 to 8. | 2 | 4 |
| `fraction_format` | DIMFRAC | How an architectural or fractional length's fraction shows: `horizontal`, stacked over a bar; `diagonal`, stacked across a slash; `not_stacked`, on the line, `12 1/2`. | `horizontal` | `horizontal` |
| `decimal_separator` | DIMDSEP | `period`, `comma` or `space`, between a decimal length's or an angle's whole part and its decimals. | `comma` | `period` |
| `suppress_leading_zeros` | DIMZIN 4 | Whether 0.5 shows as .5: `true` or `false`. | `false` | `false` |
| `suppress_trailing_zeros` | DIMZIN 8 | Whether 2.50 shows as 2.5, and 2.00 as 2. | `true` | `false` |
| `suppress_zero_feet` | DIMZIN 0 and 3 | Whether an engineering or architectural length leaves out zero feet, `0'-6"` showing as `6"`. | `true` | `true` |
| `suppress_zero_inches` | DIMZIN 0 and 2 | Whether it leaves out zero inches, `1'-0"` showing as `1'`. | `true` | `true` |
| `angular_unit` | DIMAUNIT | How an angle is written: `decimal_degrees`, `45.5°`; `degrees_minutes_seconds`, `45°30'0"`; `gradians`, `50.56g`; or `radians`, `0.794r`. | `decimal_degrees` | `decimal_degrees` |
| `angular_precision` | DIMADEC | How many decimals an angle shows, 0 to 8; AutoCAD takes -1, as many as a length, only as a variable, and its audit refuses it in a style. In degrees, minutes and seconds, 0 shows whole degrees, 1 and 2 minutes, 3 and 4 seconds, and 5 to 8 a decimal of seconds to four. | 0 | 0 |
| `suppress_angular_leading_zeros` | DIMAZIN 1 | Whether an angle's 0.5 shows as .5. | `false` | `false` |
| `suppress_angular_trailing_zeros` | DIMAZIN 2 | Whether an angle's 2.50 shows as 2.5. | `false` | `false` |
| `prefix`, `suffix` | DIMPOST | What comes before and after every value, a length or an angle, as AutoCAD writes them: one line, with no `<`, `>`, `\`, `{`, `}` or `%%`, which AutoCAD reads in DIMPOST as a control code. Either takes the place of a radius's R and a diameter's ∅, as in AutoCAD. | none | none |
| `scale` | DIMSCALE | What every length of the style is multiplied by: a number more than 0. | 1 | 1 |

The DWG writes a style as AutoCAD 2026's own of the sheet's unit with these set, and reads one back when every value is one of these: a style that draws tolerances, alternate units, ticks, fixed extension lines, a frame or fill round its text, or another placement or fit than AutoCAD's defaults is counted as not read, and so is a dimension drawn with it.

### Objects

Every object takes a `layer` key: the name of the layer it sits on, `0` or one under `layers`. Without it, the object sits on layer `0`. An object draws with its layer's colour, linetype and lineweight unless it gives its own, under `color`, `linetype`, `lineweight` and `linetype_scale`; see [An object's own colour, linetype and lineweight](#an-objects-own-colour-linetype-and-lineweight). Every object may also take an `id` key, its ID in the record, and a `name` key, a name an agent selects it by; see [IDs and names](#ids-and-names). On a sheet drawn in a layout, every object may take a `space` key too, `model`, `paper` or the name of the layout it is in; see [Layouts and viewports](#layouts-and-viewports).

| Type | Keys | What it is |
|---|---|---|
| `line` | `from`, `to` | A straight line between two points. |
| `polyline` | `points`; `closed`, `width` | Segments joined end to end through the points, in order: at least 2, or 3 when closed. Each is straight, or an arc where its point says so, and as wide as its widths; see [Polylines](#polylines). `closed: true` adds a segment from the last point back to the first; without it, `false`. When every point has the same `z`, it is AutoCAD's lightweight polyline at that elevation; otherwise it is a 3D polyline, which has no arcs and no widths. |
| `arc` | `center`, `radius`, `start_angle`, `end_angle` | Part of a circle, drawn anticlockwise from its start angle to its end angle. The angles must differ once whole turns are taken off; a whole circle is a `circle`. |
| `circle` | `center`, `radius` | A circle. |
| `ellipse` | `center`, `major_axis`, `ratio`; `start_angle`, `end_angle` | An ellipse. `major_axis` runs from the centre to one end of the major axis, as `[dx, dy, dz]`, with `dz` 0. `ratio` is the minor axis over the major axis, more than 0 and at most 1. With `start_angle` and `end_angle`, measured anticlockwise from the major axis, only the part from one to the other is drawn; give both or neither. |
| `spline` | `fit_points` or `control_points`; `degree` | A smooth curve. With `fit_points`, at least 3 points it passes through, no two in a row the same; AutoCAD fits a cubic through them, as its SPLINE command does by default. With `control_points`, the control points of a B-spline that starts at the first and ends at the last, of `degree` 1 to 10 (without it, 3), with more control points than its degree. Not both. |
| `hatch` | `pattern`, `boundary`; `scale`, `angle` | An area filled solid or with a pattern, inside a boundary; see below. |
| `donut` | `center`, `outside_diameter`; `inside_diameter` | A ring, or a filled dot when its inside diameter is 0, as AutoCAD's DONUT draws it, such as a wire's junction dot. See [Polylines](#polylines). |
| `solid` | `corners` | A filled triangle or quadrilateral, AutoCAD's 2D solid: 3 or 4 corners sharing one `z`, listed in order round the outline. AgentDrafter writes them in AutoCAD's own order, which crosses the third and fourth. |
| `revision_cloud` | `rectangle` or `polygon`, `arc_length`; `style`, `reverse` | A revision cloud, as AutoCAD's REVCLOUD draws it: a closed lightweight polyline of arcs around a rectangle (`{from, to}`) or closed polygon of 3+ points, in `normal` or tapered `calligraphy` style, carrying AutoCAD's `RevcloudProps` extended data. See [Revision clouds](revision-clouds.md). |
| `text` | `contents`, `position`, `height`; `style`, `width_factor`, `rotation`, `justify` | A single line of text, as AutoCAD's TEXT draws it. See [Text](#text). |
| `mtext` | `contents`, `position`, `height`; `width`, `style`, `width_factor`, `rotation`, `justify` | Text of one or more lines in a box, as AutoCAD's MTEXT draws it. See [Text](#text). |
| `insert` | `block`, `position`; `scale`, `rotation`, `attributes` | A block from the sheet's block library, placed as AutoCAD's INSERT places it. See [Inserts](#inserts). |
| `table` | `position`, `columns`, `rows`, `cells`; `title`, `header`, `table_style` | A grid of cells with text, as AutoCAD's TABLE draws it. See [Tables](#tables). |
| `mleader` | `leaders`, `content`; `landing`, `arrowhead`, `arrow_size`, `mleader_style` | One or more leader lines from their arrows to a landing, and multiline text or a block beyond it, as AutoCAD's MLEADER draws it. See [Leaders](#leaders). |
| `leader` | `points`; `arrowhead`, `arrow_size`, `gap`, `annotation` | One leader line, arrow first, with an optional multiline text annotation beyond its end, as AutoCAD's older LEADER draws it. See [Leaders](#leaders). |
| `dimension` | `kind`, `location`, and its kind's own keys; `text`, `dimension_style`, `continued` | A dimension that measures a length or an angle and shows it, as AutoCAD's DIMLINEAR, DIMALIGNED, DIMRADIUS, DIMDIAMETER, DIMANGULAR and DIMORDINATE draw it. See [Dimensions](#dimensions). |
| `viewport` | `center`, `width`, `height` or `clip`, `view_center`, `scale`; `locked`, `frozen_layers` | A window on the paper that shows model space at a scale, as AutoCAD's MVIEW makes one, on a sheet drawn in a layout: rectangular, or clipped to a boundary, with layers frozen in it alone. See [Layouts and viewports](#layouts-and-viewports). |

Keys before the semicolon are required; keys after it are not. A `radius` is a length more than 0.

Arcs, circles and ellipses lie in the plane parallel to XY at their centre's `z`, and so does a hatch, at the `z` its boundary's points share. Lines, polylines and splines may run anywhere in 3D.

### An object's own colour, linetype and lineweight

Every object, an insert and a block's objects and attribute definitions among them, may take a colour, a linetype, a lineweight and a linetype scale of its own, as AutoCAD's PROPERTIES gives them. Each is `ByLayer` unless the object gives it: the object then draws with its layer's.

| Key | What it is |
|---|---|
| `color` | `ByLayer`, `ByBlock`, or an AutoCAD colour index as a layer takes it: 1 to 255, or `red` to `white`. Without it, `ByLayer`. |
| `linetype` | `ByLayer`, `ByBlock`, `Continuous`, or a name under `linetypes`, written exactly as it is there. Without it, `ByLayer`. |
| `lineweight` | `ByLayer`, `ByBlock`, `Default` (AutoCAD's default lineweight, 0.25 mm as installed), or a standard lineweight in millimetres as a layer takes it. Without it, `ByLayer`. |
| `linetype_scale` | What the object's linetype is enlarged by, on top of the sheet's `ltscale`: a number more than 0, with no unit. Without it, 1. |
| `plot_style` | `ByLayer`, `ByBlock`, or a named plot style from an STB table (such as `Normal` or `Heavy`). Without it, `ByLayer`. |
| `transparency` | `ByLayer`, `ByBlock`, or a transparency percentage from `0` (opaque) to `90`. Without it, `ByLayer`. |

`ByLayer`, `ByBlock` and `Default` are written as here, in that case. A true colour is not offered yet ([#106](https://github.com/millZach/AgentDrafter/issues/106)).

`ByBlock` is for a block's objects: one drawn by an insert takes the insert's own colour, linetype or lineweight, or the insert's layer's where the insert's are `ByLayer`, so one block can draw red on one sheet and white on another. An object on the sheet itself that is `ByBlock` draws as AutoCAD draws it, white, continuous and at the default lineweight. A linetype a block's object names comes to the sheet from the block library with the block, as its layers do.

A linetype's dashes and gaps are drawn at their defined lengths times the sheet's `ltscale` times the object's `linetype_scale`, as AutoCAD multiplies LTSCALE and an object's linetype scale. An insert does not enlarge them ([Inserts](#inserts)).

```yaml
units: mm
ltscale: 2
linetypes:
  DASHED: {pattern: [5, -2.5]}
layers:
  wiring: {color: white}
objects:
  # Drawn green and dashed at 0.35 mm, whatever the layer says; its dashes
  # are 5 x 2 x 0.5 = 5 mm long.
  - line: {from: [0, 0, 0], to: [100, 0, 0], layer: wiring, color: green, linetype: DASHED, lineweight: 0.35, linetype_scale: 0.5}
  # Drawn in the wiring layer's white, at its own 0.7 mm.
  - line: {from: [0, 10, 0], to: [100, 10, 0], layer: wiring, lineweight: 0.7}
```

The DWG writes them as AutoCAD does, on each object and in the drawing's LTSCALE, and reading a DWG back gives them; an object with a true colour is not read yet. `change` sets them, and a bare null takes one away, so the object draws with its layer's again.

### Polylines

A polyline's point is `[x, y, z]`, or a mapping when the segment that starts there is an arc or has a width (Zach decided this on 2026-11-13):

| Key | Required | What it is |
|---|---|---|
| `point` | yes | Where the point is. |
| `bulge` | no | The segment's arc as AutoCAD keeps it: the tangent of a quarter of its included angle, positive anticlockwise. 1 is a half circle anticlockwise, -1 clockwise, 0 a straight segment. A number with no unit. |
| `arc_angle` | no | The segment's arc by its included angle in degrees: more than -360 and less than 360, positive anticlockwise, 180 a half circle. Not with `bulge`. A DWG read back gives `bulge`. |
| `start_width`, `end_width` | no | The segment's width at its start and at its end: lengths of 0 or more. The width changes evenly between them. Without them, 0. |

The segment a point starts runs to the next point, or, from the last point of a closed polyline, back to the first; the last point of an open polyline starts none and gives neither. `width` on the polyline gives every segment one width, AutoCAD's constant width, and is not given with a point's own widths. A 3D polyline takes no arc and no width.

A segment with no width draws as a line, with its lineweight, and its linetype starts again at each point, as AutoCAD draws a polyline. A wide segment fills a band centred on its path, with square ends and no lineweight, as AutoCAD 2026 plotted a made-up probe: where two wide segments meet, their edges are carried on to where they cross, a mitred corner. A wide segment in a linetype fills its band under each dash.

```yaml
# A rounded corner, an arrowhead and a junction dot.
- polyline:
    points:
      - [0, 0, 0]
      - {point: [10, 0, 0], arc_angle: 90}
      - [12, 2, 0]
- polyline: {points: [[20, 0, 0], {point: [26, 0, 0], start_width: 2, end_width: 0}, [28, 0, 0]]}
- donut: {center: [12, 2, 0], outside_diameter: 1}
```

A `donut`'s `outside_diameter` is more than 0, and its `inside_diameter` is 0 or more and less than that; without it, 0, a filled dot. It is written as AutoCAD's DONUT writes it, a closed polyline of two half circles on the ring's middle, as wide as the ring, so a DWG reads it back as that polyline.

### Hatches

```yaml
- hatch:
    pattern: COURSES
    scale: 2
    angle: 30
    boundary:
      - polyline: {points: [[0, 0, 0], [80, 0, 0], [80, 40, 0], [0, 40, 0]]}
      - edges:
          - line: {from: [30, 15, 0], to: [50, 15, 0]}
          - arc: {center: [50, 20, 0], radius: 5, start_angle: 270, end_angle: 90}
          - line: {from: [50, 25, 0], to: [30, 25, 0]}
          - arc: {center: [30, 20, 0], radius: 5, start_angle: 90, end_angle: 270}
    layer: section
```

| Key | Required | What it is |
|---|---|---|
| `pattern` | yes | `SOLID` for a solid hatch, or the name of a pattern under `hatch_patterns`, written exactly as it is there. |
| `scale` | no | How much the pattern is enlarged: a number more than 0, with no unit. Without it, 1. Not given with `SOLID`. |
| `angle` | no | How far the pattern is turned, an angle. Without it, 0. Not given with `SOLID`. |
| `origin` | no | The point the pattern is laid from, AutoCAD's hatch origin, `[x, y, z]`, its `z` not used: every line of the pattern runs through it, moved by the line's own `origin` turned and enlarged with the hatch. Without it, `[0, 0, 0]`. Not given with `SOLID`. A change set that moves or turns the hatch moves or turns its origin with its boundary, so the pattern goes with it. A DWG keeps the origin inside each pattern line, so a hatch read back from one is laid from `[0, 0, 0]` with its lines' origins carrying it. |
| `boundary` | yes | The closed loops that bound the area: a list of at least one. |
| `layer` | no | The layer the hatch sits on, as for every object. |

Each loop is a mapping with one key:

- `polyline`, which takes `points`: straight segments through at least 3 points, in order, always closed back to the first.
- `edges`: a list of at least 2 lines and arcs joined end to end, the last ending where the first starts. An edge is a mapping with one key: `line`, which takes `from` and `to`, or `arc`, which takes `center`, `radius`, `start_angle` and `end_angle` and runs anticlockwise from its start angle to its end angle, as an `arc` object does. Each line starts where the edge before it ends. An arc may be walked either way: forwards when it starts where the edge before it ends, backwards when it ends there. A loop's first arc is walked whichever way lets the second edge join it. A whole circle is two arcs. Two ends join when they are within a billionth of the largest coordinate or radius in the loop, or of one unit if that is more.

A point inside an odd number of loops is filled, as AutoCAD's normal island detection reads a hatch: a loop inside another is a hole, and a loop inside that hole is filled again. Every point of every loop has the same `z`. A hatch draws no outline; a sheet that wants one draws its boundary again as lines, polylines and arcs.

A hatch is written as AutoCAD's HATCH: not associative, so it does not follow its outline when the outline is edited, with every loop marked as one drawn by selecting objects. A pattern from `hatch_patterns` is written as a custom pattern, its lines stored in the hatch, as a DWG stores every hatch's pattern.

### Text

```yaml
units: mm
text_styles:
  notes: {font: romans.shx, width_factor: 0.8}
  title: {font: arial.ttf}
objects:
  - text: {contents: PUMP P-101, position: [10, 20, 0], height: 2.5, style: notes}
  - text: {contents: GENERAL ARRANGEMENT, position: [100, 5, 0], height: 7, style: title, justify: bottom_center}
  - mtext:
      contents: |
        NOTES:
        1. ALL DIMENSIONS IN MILLIMETRES.
      position: [10, 60, 0]
      height: 2.5
      width: 80
      style: notes
```

Text and multiline text lie in the plane parallel to XY at their position's `z`.

| Key | Of | What it is |
|---|---|---|
| `contents` | both | What the text says, exactly as it shows. It must show: not empty, written `''`, and not only white space. No control characters. A `text` is one line; in `mtext` a line break starts a new line, and line breaks at the end are dropped, so a YAML block (`\|`) can hold it. |
| `position` | both | The point the text is justified at: a point. |
| `height` | both | The height of the capitals: a length more than 0. |
| `width` | `mtext` | The width lines wrap in: a length of 0 or more. Without it, 0, and lines break only where the contents break them. |
| `style` | both | The text style: `Standard` or a name under `text_styles`, written exactly as it is there. Without it, `Standard`. |
| `width_factor` | both | How wide characters are drawn, from 0.01 to 100. Without it, a `text` takes its style's width factor, as AutoCAD's TEXT command does, and `mtext` draws at its style's. |
| `rotation` | both | An angle: degrees anticlockwise from the X axis. Without it, 0. |
| `justify` | both | Where the text sits against its position. Without it, `left` for `text` and `top_left` for `mtext`. |

A `text` is justified as AutoCAD's TEXT command justifies it: `left`, `center` or `right` on the baseline; `middle`, centred both ways on what the text draws; or `top_left`, `top_center`, `top_right`, `middle_left`, `middle_center`, `middle_right`, `bottom_left`, `bottom_center` or `bottom_right`, where top is the top of the capitals, middle half their height and bottom the font's lowest descender. AutoCAD's `aligned` and `fit`, which fit text between two points, are not offered. An `mtext` is justified by its box: one of the nine `top_left` to `bottom_right`.

Contents show as written. AutoCAD reads some characters in text as codes, so AgentDrafter writes them so that they show as themselves: a run of `%` in a `text`, which AutoCAD would read as a code such as `%%d`, and a `\`, `{` or `}` in `mtext`, which AutoCAD would read as formatting. Formatting codes are not read: an `mtext` that wants bold or a second font waits for a later ticket. A `text` cannot hold `\U+` and four hexadecimal digits, or `\M+` and five, which AutoCAD reads as a character code; write the character itself. An `mtext`'s `width_factor` is written as AutoCAD's text editor writes it, as a `\W` code at the start of its contents.

A justified `text` records in the DWG where its baseline starts as well as where it is justified, as AutoCAD does. AutoCAD draws the text from the start, so a build works the start out from the font's widths, by AutoCAD's own rules; when the font is not in a font folder, the start comes from its look-alike and may sit a little away from where AutoCAD would put it, until the text is edited in AutoCAD. The build's result names every such font, and its reply warns of each at its top.

### Inserts

```yaml
units: mm
block_library: blocks.yaml
layers:
  symbols: {color: yellow, lineweight: 0.35}
objects:
  - insert:
      block: MARKER
      position: [100, 60, 0]
      scale: 1.5
      rotation: 30
      attributes: {TAG: A-2, NUMBER: '102'}
      name: marker-a2
      layer: symbols
```

A block is defined once, in the block library of the drawing set, and inserted on any sheet that names the library under `block_library`. [The block library](block-library.md) sets out what a library holds. `block_library` is the path of the library's file, relative to the folder the sheet description is in, with `/` between folders on every system, such as `blocks.yaml` or `../library/blocks.yaml`. It does not start with `/` and holds none of `\ : < > " | ? *`. The library is in the sheet's declared unit: a library in another unit is an error.

| Key | Required | What it is |
|---|---|---|
| `block` | yes | The name of a block in the block library, written exactly as it is there. |
| `position` | yes | Where the block's base point sits: a point. |
| `scale` | no | How much the block is enlarged, the same along every axis: a number more than 0, with no unit. Without it, 1. |
| `rotation` | no | An angle: degrees anticlockwise from the X axis. Without it, 0. |
| `mirrored` | no | Whether the block is mirrored, its X axis reversed before it is turned, as AutoCAD's MIRROR leaves an insert: `true` or `false`. Without it, `false`. See below. |
| `readable_attributes` | no | Whether its attributes read forwards, left to right or upwards, however it is turned, as a mirror leaves them: `true` or `false`. A mirrored insert's always do; one mirrored back keeps them so. Without it, `false`: they turn with the insert. See below. |
| `attributes` | no | The values of the block's attributes: a mapping from each attribute's tag to its value. A value is one line of text, as a `text`'s contents are, and may be empty, written `''`. A tag the block defines more than once takes a list of values instead, one for each in the order the block defines them, such as `NOTE: [first, second]` ([below](#a-tag-defined-more-than-once)). An attribute the insert leaves out takes its definition's default, and so does one whose value is a bare null, such as `TAG:` ([the YAML subset](#the-yaml-subset), rule 8). A [constant attribute](block-library.md#attribute-modes) takes no value from an insert. A tag written in lowercase is taken in capitals, as the block keeps it. |
| `state` | no | For a [dynamic block](dynamic-blocks.md), a mapping from dynamic parameter name to chosen value (visibility state, lookup row, flip, linear distance, or rotation). Omitted or `null` parameters take their defaults. |
| `layer` | no | The layer the insert sits on, as for every object. |

An insert takes an `id` and a `name` as every object does ([IDs and names](#ids-and-names)).

A block that is not in the library, or an attribute tag the block does not define, is an error that lists the library's blocks or the block's attribute tags. An insert's attributes draw where the block's attribute definitions are, moved, scaled and turned with the insert.

A mirrored insert, `mirrored: true`, is AutoCAD's mirrored block reference (#242): its block's X axis is reversed, then it is scaled and turned, so what its block draws, a constant attribute among it, draws mirrored, as in AutoCAD. The DWG keeps it as AutoCAD's MIRROR leaves one, its X scale the negative of its Y and Z scales, and reading a DWG back gives `mirrored: true` for an insert scaled so, and for no other negative scale. Its attributes sit where the mirror puts them but read forwards, as AutoCAD's MIRRTEXT 0 leaves them: each reads along its mirrored direction, or that turned a half turn, whichever reads left to right or upwards, a direction in [0, 90] or between 270 and 360, with its definition's justification. The [`mirror` operation](change-sets.md#rotate-scale-and-mirror) makes one. Mirrored back, by a second mirror, an insert is no longer mirrored, but its attributes keep reading forwards, as AutoCAD's MIRRTEXT 0 leaves them after every mirror: `readable_attributes: true`, by which each reads along its definition's direction turned with the insert, or that turned a half turn, by the same rule. Reading a DWG back gives `readable_attributes: true` for an insert that is not mirrored whose attributes read so where its turn alone would not leave them. A scale is the same along every axis and more than 0: a stretched insert waits for a later ticket.

#### A tag defined more than once

```yaml
  - insert: {block: REVISIONS, position: [0, 0, 0], attributes: {NOTE: [ISSUED FOR REVIEW, null, AS BUILT], 'NOTE#1': A}}
```

A block may define one tag on more than one attribute definition, as AutoCAD allows ([the block library](block-library.md#a-tag-defined-more-than-once)). An insert gives that tag's values as a list, in the order the block defines them: the first item is the tag's first occurrence, the second its second, and so on. Zach decided on 2026-11-13 (#99):

- A tag the block defines once takes one value, as before; a list there is an error.
- A tag it defines more than once takes a list, and one value there is an error asking for a list.
- The list may be shorter than the times the block defines the tag: the occurrences it does not reach take their definitions' defaults. It may not be longer: that is an error saying how many times the block defines the tag.
- An item written as a bare null, such as `null`, takes its definition's default, so a list can skip an occurrence.
- A constant definition carries no value, so the list covers only the tag's other definitions, in order. A tag with one carried definition and one constant one takes one value.
- In a template, each item may be a value from the row, `NOTE: [{row: REV_NOTE}, AS BUILT]`, and a tag defined once takes `{row: NAME}` as before.

A tag written in lowercase, such as `note`, is taken in capitals, `NOTE`, as AutoCAD keeps every tag, and the reply carries a `tag_in_capitals` notice saying where it is written: the build's `notices` for a sheet built on its own, and the sheet's `notices` in a drawing set's build or a change set's reply. Giving one tag twice, in two cases, such as `{note: A, NOTE: B}`, is an error.

A built sheet holds the blocks it inserts, as the library defines them, and the library's layers, linetypes and text styles those blocks use. A layer, linetype or text style the sheet defines too must be defined the same in both, or the build stops with an error at the sheet's definition: otherwise a block would not draw on the sheet as the library defines it. Layer `0`, `Continuous` and `Standard`, when the library does not list them, are the sheet's own, as they are in AutoCAD. The sheet's own objects, an insert among them, sit on the sheet's layers, so an insert on a layer the library defines lists that layer under `layers` as well.

### Tables

A table is a grid of cells with text, AutoCAD's TABLE: an I/O list, a cable schedule, a bill of materials. It hangs from its top left corner, its rows running down and its columns across, as AutoCAD's TABLE draws one. A table belongs on a sheet, not in a block.

```yaml
- table:
    id: cables
    position: [20, 277, 0]
    columns: [30, 40, 40, 25]
    rows: [8, 6, 6, 6, 6]
    title: true
    header: true
    table_style: schedule
    cells:
      - [{text: CABLE SCHEDULE, merge: [1, 4]}, '', '', '']
      - [CABLE, FROM, TO, CORES]
      - [C-101, JB-101, MCC-1, {text: '4', justify: middle_center}]
      - [C-102, JB-102 STRIP B, MCC-1, {text: '2', justify: middle_center}]
      - [{text: C-103, style: Standard}, {text: JB-103, height: 2}, {text: SPARE, merge: [1, 2]}, '']
```

| Key | Required | What it is |
|---|---|---|
| `position` | yes | The table's top left corner: a point. |
| `columns` | yes | The width of each column, left to right: at least one length more than 0. |
| `rows` | yes | The height of each row, top to bottom: at least one length more than 0. A row is as high as it is given; text that needs more is reported by [the checker](check.md), as AutoCAD would make the row taller. |
| `cells` | yes | The cells, row by row from the top: one row of cells for each of `rows`, each holding one cell for each of `columns`. |
| `title` | no | Whether the first row is a title row, drawn as the table style's `title` rows are: `true` or `false`. Without it, `false`. |
| `header` | no | Whether the row after the title, or the first row when there is no title, is a header row. Without it, `false`. The other rows are data rows. |
| `table_style` | no | The [table style](#table-styles): `Standard` or a name under `table_styles`, written exactly as it is there. Without it, `Standard`. |
| `layer` | no | The layer the table sits on, as for every object. |

A cell is its text alone, such as `C-101` or `'4'`, `''` for an empty cell, or a mapping:

| Key | What it is |
|---|---|
| `text` | What the cell says, exactly as it shows: a line break starts a new line, and line breaks at the end are dropped. Empty, or something that shows; no control characters but line breaks. A `\`, `{` or `}` shows as itself. Without it, the cell is empty. |
| `style` | The text style its text draws with, instead of its row's. |
| `height` | The height of its text's capitals, instead of its row's: a length more than 0. |
| `justify` | Where its text sits in it, instead of where its row's does: one of the nine justifications of `mtext`. |
| `merge` | How many rows and columns the cell spans, `[rows, columns]`, itself among them: two whole numbers from 1, not both 1. The cells it spans are written `''`; a merge keeps within the table and off other merges. |

A cell's text is laid out as AutoCAD lays it out: as multiline text as wide as its cell less both side margins, so it wraps between words, at the cell's left margin, its middle or its right margin across and its top margin, its middle or its bottom margin up, as it is justified. The borders are every row's boundary and every column's, left out inside a merged cell, in the table style's border pen. A cell's text draws in the table's own colour and its layer's lineweight, as AutoCAD 2026 wrote the texts of a table it laid out: `ByBlock` in colour and `ByLayer` in lineweight, on the table's layer. Where the style's border pen is `ByBlock`, the borders take the table's own colour, linetype and lineweight.

The DWG holds an AutoCAD TABLE, its table style and its anonymous block of the lines and text it draws, which AutoCAD shows until it next lays the table out; AutoCAD 2026 opened made-up tables without recovery, audited them clean, and laid them out again as they were written. Reading a DWG back gives the table, and its style, when they have a model in the core; a table whose cell holds a block, a field or a formula, or overrides more than its text style, height and justification, is counted as not read, as is a table with any override of its own, a row's or a column's, such as a fill. In a [template](drawing-set.md#templates), a cell's text may be a value from the row, `{row: NAME}`, written as the cell or as its `text`.

A change set adds a table with `add_table`, sets a cell with `set_cell` and adds or deletes a row or a column with `add_row`, `delete_row`, `add_column` and `delete_column`, keeping the table's merges, title and header in line ([Tables in change sets](change-sets.md#tables)).

### Leaders

A leader points at something on the sheet with an arrow and says what it is: a device on a panel layout, a part number, a note on a loop sheet. A sheet holds two kinds, as AutoCAD does: the multileader, `mleader`, AutoCAD's MLEADER, and the older leader, `leader`, AutoCAD's LEADER and QLEADER, which plant drawings hold. Both belong on a sheet, not in a block.

An arrow is placed at a point: a multileader's, an older leader's, and one `add_leader_line` adds. An operation may give it as a [snapped point](change-sets.md#snapped-points), such as an insert's insertion point, which is written as the point it names: it does not follow the object afterwards.

```yaml
- mleader:
    id: terminals
    mleader_style: callout
    leaders:
      - [[180, 80, 0], [320, 110, 0]]
      - [[260, 85, 0], [320, 110, 0]]
    content: {text: TB-1 TERMINALS}
- leader:
    id: note-3
    points: [[210, 70, 0], [230, 30, 0], [250, 15, 0]]
    annotation: {text: SEE NOTE 3, style: notes, height: 2.5}
```

A multileader takes these keys:

| Key | Required | What it is |
|---|---|---|
| `leaders` | yes | Its leader lines: at least one, each a list of at least two points from its arrow's point, through any bends, to where it meets the landing, each point apart from the one before. Every leader line ends at the same point, where the landing starts: a multileader with landings in two places waits. A leader line's first segment is at least as long as its arrow, unless its arrowhead is `none`, since AutoCAD draws the line from behind the arrowhead. |
| `content` | yes | Its text: a mapping of `text`, what it says, exactly as it shows, a line break starting a new line, as an `mtext`'s contents are written; and, instead of its style's, `style`, a text style, `height`, a length more than 0, and `attachment`, where the landing meets it, as a [multileader style](#multileader-styles) names one. Or a block: a mapping of `block` and its keys ([Block content](#block-content)). Not both. |
| `landing` | no | How long its landing is, instead of its style's: a length of 0 or more, 0 for none. |
| `gap` | no | The space between its landing's end and its text, instead of its style's `landing_gap`: a length of 0 or more. Block content takes none. |
| `arrowhead` | no | Its arrowhead, instead of its style's: `closed_filled`, `dot`, `open` or `none`. |
| `arrow_size` | no | How long its arrowhead is, instead of its style's: a length more than 0. |
| `mleader_style` | no | The [multileader style](#multileader-styles) it draws with: `Standard` or a name under `mleader_styles`, written exactly as it is there. Without it, `Standard`. |
| `layer` | no | The layer it sits on, as for every object. |

A multileader is laid out as AutoCAD 2026 laid out made-up ones in SHX and TrueType fonts, and laid them out again unchanged:

- The landing runs right from where the leader lines meet when the first leader line's last segment runs right, or straight up or down, and left when it runs left.
- The text sits its gap, its own or its style's, beyond the landing's end: attached at its top left corner on the right, and at its top right corner on the left. It is multiline text and wraps nowhere but where its text breaks.
- Up, the landing meets the text where its attachment says: at the top of its box, at the middle of its top line, at the middle of the text, at the middle of its bottom line, or at the bottom of its box. The box is the multiline text's own. A line here reaches from the text's height above its baseline down to the lowest point the text's last line draws: AutoCAD measures the top line and the bottom line alike by the last line's descent, so `GLAND PLATE` over `gy` sits lower than over `GY`.
- Each leader line starts an arrow's length behind its arrow's point, where the arrowhead ends, as AutoCAD draws it: a closed filled arrowhead is a filled triangle, a sixth of its length wide each side of the line; a dot is a filled dot as wide as the arrow is long, with the line's last half arrow behind it; an open arrowhead is the triangle's sides and its middle. With `none`, the line runs to the arrow's point.
- The leader lines, the landing and the arrowheads draw in the style's leader pen, which `ByBlock` makes the multileader's own colour and lineweight; the text draws in the multileader's own colour and lineweight. All take its linetype.

#### Block content

```yaml
- mleader:
    id: ft-101-tag
    leaders:
      - [[150, 120, 0], [175, 140, 0]]
    content: {block: TAG_BALLOON, attributes: {TAG: FT-101}, connection: extents}
```

A multileader's content may be a block of the sheet's block library instead of text, as AutoCAD's block content is: a tag balloon or a hexagon at the end of a leader. Its keys:

| Key | Required | What it is |
|---|---|---|
| `block` | yes | The name of a block in the block library, written exactly as it is there. |
| `attributes` | no | The values of the block's attributes, written as an [insert's](#inserts) are: a tag the block defines more than once takes a list, a tag in lowercase is taken in capitals, and an attribute left out takes its definition's default. A value holds no `%%`: in block content AutoCAD 2026 draws `%%d`, `%%c` and `%%p` as symbols however they are escaped, so such a value, or a default that holds one, is refused. In a template, a value may come from the row, `{row: NAME}`, and a row's value that puts `%%` there is refused at the row's value. |
| `scale` | no | How much the block is enlarged, the same along every axis: a number more than 0. Without it, 1. |
| `rotation` | no | An angle: degrees anticlockwise from the X axis. Without it, 0. |
| `connection` | no | How the block meets the landing, AutoCAD's block connection type: `extents`, by the centre of the block's extents, AutoCAD's default; or `insertion`, by its base point. Without it, `extents`. |

AutoCAD 2026 made its own block content on both sides of a landing, with both connections, scaled and turned, and placed every block by one rule, which the pipeline follows:

- The block's attachment point is the centre of the box round what it draws, for `extents`, or its base point, for `insertion`, placed with the content's scale and rotation.
- The block sits with that point level with the landing, slid along the landing's line until the first place where the line crosses the block's outline, coming from the landing, is at the landing's end. When the line crosses nothing, the attachment point itself is at the landing's end. A circle's edge, not its centre, meets the landing; so does a box's near side. The landing's gap is not used.
- The outline is what the block draws, and what the blocks it inserts draw: lines, polylines along their middle, whatever their width, arcs, circles, ellipses, 2D solids and donuts. A line that runs along the landing's line does not cross it. A hatch counts towards the extents but is not crossed, and an attribute counts towards neither.

The block draws as an insert of it draws ([Inserts](#inserts)): its objects on layer `0` take the multileader's layer, its `ByBlock` objects the multileader's own colour, linetype and lineweight, and each visible attribute draws its value. The checker counts its lines and its attributes as it counts an insert's, so its own lines never count against its own attributes.

A block of text, multiline text or a constant attribute is refused as content, and so is one that inserts a block with attributes: where AutoCAD's landing meets letters depends on the font's outline. So is a block with a spline, whose extents AutoCAD takes past its curve. Block content needs a landing: a multileader with block content and a landing of 0, its own or its style's, is refused, since AutoCAD connected one to the block's own point instead, and it takes no `gap`.

The DWG holds the multileader's block content as AutoCAD 2026 wrote its own: the block, where its base point sits, its scale, rotation and connection, and a value for each of the block's attribute definitions, by the definition and its place among them, as it is written: AutoCAD escapes no `%%` there, not with `%%%` as in an insert's attribute nor with `\U+0025`, which is why a value with `%%` is refused. A multileader whose block content's value holds `%%` is not read. AutoCAD opened made-up multileaders with block content without recovery, its audit found no errors, and laying each out again moved no block.

An older leader takes these keys:

| Key | Required | What it is |
|---|---|---|
| `points` | yes | Its points: at least two, its arrow's point first, each apart from the one before. Its first segment is at least as long as its arrow, unless its arrowhead is `none`. |
| `arrowhead` | no | Its arrowhead: `closed_filled`, `dot`, `open` or `none`. Without it, `closed_filled`. |
| `arrow_size` | no | How long its arrowhead, and its hook, are: a length more than 0. Without it, 2.5 mm on a metric sheet and 0.18 in on an imperial one, as AutoCAD's ISO-25 and Standard dimension styles have them. |
| `gap` | no | The space between its end and its annotation: a length of 0 or more. Without it, 0.625 mm on a metric sheet and 0.09 in on an imperial one. |
| `annotation` | no | Its annotation, multiline text: a mapping of `text`, as a multileader's content's is written, `style`, a text style, without it `Standard`, and `height`, a length more than 0, without it 2.5 mm or 0.18 in. Without it, the leader has none. |
| `layer` | no | The layer it sits on, as for every object. |

Its annotation sits its gap beyond the leader's end, attached at the middle of its left side, or of its right side when the last segment runs left. With an annotation and a last segment more than 15° from the horizontal, AutoCAD ends the leader in a hook, a horizontal segment as long as its arrow towards the annotation, which it adds as it draws; the pipeline draws it so too. The leader draws in its own pen.

The DWG holds an AutoCAD MULTILEADER and its MLEADERSTYLE, and an AutoCAD LEADER with its annotation an MTEXT of its own that the leader points at, as AutoCAD writes them. A LEADER draws with a dimension style: it names `Standard`, which every drawing has, and carries its arrow's size, its gap, a scale of 1, its annotation level with its end and, but for a closed filled arrowhead, its arrowhead as AutoCAD's own overrides, so it draws the same whatever `Standard` holds. An arrowhead other than closed filled is a block AutoCAD names, such as `_Dot`, which the DWG holds once with what AutoCAD 2026 put in it. A block of the sheet's block library with that name, in any case, is that block when it is drawn as AutoCAD's, and is written once for both; drawn otherwise, it stops the build with an error naming it, since a drawing holds one block of a name. AutoCAD 2026 opened the example sheet without recovery, its audit found no errors, and laying every leader out again changed none.

Reading a DWG back gives the multileaders, their styles and the older leaders, each with its annotation and not as an `mtext` of its own, when they have a model in the core. A multileader whose block content the model does not place, or that does not sit where the model places it, or that is not upright, scaled the same along every axis and in the multileader's own colour, whose leader lines are splines or have breaks, with more than one landing, its own leader pen, any override of its style the model does not hold, such as its text's colour, or a landing on the other side from the one its leader line runs to, is counted as not read; so is an older leader with a spline path, an annotation that is not multiline text where the model puts it or that draws with a layer, colour, linetype, lineweight or visibility other than the leader's, its own pen from its dimension style, or text above its end, and its annotation is counted with it. An arrowhead block that only leaders draw with is read with them and not counted; one an insert places is read as a block like any other. In a [template](drawing-set.md#templates), a multileader's content's `text`, an attribute value of its block content, and an older leader's annotation's `text` may be a value from the row, `{row: NAME}`.

A change set adds a multileader with `add_mleader` (AutoCAD's `MLEADER`) and an older leader with `add_leader` (`LEADER`), adds and deletes a multileader's leader lines with `add_leader_line` and `delete_leader_line` (`MLEADEREDIT`), and `move` moves every point of either; `change` sets a template leader's text from the row in that sheet's row ([Leaders in change sets](change-sets.md#leaders)). A query for text finds a leader by its text ([Queries](query.md)). The checker counts a leader's text as text and its lines, landing and arrowheads as lines, never against each other ([Checks](check.md)).

### Dimensions

A dimension measures a length or an angle on the sheet and shows it, drawn as AutoCAD 2026 draws its own: a panel's cut-outs, a back panel's drilling, a bracket's angle, a hole pattern's ordinates. It belongs on a sheet, not in a block. Its `kind` says what it measures:

| `kind` | AutoCAD's | Keys | What it measures |
|---|---|---|---|
| `linear` | DIMLINEAR | `points`, `location`; `angle`, `continued` | The distance between its two points along its angle, in degrees anticlockwise from the X axis: 0, horizontal, without it; 90, vertical. |
| `aligned` | DIMALIGNED | `points`, `location`; `continued` | The distance between its two points. |
| `radius` | DIMRADIUS | `center`, `point`, `location` | The radius of the circle or arc round `center` through `point`. |
| `diameter` | DIMDIAMETER | `center`, `point`, `location` | Its diameter. |
| `angular` | DIMANGULAR, of two lines | `lines`, `location`; `continued` | The angle between two lines, each by two points, `[[a, b], [c, d]]`, the first selected and then the second: of the four sectors their rays make where they meet, the one `location` is in. The lines need not reach each other. |
| `three_point_angular` | DIMANGULAR, of an arc, a circle or three points | `center`, `points`, `location`; `continued` | The angle at `center`, the vertex, between the rays through its two `points`: of the two, the one `location` is in, which may be more than 180°. An arc's or a circle's is measured from its centre. |
| `ordinate` | DIMORDINATE | `feature`, `axis`, `location` | The X, `axis: x`, or the Y, `axis: y`, of its `feature`, from the origin: the world's, which is AutoCAD's UCS origin in the drawings AgentDrafter writes. The value shows without its sign. |

Every dimension also takes `text`, its own text, where `<>` stands for the measured value, as in AutoCAD: `4X <>` shows `4X ∅8`. It is one line that shows something, with no `\`, `{`, `}` or `%%`; without it, the dimension shows the measured value. It takes `dimension_style`, the [dimension style](#dimension-styles) it draws with: its unit's own or a name under `dimension_styles`, and without it, its unit's own. A linear, aligned or angular dimension takes `continued: true` when it continues another, as AutoCAD's DIMCONTINUE adds it: AutoCAD marks it so, by overriding the style's DIMTMOVE to 1, which moves nothing while its text is in its own place, and it draws as any other. A baseline or continued dimension is otherwise an ordinary dimension of its kind, which the operations `add_baseline_dimension` and `add_continued_dimension` add from another ([Dimensions from others](change-sets.md#dimensions-from-others)). Its points and location lie at one height.

```yaml
- dimension: {id: width, kind: linear, points: [[60, 50, 0], [360, 50, 0]], location: [210, 30, 0], layer: dims}
- dimension: {id: chamfer, kind: aligned, points: [[360, 230, 0], [340, 250, 0]], location: [360, 250, 0]}
- dimension: {id: slot-end, kind: radius, center: [150, 80, 0], point: [156, 80, 0], location: [168, 64, 0]}
- dimension: {id: mounting-holes, kind: diameter, center: [75, 235, 0], point: [79, 235, 0], location: [95, 270, 0], text: 4X <>}
- dimension: {id: arm-angle, kind: angular, lines: [[[20, 20, 0], [120, 20, 0]], [[20, 20, 0], [90, 90, 0]]], location: [70, 35, 0]}
- dimension: {id: vent, kind: three_point_angular, center: [200, 60, 0], points: [[230, 60, 0], [200, 90, 0]], location: [215, 70, 0]}
- dimension: {id: hole-x, kind: ordinate, feature: [130, 60, 0], axis: x, location: [140, 90, 0]}
```

`location` is where the dimension line goes, as AutoCAD's commands ask for it, and a point of it may be a [snapped point](change-sets.md#snapped-points) in an operation, such as a polyline's endpoint or a circle's centre and a quadrant on it:

- A linear or aligned dimension's `points` are where its extension lines start, the first and then the second, apart along its line; its `location` is a point its dimension line passes through, to one side of both. The reader keeps it where AutoCAD does, where the dimension line meets the second extension line.
- A radius or diameter dimension's `location` is where its text goes, inside or outside the circle: its line runs from the centre through it, and its arrow points where that line meets the circle. The reader keeps `point` there, as AutoCAD's DIMRADIUS does; its distance from `center` is the radius. Text too near the circle is moved clear of it, as AutoCAD moves it, and a DWG keeps only where it was moved to, so the reader gives back a `location` that draws the same: where the text went, or, when that lies past the centre, the point halfway from the centre to the arrow.
- An angular dimension's `location` is a point its arc passes through, in the angle it measures: the arc is centred on the vertex, its radius the location's distance from it. The reader keeps it a third of the way round the arc from its first end, as AutoCAD keeps it; when the text is outside and nearer the first ray than the second, AutoCAD keeps it two thirds of the way round, and the DWG writes it there and reads it back to its third.
- An ordinate dimension's `location` is the end of its leader, where its text goes, apart from its feature.

A dimension is laid out as AutoCAD 2026 laid out its own of made-up geometry in its `acad` and `acadiso` templates, measured across lengths, angles and style values, and laid out again unchanged:

- An extension line starts its style's offset from its point and reaches its extension past the dimension line. The dimension line runs between them with an arrowhead at each end, the line starting an arrowhead's length behind each tip, as a leader's does.
- Text and arrowheads sit between the extension lines when they fit, AutoCAD's best fit: the text needs its length along the line and a gap at each end, the arrowheads their two lengths and a gap each. Text above its line fits apart from the arrowheads; centred text, which breaks the line, needs room for both. When both fit, both are inside; when each fits alone but not both, the one that needs more room is inside, the text when they need the same; when only the text fits, the arrowheads go outside, each with a tail as long as itself; when only the arrowheads fit, the text goes past the second extension line, an arrowhead's length and a gap beyond it; when neither fits, both go outside, the text past the second arrowhead's tail. No arrowheads always fit.
- Text above its line is turned to read from the bottom or the right, a gap above the line. Centred text is horizontal; outside the extension lines it goes the way the line reads, and ends in a landing as long as an arrowhead when the line runs 15° or more from the horizontal.
- A radius or diameter dimension's text outside the circle is no nearer than two arrowheads' lengths past it, and inside no nearer than two short of it: text placed nearer is moved, as AutoCAD moves it, past the centre to the far side when the circle is too small to hold it inside. Its leader runs from the text to the arrow, which points in from outside the circle or out from inside it. Above its line, the line runs on to the centre, or across the circle with an arrow at its far side, when the text is outside. Centred, the text lands as centred text does, and a centre mark crosses the circle's centre when the text is outside it.
- An angular dimension's arc runs round its vertex, anticlockwise from the first ray of the angle it measures to the second, with an arrowhead at each end, its point on the ray and its body on the arc. An extension line runs along each ray from the line or the point there, any of a line behind the vertex where it meets the other taken as at the vertex, from its end nearer the arc, the style's offset towards it, to the style's extension past it; there is none where the arc meets the line itself. Text and arrowheads fit by the angles they need on the arc, a length `c` taking the angle of a chord `c` long, or half a turn when the circle is not that wide across: text above the arc its length and a gap at each end; centred text its box, a gap round it, across the arc's tangent at its middle, and an arrowhead's length, or three when the arrowheads are inside too; each arrowhead its length. Text inside sits at the middle of the arc, above it along its tangent and turned to read from the bottom or the right, or centred on it, which breaks the arc wherever its box and a gap round it cover it, as often as the arc goes in and out of the box, as centred text outside does where it lies across the arc. Text outside goes beyond the first ray, clockwise: the arc runs on past the arrowhead, and its tail, and the text sits above a line along the arc's tangent as long as itself and its gaps, read along it and turned round when the first ray points into the lower half, or, centred, beside a landing an arrowhead long, to the right when the first ray points into the upper half and the left when the lower. Arrowheads outside have tails as long as themselves, and text above its arc keeps the arc between the rays, as a linear dimension's line.
- Not matched: an angular dimension over 180° on an arc too small for its centred text, the text wider than the arc's whole circle or outside it, as a long text on a small arc. Of 489 such made-up angles measured, AutoCAD laid out 80 otherwise, its arrowheads outside or its landing the other way, and no rule found fitted them; such a dimension draws as these rules lay it out, and AutoCAD lays it out its own way when it is edited. The other 1,403 dimensions measured with it AutoCAD laid out again unchanged.
- An ordinate dimension's leader runs from its style's offset past the feature, across the axis it measures, to its location, backwards when the location is level with the feature across the axis, as AutoCAD runs it: straight when the two are in line; otherwise kinked, from the feature to four arrowheads short of the location, or two past the feature when that is nearer, then across to two short of the location, in line with it, and on. Its text reads along the leader, up for an X and across for a Y, whichever way it runs: above its line, a gap above the leader past its end, which runs on under it; centred, in line with the leader past its end, a gap away.
- A length shows in the style's unit format, an angle in its angular unit, each rounded half away from zero as AutoCAD rounds it, its zeros suppressed as the style says ([Unit formats](#unit-formats)); a radius's starts with R and a diameter's with ∅, unless the style gives a prefix or a suffix, which an angle takes too.
- The lines and arrowheads draw in the dimension's own colour and lineweight and its layer's linetype, and the text in its own colour and its layer's lineweight, as AutoCAD's block of a dimension draws.

The DWG holds AutoCAD's DIMENSION with its own block, `*D1` and on, holding what it draws as AutoCAD writes it, an angular dimension's arcs as ARCs and a stacked fraction in its MTEXT as AutoCAD codes it, so AutoCAD shows it without laying it out again; the build adds AutoCAD's `Defpoints` layer for its definition points. A dimension reads back from a DWG when the model draws it: one of the seven kinds, flat, with no override of its style but AutoCAD's own, DIMUPT on a radius or diameter and DIMTMOVE on a continued one, its text where AutoCAD keeps it, an angular one's arc point where AutoCAD keeps it and an ordinate one's origin the world's. One that does otherwise, such as arc length and jogged radius dimensions, text moved off its line or rotated, oblique extension lines, or an ordinate measured from another origin, is counted as not read. Its block and `Defpoints` are read with it, never as a block or a layer of the sheet's.

#### Unit formats

A style shows lengths and angles as AutoCAD 2026 showed made-up ones in every unit format of its `acad` and `acadiso` templates' styles:

| Format | `12.5` shows | Its rules |
|---|---|---|
| `scientific` | `1.2500E+01` | One digit, the precision's decimals and a signed exponent of two digits, after a period. |
| `decimal` | `12.5000` | The precision's decimals after the style's decimal separator. |
| `engineering` | `1'-0.50"` | Feet and decimal inches, twelve to the foot, after a period. Zero feet are left out when `suppress_zero_feet` says, and the inches are then always shown: `0.50"`; zero inches when `suppress_zero_inches` says, when the feet are shown: `1'`. |
| `architectural` | `1'-0½"` | Feet and inches with a fraction of them, as fine as 1/2^precision and reduced, zeros left out as for engineering; a fraction with no whole inches before it and no feet shows alone, `½"`. |
| `fractional` | `12½` | A whole number and a fraction, with no feet and no inch mark. |
| `windows_desktop` | `12.50` | Decimal, with a period whatever the separator, and the thousands grouped with commas, `2,380.44`, as Windows writes a number in US English, the language of the laptop AutoCAD was measured on. |

A stacked fraction draws as AutoCAD lays out its MTEXT: over a bar as long as the wider number, the numerator `h/6` above the bar and the denominator `7h/6` below, each in its middle; or across a slash `2h` long at 55°, the numerator on the text's middle and the denominator `h` below; both numbers as high as the rest, the text beside them on the box's bottom, or in its middle on an imperial sheet, where AutoCAD aligns Standard's text at its middle.

An angle in degrees, gradians or radians takes the angular precision's decimals after the decimal separator; in degrees, minutes and seconds, its seconds' decimals follow a period. An angle rounded to a whole turn shows as 0, in degrees and in degrees, minutes and seconds, as AutoCAD shows it.

AutoCAD allows engineering and architectural on a metric drawing, counting its millimetre as an inch, so that 25.4 mm shows as `2'-1.40"`; the reader refuses them on a sheet not in `in`, where they would show a length as so many inches that is not.

### IDs and names

```yaml
units: mm
objects:
  - polyline: {points: [[0, 0, 0], [100, 0, 0], [100, 60, 0], [0, 60, 0]], closed: true, id: frame, name: outline}
  - circle: {center: [50, 30, 0], radius: 10}
```

Every object has an ID, fixed for its life, and agents select objects by ID or by name, never by coordinates. Zach decided on 2026-11-11 that an object's ID lives in its record: the sheet description, or the template a sheet of a drawing set is built from, is the one truth for it.

- **`id`**, which every object may take, is the object's ID in its file: 1 to 255 ASCII letters, digits, `_`, `-`, `.` and `:`, such as `frame` or `loop:FT-101.3`. A generator can write its own. Each `id` is given once in a file, and two that differ only in case are one ID: the second is the `duplicate_id` error, which names the line and place of the first.
- **An object without an `id`** takes its place in the file, such as `objects[1]`. It holds while the file is unchanged, and moves when an object is added or taken away before it; an object that must keep its ID through such an edit carries an `id`. A build never writes into a sheet description.
- **Operations write an `id` in as they edit** ([Change sets](change-sets.md#ids-stay-put)), so that an ID stays fixed for the object's life once its records are edited: every object an operation adds is given an `id`, and deleting an object writes the place of each later object that has no `id` as its `id`, such as `id: objects[2]`. A place may be written as an `id` for that reason, whole, as `objects[3]` or `rows[1].objects[0]`, and the object keeps that ID wherever it moves; an object without an `id` whose place is another object's written `id` is the `duplicate_id` error.
- **The object's ID** is its sheet's name, `/`, and that, such as `frame-plate/frame` or `frame-plate/objects[1]`. A sheet description built on its own is named by its file, without the extension (`frame-plate.yaml` above); a sheet of a drawing set by its row's `sheet`. A template's objects are on every sheet built from it, and each sheet's name is given once in a set, so `DEMO-001/border` and `DEMO-002/border` are two objects and every ID is unique in the set.
- **The DWG carries it.** A build writes every object's ID into the DWG with the object, as extended data under the registered application `AGENTDRAFTER` ([The block library](block-library.md#how-it-is-written)): the text `sheet` and the sheet's name, then `id` and the object's `id` or place, then, for an object with a name, `name` and its name. A DWG's extended data holds strings of at most 255 characters, so the two parts are kept apart. AutoCAD keeps extended data through a save, so a revision can find the object again. Building the same set twice gives every object the same ID.
- **`name`**, which every object may take as well, is a name an agent selects the object by, such as `outline` or `jb`: 1 to 255 characters, not starting or ending with a space, with no control characters. Each name is given once on a sheet, whatever objects carry it, and names that differ only in case are one name: the second is the `duplicate_name` error. A name is not an ID, which a sheet's objects all have and which is unique in the set, and it is never a tag or an attribute tag.
- **`copy_of`**, which `copy` and `array` write on each copy, names the object it is a copy of by its ID in the file: the first object of the copies, its source or its source's own `copy_of`. It changes nothing drawn and is not written into the DWG. The engine writes and keeps it: a change set's add and `change` do not take it. A value the object takes from the row it shares with its copies until one of them is set, when that one takes its own ([Change sets](change-sets.md#copy-and-array)). A plain number, `true` or `false` is read as its text; it is not empty.
- **A block's objects take no `id` and no `name`**: an insert is the object on the sheet, and its block's objects are shared by every insert of it.

[Queries](query.md) find objects by ID or by name.

### Paper

```yaml
units: in
paper: ANSI B landscape
objects:
  - polyline: {points: [[0, 0, 0], [17, 0, 0], [17, 11, 0], [0, 11, 0]], closed: true}
```

Zach decided on 2026-11-11 that a sheet's paper is declared. `paper` says what the sheet is drawn on, and [the checker](check.md) holds the sheet to it: its extents are the paper's size, and nothing draws off it. A build is the same with it or without it, and the DWG is unchanged by it. Zach decided on 2026-11-13 that a [plot](plot.md#the-sheet-size) and a [render](render.md) are drawn on it: the page is the paper, what lies off it cut at its edge. A sheet without it is checked all the same, and told that it declares no paper, and is plotted and rendered to its extents.

- **Its size**: two lengths more than 0, `[width, height]`, such as `[17, 11]`, in the declared unit, or each with its own, such as `[431.8 mm, 279.4 mm]`.
- **Or a standard size by name, with the way it lies**: `landscape` lays its longer side across, along X, and `portrait` up. The names are `ANSI A`, `ANSI B`, `ANSI C`, `ANSI D` and `ANSI E` (8.5 by 11, 11 by 17, 17 by 22, 22 by 34 and 34 by 44 inches), and `ISO A0`, `ISO A1`, `ISO A2`, `ISO A3` and `ISO A4` (841 by 1189, 594 by 841, 420 by 594, 297 by 420 and 210 by 297 mm), each followed by `landscape` or `portrait` and written exactly so: `ANSI B landscape` is 17 by 11 inches, converted to the declared unit. A name without the way it lies is the `bad_value` error, which names both.
- **Its lower left corner is at the origin**, `[0, 0]`: the sheet is drawn in model space at its paper's size from there, as a template draws its trim line, or, on a sheet drawn in a layout, in paper space.

A template declares its paper as a sheet description does; the set's settings take none, since the template draws the border.

### Layouts and viewports

```yaml
units: mm
paper: ISO A3 landscape
layout:
  name: Backplane
objects:
  # Model space, at full size: a backplane 600 wide and 800 tall.
  - polyline: {id: backplane, points: [[0, 0, 0], [600, 0, 0], [600, 800, 0], [0, 800, 0]], closed: true}
  - text: {id: tag, contents: PLC-101, position: [160, 660, 0], height: 10, justify: middle_center}
  # Paper space, at the paper's size: the border and the title.
  - polyline: {id: frame, space: paper, points: [[10, 10, 0], [410, 10, 0], [410, 287, 0], [10, 287, 0]], closed: true}
  - text: {id: title, space: paper, contents: CONTROL PANEL CP-101 BACKPLANE, position: [255, 37, 0], height: 4}
  # The whole backplane at 1:4, and a detail at 2:1, locked.
  - viewport: {id: view-a, center: [110, 148, 0], width: 180, height: 250, view_center: [300, 380, 0], scale: '1:4'}
  - viewport: {id: view-c, center: [320, 90, 0], width: 60, height: 50, view_center: [81, 462, 0], scale: '2:1', locked: true}
```

A sheet may be drawn in a paper-space layout, as plants draw panel layouts and network one-lines: the drawing at full size in model space, and the sheet's border and title block in paper space, at the paper's size, with viewports that show model space at a scale. `layout` makes a sheet one, and gives the layout its name, as AutoCAD's tab shows it, a plain number read as its text as every name is: 1 to 255 characters, none of `<>/\":;?*|,=` and the backquote, and not `Model`, the model-space tab's. `examples/layout.yaml` is one.

- **Its paper is a standard size by name**, such as `ISO A3 landscape`, which the layout's page setup plots whole, and its unit is `mm` or `in`, as AutoCAD's paper is. A layout without a paper, or on a paper given by its size, is the `bad_value` or `missing_key` error. Paper space's lower left corner is at the paper's, `[0, 0]`.
- **Each object is in model space** unless it says `space: paper`, or the layout's name, or `space: model`; a sheet without a layout has model space alone, and an object that says `space: paper` on one is the `bad_value` error. `space` is a choice, as `justify` is, so a bare null there is the `bad_value` error too. Both spaces are in the declared unit.
- **A viewport** is in paper space: a window `width` by `height` on the paper round its `center`, that shows model space round `view_center`, the point of model space at its middle, AutoCAD's view centre, at its `scale`. A scale is paper to model: a ratio of two numbers more than 0, `'1:4'` or `'2:1'`, or a number more than 0, `0.25`. Quote a ratio: the reader takes `1:4` unquoted too, but a YAML reader that guesses types may read it as a number. The scale, one over it, and the viewport's height and width over it, the model height and width it shows, are each a finite number more than 0 that a DWG holds whole, and the part of model space it shows reaches no further than a DWG's numbers do; a scale that breaks this, such as `1e-309`, is the `bad_number` error. `locked: true`, or `'true'`, locks its view, as AutoCAD's display lock does: it draws the same, and [`change_viewport`](change-sets.md#viewports) will not change its scale or view centre unless it unlocks it too. A viewport looks straight down, so its points have 0 for `z`. A rectangular viewport frames what it shows with its own pen; put it on a layer that does not plot to leave its frame off a PDF. A viewport may instead be clipped to a boundary, and may freeze layers in it alone; see [Clipped viewports](#clipped-viewports) and [Layers frozen in one viewport](#layers-frozen-in-one-viewport).

**How it draws.** A [render](render.md), a [plot](plot.md), the viewer and [the checker](check.md) draw a sheet drawn in a layout as it prints, on its paper: paper space first, as the page setup plots it, then each viewport in turn, model space seen through it and its frame. Model space shows through a viewport moved and scaled from its view centre to its centre, by its scale, and cut at its edge: a line whose centre runs outside draws nothing there, and the ink of what shows, a pen's width and its round ends among it, stops at the edge, in the PNG, the PDF, by a clipping path, and the viewer alike, as AutoCAD 2026 plotted a made-up probe of lines 2.11 mm wide; a line that lies along its edge is left out, as AutoCAD left it out of its plot. Text, hatch patterns and geometry scale with it. A lineweight is a width on paper, so it does not; and a linetype's dashes are as long on the paper as in paper space, as AutoCAD's PSLTSCALE 1, every drawing's own, draws them. A rectangular viewport's frame plots as AutoCAD's does: its thinnest line, in its colour, whatever its lineweight. A model object that no viewport shows draws nothing. These were measured in AutoCAD 2026's plots of made-up layouts at 1:48 to 2:1, in millimetres and inches, through acad.ctb: at 300 dots per inch every pixel of AutoCAD's ink lies within a pixel of AgentDrafter's, and 99.99% or more of AgentDrafter's within a pixel of AutoCAD's away from the paper's edge, where AutoCAD leaves out a line on the edge itself.

**What the DWG holds.** Every new drawing has AutoCAD's empty layout `Layout1`, which a sheet without a layout leaves as it is, so its DWG is the same byte for byte as before layouts. A sheet with one names that layout after its own, and writes, as AutoCAD 2026 wrote made-up ones:

- its page setup: AutoCAD's own `DWG To PDF.pc3`, the full-bleed paper of the sheet's size laid its way, such as `ISO_full_bleed_A3_(420.00_x_297.00_MM)`, the plot area the layout, at 1:1, or 1:25.4 when a millimetre sheet is on inch paper and 25.4:1 when an inch sheet is on millimetre paper, through `monochrome.ctb` with lineweights, paper space first;
- the layout's own overall viewport, then its paper-space objects and viewports on `*Paper_Space`, each with its ID, and model space's objects on `*Model_Space`;
- each viewport as AutoCAD's VIEWPORT seen from above: its centre and size on the paper, its view centre, and its view height, its height over its scale, with its display lock, its clipping boundary and the layers frozen in it;
- AutoCAD's TILEMODE 0, so that the drawing opens on its layout.

AutoCAD 2026 opened every such DWG without recovery, its audit found no errors, regenerating every viewport moved none, and its save reads back whole. Reading a DWG back gives the sheet it holds: its model space and every layout, in the order of their tabs, whatever tab it was saved on, its objects from every space in the order they were made, each with its space. The tab AutoCAD saved it current on, a layout or the Model tab, says only which tab it opens on, given as `opens_on`; it changes nothing that is read, drawn or plotted, so a DWG saved on its Model tab renders and plots every layout, as one saved on a layout does. A drawing of model space alone, beside AutoCAD's own empty layout, reads as model space alone, as before. A viewport reads back when the core draws it: on, seen from above with no twist, perspective or front or back clipping, rectangular or clipped to a boundary the core reads (below), and freezing layers of the drawing's own; any other is counted not read. Its scale reads back as the simplest ratio that gives its view height, so `1:4` is 0.25. A layout's page setup reads back as its paper when it is a standard paper plotted whole at full size, as a build writes it. A layout whose page setup does not read back is read without a paper and counted under `layouts_not_read`, the one the drawing opens on and any that holds anything; AutoCAD's own empty layout, on its own page setup, is passed over.

### Several layouts

```yaml
units: mm
layouts:
  - {name: Assembly, paper: ISO A3 landscape}
  - {name: Wiring, paper: ISO A4 portrait}
objects:
  # Model space, which both layouts show.
  - polyline: {id: plate, points: [[0, 0, 0], [500, 0, 0], [500, 700, 0], [0, 700, 0]], closed: true}
  # Each layout's paper space names its layout.
  - polyline: {id: a-frame, space: Assembly, points: [[10, 10, 0], [410, 10, 0], [410, 287, 0], [10, 287, 0]], closed: true}
  - viewport: {id: view-a, space: Assembly, center: [90, 150, 0], width: 140, height: 200, view_center: [250, 330, 0], scale: '1:5'}
  - polyline: {id: w-frame, space: Wiring, points: [[10, 10, 0], [200, 10, 0], [200, 287, 0], [10, 287, 0]], closed: true}
  - viewport: {id: view-w, space: Wiring, center: [105, 160, 0], width: 180, height: 250, view_center: [250, 350, 0], scale: '1:3'}
```

One DWG may hold more than one sheet, as AutoCAD's layout tabs do: a sheet description, or a template, may list two or more layouts under `layouts`, which share one model space. `examples/layouts.yaml` is one.

- **Each layout** has its `name`, as `layout`'s is, each given once whatever its case, as AutoCAD names each tab once, and its `paper`, a standard size by name; one that gives no paper takes the sheet's `paper`. A layout with neither is the `missing_key` error. The sheet is then in `mm` or `in`, as for one layout. `layout` and `layouts` are not given together, and `layouts` lists two or more.
- **Each object in paper space names its layout** under `space`, exactly as it is given; `paper` names no one of several, and is the `bad_value` error, as is a name the sheet does not have. Every viewport says which layout it is in. An object that gives no `space`, or says `model`, is in the model space every layout shows. On a sheet of one layout, its name and `paper` are one space.
- **An object's ID does not name its layout**: it is the sheet's name joined to its `id`, as on any sheet, unique on the sheet whatever space it is in, and fixed for its life, though `change` may move it to another space.
- **Each layout prints on its own**: a [render](render.md) writes a PNG for each, a [plot](plot.md) a page for each, at its paper, [the checker](check.md) checks each and names it in each finding, and [`query layout`](query.md#layout) lists them all. Each draws its own paper space and model space through its own viewports, as a sheet of one layout does.
- **The DWG** holds each as its own AutoCAD layout, in the order given: the first takes AutoCAD's `Layout1`, on `*Paper_Space`, and the drawing opens on it; each after it is added after it, on a block of its own, `*Paper_Space0` and on, with its page setup and its overall viewport as the first's, and its tab numbered in order, as AutoCAD 2026 numbered made-up layouts' tabs. AutoCAD keeps the current layout on `*Paper_Space`; a DWG opens on the layout that was current when it was saved.

### Clipped viewports

```yaml
  # The terminal strip at 1:1, through a circle.
  - viewport: {id: detail-x2, space: Assembly, clip: {center: [230, 195, 0], radius: 55}, view_center: [224, 150, 0], scale: '1:1', layer: details}
  # The relay at 1:2, through a polyline with a round end.
  - viewport: {id: detail-k1, space: Assembly, clip: {points: [[290, 80, 0], [370, 80, 0], {point: [370, 190, 0], arc_angle: 180}, [290, 190, 0]]}, view_center: [100, 550, 0], scale: '1:2', layer: details}
```

A viewport may be clipped to a boundary instead of its rectangle, as AutoCAD's VPCLIP and MVIEW's Object and Polygonal options clip one. `clip` gives the boundary on the paper: a closed polyline's `points`, at least three, each `[x, y, 0]` or a polyline's point with a `bulge` or an `arc_angle` (see [Polylines](#polylines)) and no width, or a circle's `center` and `radius`. It encloses an area; one that is a line or a point is the `bad_value` error, as is a boundary given both ways.

- **Its window is the box round the boundary**, as AutoCAD 2026's VPCLIP left a made-up viewport: so a clipped viewport takes no `center`, `width` or `height`, and any of them is the `bad_value` error, and its `view_center` is the point of model space at the middle of that box. [`clip_viewport`](change-sets.md#viewports) clips one and works its view centre out, so that model space stays where it showed.
- **Model space shows inside the boundary alone**: a line is cut where it crosses it, its arcs drawn as their chords on the paper, and the ink of what shows, its pen's width and round ends, and every fill, stops at the boundary, in the PNG, by a mask, the PDF, by a clipping path, and the viewer, by its stencil. A line that lies along one of the boundary's straight edges that runs across or up the paper is left out, as along a rectangular viewport's edge; one along a slanting edge or an arc is drawn, and so is one that lies on a curved boundary: AutoCAD 2026 plotted a made-up probe so.
- **The boundary is the viewport's frame**: it draws and plots with the viewport's own pen, its lineweight among it, as AutoCAD 2026 plotted a clipping boundary, and the viewport's rectangle does not draw. Put the viewport on a layer that does not plot to leave the boundary off a PDF.
- **The DWG** holds the boundary as AutoCAD's VPCLIP leaves it: a closed lightweight polyline, its bulges among it, or a circle, an object of the layout's paper space on the viewport's layer with its pen, written before the viewport, whose reactor is the viewport; the viewport names it as its clipping boundary, with AutoCAD's clipped status. It carries no ID of its own: it is the viewport's. A boundary reads back with its viewport when it is such a polyline or circle drawn with the viewport's pen; a viewport whose boundary is anything else, or on another layer or pen, is counted not read, and its boundary read as an object of its own.

### Layers frozen in one viewport

```yaml
  # The plate with its dimensions frozen in this viewport alone.
  - viewport: {id: view-wired, space: Wiring, center: [57.5, 160, 0], width: 85, height: 160, view_center: [250, 350, 0], scale: '1:6', frozen_layers: [dims]}
```

`frozen_layers` lists the layers frozen in one viewport, as AutoCAD's VPLAYER freezes them: their objects neither draw nor plot through that viewport, an insert's own objects on such a layer among them, and the checker does not check them through it, but they draw through every other viewport. Each is layer `0` or one under `layers`, named exactly as it is defined, a name the sheet does not define being the `unknown_name` error and one given twice the `bad_value` error; the reader keeps them in the order of the drawing's layer table, layer `0` first and then the order `layers` gives, as AutoCAD 2026 kept them when it saved them. [`set_frozen_layers`](change-sets.md#viewports) sets the list. The DWG holds them as AutoCAD's VPLAYER writes them: each layer's handle in the viewport's frozen-layer list.

### Fonts

AutoCAD's SHX fonts are Autodesk's files and are never in this repository (ADR-0008). AgentDrafter reads fonts from the font folders a setting names: the folders `--fonts` gives, then those the environment variable `AGENTDRAFTER_FONTS` lists, separated as the system separates paths (`;` on Windows, `:` elsewhere). On a machine that has AutoCAD, name its `Fonts` folder for the SHX fonts. A font is found by its file name, ignoring case.

A TrueType font in none of those folders is then looked for in the system's own font folders, as AutoCAD finds it, each with the folders inside it however deep, the folder's own files first and then its folders in the order of their names. Where a folder holds files whose names differ only in case, the one in the name's own case wins, then the first in the order of their names, so the same font wins however the system lists the folder. A folder already searched, as through a link back to one above it, is not searched again:

| System | Its font folders, in order |
|---|---|
| Windows | `%WINDIR%\Fonts`, then the user's own, `%LOCALAPPDATA%\Microsoft\Windows\Fonts` |
| macOS | `/System/Library/Fonts`, `/Library/Fonts`, `~/Library/Fonts` |
| Linux and other Unix systems | `/usr/share/fonts`, `/usr/local/share/fonts`, `~/.local/share/fonts`, `~/.fonts` |

An SHX font is never looked for there: AutoCAD's SHX fonts live only in its own folder. A font found in a system folder says so: `source` is `system` in `measure-text`'s `font`, and in a substitution whose look-alike was found there. `--no-system-fonts` on a command, `no_system_fonts: true` in its input, or `AGENTDRAFTER_SYSTEM_FONTS=0` in the environment (or `false`, `no`, `off`) leaves the system's folders out, so what is measured does not depend on the fonts a machine has installed. CI's demo plant, the speed bench and every test that holds output to its bytes leave them out.

A font that is in no font folder, or cannot be read, is replaced by a look-alike for measuring. A TrueType font with an open twin of the same widths takes the twin when a font folder holds it: Liberation Sans or Arimo for Arial (`arial.ttf`, `arialbd.ttf`), Liberation Serif or Tinos for Times New Roman (`times.ttf`, `timesbd.ttf`), Liberation Mono or Cousine for Courier New (`cour.ttf`, `courbd.ttf`). Every other font takes Hershey's Roman Simplex, which is built in and in the public domain, and whose letters and digits have the same advances as AutoCAD's `romans.shx`. The DWG still names the style's own font. A build's result lists each substitution under `font_substitutions`: the style, its font, the look-alike used and why.

A reply in which a look-alike stood in also warns of it at its top, under `warnings`, after `ok` and before `result`, so an agent reads it without looking inside each sheet's result. There is one warning per font, naming every sheet it was used on; `measure-text`, which measures no sheet, names none. Every command that measures or draws text does so, through every interface: `build`, `render`, `plot`, `display-list`, `check`, `change` for the sheets it rebuilt and checked, `accept` for the sheets it checked, and `measure-text`. A reply in which no look-alike stood in has no `warnings`.

```json
{"ok":true,"warnings":[{"code":"font_look_alike","font":"romans.shx","look_alike":"Hershey Roman Simplex","sheets":["DEMO-001","DEMO-002","DEMO-003"]}],"result":{...}}
```

AgentDrafter reads SHX fonts in the `AutoCAD-86 shapes` and `AutoCAD-86 unifont` layouts, which AutoCAD's own text fonts use, and TrueType fonts through ttf-parser (MIT or Apache-2.0). Big fonts, for Asian scripts, are not read.

A TrueType font encoded as a Windows symbol font, whose character map has a Windows symbol table (platform 3, encoding 0), holds its characters at U+F000 plus their code: `I`, 0x49, at U+F049. Its Unicode table, when it has one, may hold them there too. Some of the TrueType fonts AutoCAD draws a drawing's PostScript fonts with are such fonts. A character that such a font's Unicode tables hold no glyph for is looked up in its symbol table, at its own code and then, below U+0100, at U+F000 plus its code, as Windows and AutoCAD do, so its text measures, draws, plots and is checked in its own characters, not in missing-glyph boxes. A font with no symbol table is read as before. A PDF's searchable text holds the characters as written. Its Mac table (platform 1, encoding 0) is not read.

### Measuring text

`agentdrafter measure-text <contents> --font <file> --height <height> [--width-factor <factor>] [--fonts <folder>]... [--no-system-fonts]` measures one line of text as AutoCAD measures a text object (AutoLISP's `textbox`): `width` is from the left edge of what the text draws to the right edge, and `left`, `right`, `bottom` and `top` say where that is from the start of the line, in the unit of the height. `advance` is how far the pen moves. `--font` names a font as a text style does, by its file name alone, and is read only from the font folders, and for a TrueType font the system's. `font` says which font was used and, when it was a look-alike, why; `source` is `system` when it was found in a system folder. A space at either end draws nothing, so it adds nothing to the width.

Measured against AutoCAD 2026 with its own fonts, on the machine that has them: the widths of 139 made-up strings in `romans.shx`, `simplex.shx`, `txt.shx`, `isocp.shx`, Arial and Times New Roman, 0.1 to 7 units high, match AutoCAD's to within 1e-12 of the unit, and the box of each of the 1,151 characters of the four SHX fonts, 100 high, matches to the eight decimal places AutoCAD printed. AutoCAD does not kern.

An SHX character's box is not always the box of the strokes it draws. AutoCAD's box, which AgentDrafter follows, reaches past them in four ways:

- An arc of up to half a turn is boxed by its chord and the chord moved out to the arc's middle, which reaches past a quarter arc; a longer arc is boxed by its whole circle.
- An octant arc (shape code 10) of half a turn that passes through the point of its circle at 180°, the side to the left of its centre, between its ends is boxed by its whole circle. One that only starts or ends there is not, and neither is a bulge arc.
- An octant arc is boxed when the pen is up too, though it draws nothing.
- When the pen comes down, AutoCAD counts the point the pen last moved to as drawn, even when a pop (shape code 6) has put the pen somewhere else since.

The last three were found from the ogonek letters Ą, ą, Ę and ę in `romans.shx` and Ą in `simplex.shx`, which AutoCAD boxed up to 0.12 of the height wider or deeper than their strokes (#28). They were confirmed on 20 made-up shapes in AutoCAD 2026: half circles through 180° and not, clockwise and anticlockwise, at three scales, with the pen down and up, and pops followed by the pen coming down and not. The four fonts draw no other character these rules change. Not measured: a fractional arc (shape code 11) of half a turn, and a fractional or bulge arc with the pen up. AgentDrafter boxes those by the first rule alone, and only when the pen is down.

## Values

A **point** is a list of exactly three lengths, `[x, y, z]`. Every coordinate is 3D; on a flat sheet `z` is `0`.

A **length** is a number, with or without its own unit:

- `100` is 100 of the declared unit.
- `4 in` or `4in` is four inches, converted to the declared unit.
- A number is an optional `-`, digits, an optional `.` followed by digits, and an optional exponent such as `e3` or `E-2`. Nothing else is a number: not `+1`, `.5`, `5.`, `1_000`, `0x10`, `inf` or `nan`.
- A unit is one of `mm`, `cm`, `m`, `in`, `ft`, in lower case, after the number and at most one space.
- A length that is too large to hold once converted to the declared unit, such as `1e308 m` on a sheet in `mm`, is an error.

An **angle** is a number of degrees, counted anticlockwise, with no unit. Whole turns are taken off, so `360` is `0`, `-90` is `270` and `450` is `90`.

A **ratio** and a **scale** are numbers with no unit, and so is a **width factor**. A **degree** is a whole number, digits only. `closed` is `true` or `false`, in lower case. Text **contents** are text: `contents: 100` says 100.

A hatch pattern's `origin` and `delta` are lists of exactly two lengths, `[x, y]`, in the plane the hatch lies in. They are the one place a sheet description has two coordinates rather than three.

There is no default unit. A sheet with no `units` is an error, so that no number is ever read in a unit nobody stated.

## The YAML subset

A sheet description is YAML 1.2 with these limits. Each one is checked, and breaking it is an error.

A file may start with a UTF-8 byte-order mark. It is ignored: every rule below applies as if it were not there, and lines and columns are counted without it.

1. **One document.** A leading `---` and a trailing `...` are allowed. A second document is an error.
2. **No anchors or aliases.** `&name` and `*name` are errors. Write each value out.
3. **No tags.** `!!str`, `!!int`, `!name` and the like are errors.
4. **No directives.** A line starting with `%`, such as `%YAML 1.2` or `%TAG`, is an error.
5. **Keys are text, and each appears once.** A list or mapping used as a key is an error. The same key twice in one mapping is an error, even in flow style.
6. **No unknown keys.** A key the sheet description does not define is an error, and the error lists the keys that are allowed there.
7. **Every scalar is text; the sheet description gives it its meaning.** YAML's guessing of types is off: `yes`, `no`, `0x10` and `.inf` are not booleans or numbers. A coordinate is read as a length and nothing else, so `null` there is an error. Quoted and plain scalars mean the same: `'mm'`, `"mm"` and `mm` are one unit, and `layer: 0` and `layer: '0'` are one layer.
8. **A bare null leaves a text field out.** Zach decided on 2026-11-11 that on a field whose value is text or a path, a bare null means the field is not written. A bare null is `null`, `Null`, `NULL` or `~`, unquoted, or a value left empty, as in `name:`. An optional field written so takes its default, as if it were left out: `layer: ~` puts an object on layer `0`, and an attribute written `TAG:` takes its definition's default. A required field written so is the `missing_key` error, such as `contents: null` on a text or `block:` on an insert. Quoted, it is text: `'null'` is the word null, and `''` is empty text. The reader can tell `name:` from `name: null`, but a YAML reader that guesses types makes both null, so the schema cannot, and the two mean the same. The rule is the same for every text and path field of every kind of file: a sheet description, a template, a block library, a set's settings and its rows, and for the keys of an object a [change set](change-sets.md#the-operations) adds, where JSON's `null` is a bare null. Elsewhere `null` is read as the text it is, so `closed: null` is an error, as `closed: maybe` is.

Comments, block style and flow style (`[0, 0, 0]`, `{from: ..., to: ...}`) are all allowed.

## The schema

`agentdrafter schema` prints one line of JSON, `{"ok":true,"result":{...}}`, whose result is a JSON Schema of the sheet description; `agentdrafter schema block-library` prints the block library's in the same way, and everything below holds for both. JSON Schema describes JSON, and a YAML reader that guesses types turns `100` into a number and `4 in` into a string, so in the schema a length is a number or a string that reads as one. Quoted and plain scalars mean the same here, so the schema takes every value the reader takes in either form: `color: '42'`, `lineweight: '0.50 mm'`, `closed: 'true'` and `degree: '2'` pass both. What JSON Schema cannot see is left to the reader: such a YAML reader turns `+5` and `5` into one number, and some checks compare one value with another, such as a degree with the number of control points, or reach into another file, such as whether a block or an attribute tag is in the block library. The rules of the YAML subset are not JSON Schema either; the schema's top-level description states them. The schema is built from the same tables the reader checks keys against, and tests keep the two in step.

## Errors

`agentdrafter build` prints one line of JSON on standard output. On failure it is:

```json
{"ok":false,"error":{"code":"unknown_key","message":"unknown key `too`; a line takes from, to, layer, id","field":"objects[0].line.too","line":5,"column":7,"file":"bad.yaml"}}
```

`field` is the path to the value in the sheet description: keys joined with `.`, list items as `[index]` counted from 0, such as `layers.hidden.linetype` or `objects[2].arc.radius`. `line` and `column` count from 1. `field`, `line`, `column` and `file` appear when they are known. An error in the block library a sheet names gives the library's file and a path in the library, such as `blocks.MARKER.objects[1].attdef.tag`. An error about a choice, such as a key, a unit, a layer, a linetype, a text style, a colour, a lineweight, a justification, a block or an attribute tag, lists the valid choices in its message.

| Code | Meaning |
|---|---|
| `usage` | The command was called wrongly, or the output path is the sheet description itself, or the text of a sheet description that names a block library was read without its file. |
| `read_failed` | The sheet description, or the block library it names, could not be read. |
| `yaml_syntax` | The text is not well-formed YAML. |
| `not_in_subset` | An anchor, alias, tag, directive, second document or key that is not text. |
| `duplicate_key` | A key appears twice in one mapping, two layer, linetype, hatch pattern or block names differ only in case, or an insert gives one attribute tag twice in two cases. The message gives the line of the first. |
| `duplicate_name` | Two objects on a sheet carry the same name, or names that differ only in case. The message gives the line of the first's `name`. |
| `duplicate_id` | Two objects in one sheet description or template carry the same `id`, or IDs that differ only in case, or an object without an `id` sits at a place another object's `id` is written as. The message gives the line and place of the first. |
| `unknown_key` | A key the sheet description does not define, including an unknown object type, an attribute definition on a sheet or an insert in a block. |
| `missing_key` | A required key is missing or is a bare null, or the file is empty. |
| `missing_unit` | The sheet declares no unit. |
| `unknown_unit` | A unit that is not one of `mm`, `cm`, `m`, `in`, `ft`. |
| `wrong_type` | A value of the wrong shape, such as a point without three coordinates, a polyline with too few points, a hatch with no loops, `closed: yes`, a list for an attribute tag the block defines once, or one value for a tag it defines more than once. |
| `bad_number` | A value that should be a number is not one, carries a unit where none belongs, or is too large to hold in the declared unit. |
| `bad_value` | A value of the right shape that is not allowed: a colour outside 1 to 255, a lineweight that is not standard, a radius or text height that is not positive, a ratio outside 0 to 1, a width factor outside 0.01 to 100, an arc whose angles are equal, a linetype pattern that breaks AutoCAD's rules, a name AutoCAD would refuse, a hatch pattern line whose lines would lie on one another, a hatch scale or angle given with `SOLID`, a boundary that does not lie flat or whose edges do not join up, a text style that is `Standard` in another case, a font that is not an SHX or TrueType file name, a justification that does not exist, text contents that are empty or would not show as written, an attribute tag that holds a space, a list longer than the times its block defines a tag, a name AgentDrafter refuses, an insert's scale that is not more than 0, a block library path that is not relative or holds a character it cannot, a block library in another unit than the sheet, a layer, linetype or text style the sheet and the blocks it inserts define differently. |
| `unknown_name` | A layer, linetype, hatch pattern or text style that the sheet description does not define, a block that is not in the block library, or an attribute tag the block does not define. |
| `output_exists` | A file is already where the DWG would go, and `--overwrite` was not given. |
| `not_a_build` | `--overwrite` was given, but the file where the DWG would go is not a DWG AgentDrafter built, or cannot be read to tell. It may be an original, and an original is never overwritten. |
| `write_failed` | The DWG could not be written. |
| `dwg_write` | The DWG library could not write the drawing. |
| `dwg_read` | The DWG library could not read a drawing. |
| `missing_value` | A row of a drawing set lacks a value its template needs. See [the drawing set](drawing-set.md#errors). |
| `sheets_failed` | Some sheets of a drawing set failed and were not written; the others were built. |

## How it is read

The YAML is read by [saphyr-parser](https://crates.io/crates/saphyr-parser) (MIT or Apache-2.0), a YAML 1.2 parser that reports each node with its anchor, tag and position. AgentDrafter builds its own tree from those events, refusing anything outside the subset as it goes, so a duplicate key or an anchor is caught where it is written rather than lost in a parsed value. The code is `crates/agentdrafter-core/src/description/`: `mod.rs` reads the top level and the values, `layers.rs` the linetypes and layers, `objects.rs` the objects, `hatch.rs` the hatch patterns and hatches, `text.rs` the text styles and text, `blocks.rs` the block library, its blocks and inserts of them, `ids.rs` each object's `id`, `records.rs` a sheet with its objects' IDs, and `schema.rs` writes both schemas, with `hatch_schema.rs` and `block_schema.rs` writing their hatch and block parts. Fonts and measuring are in `crates/agentdrafter-core/src/text/`, text's side of the DWG in `src/dwg/text.rs`, and blocks' in `src/dwg/block.rs`.
