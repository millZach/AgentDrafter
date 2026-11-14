# The sheet description

A sheet description is the text file that states what is on a new sheet. It is the sheet's record: the DWG is built from it (ADR-0003). It is written in a strict subset of YAML, set out below, so that an agent can write it, a person can read it, and a mistake is an error that names its field rather than a guess.

So far a sheet description holds lines, polylines, arcs, circles, ellipses, splines and hatches, on layers that carry a colour, a linetype and a lineweight, text and multiline text in text styles that name a TrueType or an SHX font, inserts of blocks with attributes from a [block library](block-library.md), and tables in table styles. A template is written with the same keys, and leaves some values to a row of a [drawing set](drawing-set.md), which builds one sheet per row.

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

`agentdrafter build examples/one-line.yaml` writes `examples/one-line.dwg` in the 2018 format and prints what it wrote as JSON. `examples/geometry.yaml` holds every kind of line and curve, on layers with linetypes, `examples/hatch.yaml` holds solid and patterned hatches, and `examples/text.yaml` every kind of text, in every justification. `examples/text-widths.yaml` holds the fixed strings AgentDrafter's measured widths are compared with AutoCAD's on. `examples/blocks/` holds a block library and two sheets that insert its blocks, and `examples/table.yaml` two tables.

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
| `linetypes` | no | The sheet's linetypes: a mapping from each linetype's name to its definition. |
| `layers` | no | The sheet's layers: a mapping from each layer's name to its colour, linetype and lineweight. |
| `hatch_patterns` | no | The sheet's hatch patterns: a mapping from each pattern's name to its lines. |
| `text_styles` | no | The sheet's text styles: a mapping from each style's name to its font and width factor. |
| `table_styles` | no | The sheet's table styles: a mapping from each style's name to what its tables' cells draw with. See [Table styles](#table-styles). |
| `objects` | yes | A list of objects, which may be empty, in drawing order. Each object is a mapping with one key, its type. |
| `paper` | no | The paper the sheet is drawn on, which the checker holds it to. See [Paper](#paper). |

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

### Objects

Every object takes a `layer` key: the name of the layer it sits on, `0` or one under `layers`. Without it, the object sits on layer `0`. An object draws with its layer's colour, linetype and lineweight unless it gives its own, under `color`, `linetype`, `lineweight` and `linetype_scale`; see [An object's own colour, linetype and lineweight](#an-objects-own-colour-linetype-and-lineweight). Every object may also take an `id` key, its ID in the record, and a `name` key, a name an agent selects it by; see [IDs and names](#ids-and-names).

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
| `text` | `contents`, `position`, `height`; `style`, `width_factor`, `rotation`, `justify` | A single line of text, as AutoCAD's TEXT draws it. See [Text](#text). |
| `mtext` | `contents`, `position`, `height`; `width`, `style`, `width_factor`, `rotation`, `justify` | Text of one or more lines in a box, as AutoCAD's MTEXT draws it. See [Text](#text). |
| `insert` | `block`, `position`; `scale`, `rotation`, `attributes` | A block from the sheet's block library, placed as AutoCAD's INSERT places it. See [Inserts](#inserts). |
| `table` | `position`, `columns`, `rows`, `cells`; `title`, `header`, `table_style` | A grid of cells with text, as AutoCAD's TABLE draws it. See [Tables](#tables). |

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

A justified `text` records in the DWG where its baseline starts as well as where it is justified, as AutoCAD does. AutoCAD draws the text from the start, so a build works the start out from the font's widths, by AutoCAD's own rules; when the font is not in a font folder, the start comes from its look-alike and may sit a little away from where AutoCAD would put it, until the text is edited in AutoCAD. The build's result names every such font.

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
| `attributes` | no | The values of the block's attributes: a mapping from each attribute's tag to its value. A value is one line of text, as a `text`'s contents are, and may be empty, written `''`. A tag the block defines more than once takes a list of values instead, one for each in the order the block defines them, such as `NOTE: [first, second]` ([below](#a-tag-defined-more-than-once)). An attribute the insert leaves out takes its definition's default, and so does one whose value is a bare null, such as `TAG:` ([the YAML subset](#the-yaml-subset), rule 8). A [constant attribute](block-library.md#attribute-modes) takes no value from an insert. A tag written in lowercase is taken in capitals, as the block keeps it. |
| `layer` | no | The layer the insert sits on, as for every object. |

An insert takes an `id` and a `name` as every object does ([IDs and names](#ids-and-names)).

A block that is not in the library, or an attribute tag the block does not define, is an error that lists the library's blocks or the block's attribute tags. An insert's attributes draw where the block's attribute definitions are, moved, scaled and turned with the insert. Only uniform, positive scales are offered: a mirrored or stretched insert waits for a later ticket.

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
- **Its lower left corner is at the origin**, `[0, 0]`: the sheet is drawn in model space at its paper's size from there, as a template draws its trim line.

A template declares its paper as a sheet description does; the set's settings take none, since the template draws the border.

### Fonts

AutoCAD's SHX fonts are Autodesk's files and are never in this repository (ADR-0008). AgentDrafter reads fonts from the font folders a setting names: the folders `--fonts` gives, then those the environment variable `AGENTDRAFTER_FONTS` lists, separated as the system separates paths (`;` on Windows, `:` elsewhere). On a machine that has AutoCAD, name its `Fonts` folder for the SHX fonts and the system's fonts folder for the TrueType ones. There is no default folder. A font is found by its file name, ignoring case.

A font that is in no font folder, or cannot be read, is replaced by a look-alike for measuring. A TrueType font with an open twin of the same widths takes the twin when a font folder holds it: Liberation Sans or Arimo for Arial (`arial.ttf`, `arialbd.ttf`), Liberation Serif or Tinos for Times New Roman (`times.ttf`, `timesbd.ttf`), Liberation Mono or Cousine for Courier New (`cour.ttf`, `courbd.ttf`). Every other font takes Hershey's Roman Simplex, which is built in and in the public domain, and whose letters and digits have the same advances as AutoCAD's `romans.shx`. The DWG still names the style's own font. A build's result lists each substitution under `font_substitutions`: the style, its font, the look-alike used and why.

AgentDrafter reads SHX fonts in the `AutoCAD-86 shapes` and `AutoCAD-86 unifont` layouts, which AutoCAD's own text fonts use, and TrueType fonts through ttf-parser (MIT or Apache-2.0). Big fonts, for Asian scripts, are not read.

A TrueType font encoded as a Windows symbol font, whose character map has a Windows symbol table (platform 3, encoding 0), holds its characters at U+F000 plus their code: `I`, 0x49, at U+F049. Its Unicode table, when it has one, may hold them there too. Some of the TrueType fonts AutoCAD draws a drawing's PostScript fonts with are such fonts. A character that such a font's Unicode tables hold no glyph for is looked up in its symbol table, at its own code and then, below U+0100, at U+F000 plus its code, as Windows and AutoCAD do, so its text measures, draws, plots and is checked in its own characters, not in missing-glyph boxes. A font with no symbol table is read as before. A PDF's searchable text holds the characters as written. Its Mac table (platform 1, encoding 0) is not read.

### Measuring text

`agentdrafter measure-text <contents> --font <file> --height <height> [--width-factor <factor>] [--fonts <folder>]...` measures one line of text as AutoCAD measures a text object (AutoLISP's `textbox`): `width` is from the left edge of what the text draws to the right edge, and `left`, `right`, `bottom` and `top` say where that is from the start of the line, in the unit of the height. `advance` is how far the pen moves. `--font` names a font as a text style does, by its file name alone, and is read only from the font folders. `font` says which font was used and, when it was a look-alike, why. A space at either end draws nothing, so it adds nothing to the width.

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
