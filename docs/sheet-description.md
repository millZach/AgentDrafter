# The sheet description

A sheet description is the text file that states what is on a new sheet. It is the sheet's record: the DWG is built from it (ADR-0003). It is written in a strict subset of YAML, set out below, so that an agent can write it, a person can read it, and a mistake is an error that names its field rather than a guess.

So far a sheet description holds lines, polylines, arcs, circles, ellipses and splines, on layers that carry a colour, a linetype and a lineweight, and text and multiline text in text styles that name a TrueType or an SHX font. Hatch, blocks, attributes and templates come in later tickets, and this page grows with them.

`agentdrafter schema` prints the same keys as a JSON Schema (draft 2020-12), with what each means, for an agent to read before it writes a sheet description.

## An example

```yaml
# The smallest sheet description: one line, in millimetres.
units: mm
objects:
  - line:
      from: [0, 0, 0]
      to: [100, 50, 0]
```

`agentdrafter build examples/one-line.yaml` writes `examples/one-line.dwg` in the 2018 format and prints what it wrote as JSON. `examples/geometry.yaml` holds every kind of geometry, on layers with linetypes, and `examples/text.yaml` every kind of text, in every justification. `examples/text-widths.yaml` holds the fixed strings AgentDrafter's measured widths are compared with AutoCAD's on.

Every DWG a build writes carries AgentDrafter's build mark: a custom drawing property, `AgentDrafter` with the value `build`, which AutoCAD shows under DWGPROPS. `--overwrite` replaces only a DWG that carries it.

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
| `linetypes` | no | The sheet's linetypes: a mapping from each linetype's name to its definition. |
| `layers` | no | The sheet's layers: a mapping from each layer's name to its colour, linetype and lineweight. |
| `text_styles` | no | The sheet's text styles: a mapping from each style's name to its font and width factor. |
| `objects` | yes | A list of objects, which may be empty, in drawing order. Each object is a mapping with one key, its type. |

### Linetypes

A linetype is a pattern of dashes, gaps and dots that a layer draws its objects with. Every drawing already has `Continuous`, the solid line; any other linetype a layer uses is defined here. `Continuous`, `ByLayer` and `ByBlock` are AutoCAD's own and cannot be defined again.

| Key | Required | What it is |
|---|---|---|
| `pattern` | yes | 2 to 12 lengths, repeated along each object: a positive length is a dash, a negative one a gap, and 0 a dot. It starts with a dash or a dot, then a gap, as in an AutoCAD `.lin` file. The lengths are in the declared unit and may carry their own. |
| `description` | no | What AutoCAD's linetype manager shows beside the name, at most 47 characters. |

### Layers

A layer is a named group of objects that draw with its colour, linetype and lineweight. Every drawing already has layer `0`, which is white, continuous and of the default lineweight; listing `0` under `layers` sets its properties.

| Key | Required | What it is |
|---|---|---|
| `color` | no | An AutoCAD colour index from 1 to 255, or one of `red` (1), `yellow` (2), `green` (3), `cyan` (4), `blue` (5), `magenta` (6) or `white` (7). Without it, 7. |
| `linetype` | no | `Continuous`, or a name under `linetypes` written exactly as it is there. Without it, `Continuous`. |
| `lineweight` | no | The pen width the layer plots with, in millimetres, with or without `mm`, written out as digits with a `.` and digits if any, such as `0.35` or `0.50`, with no sign or exponent: one of AutoCAD's standard lineweights, 0, 0.05, 0.09, 0.13, 0.15, 0.18, 0.2, 0.25, 0.3, 0.35, 0.4, 0.5, 0.53, 0.6, 0.7, 0.8, 0.9, 1, 1.06, 1.2, 1.4, 1.58, 2 or 2.11. Without it, AutoCAD's default lineweight. |

A lineweight is the one number on a sheet that is not in the declared unit. It is a width on paper, not a length on the drawing: a 0.35 mm pen is 0.35 mm on a sheet drawn in inches. Written in any unit but `mm`, it is an error.

A layer or linetype name is 1 to 255 characters, does not start or end with a space, and holds none of `< > / \ " : ; ? * | = , `` ` `` and no control characters. AutoCAD ignores case in names, so two names that differ only in case are an error, as the same key given twice would be. An object or a layer names its layer or linetype exactly as it is defined.

### Text styles

A text style is a font that text draws with, and how wide. Every drawing already has `Standard`, whose font is AutoCAD's `txt.shx`; listing `Standard` under `text_styles` sets its font and width factor. A text style's name follows the rules for layer and linetype names above, so `Standard` is written just so: `standard` or `STANDARD` under `text_styles` is an error, since AutoCAD would take it for `Standard`.

| Key | Required | What it is |
|---|---|---|
| `font` | yes | The font, by its file name alone: an AutoCAD SHX font such as `romans.shx`, or a TrueType font such as `arial.ttf`. The extension, `.shx` or `.ttf`, may be in any case. The name holds no folder and none of `/ \ < > : " \| ? *`. |
| `width_factor` | no | How wide characters are drawn, as a share of their normal width: from 0.01 to 100, with 1 normal and 0.8 narrower. Text in the style takes it unless it states its own. Without it, 1. |

The DWG names the font, and AutoCAD finds it on the machine that opens the drawing. AgentDrafter reads the font only to measure text, which it does to place justified text and for `agentdrafter measure-text`; see [Fonts](#fonts).

### Objects

Every object takes a `layer` key: the name of the layer it sits on, `0` or one under `layers`. Without it, the object sits on layer `0`. An object draws with its layer's colour, linetype and lineweight; it has none of its own.

| Type | Keys | What it is |
|---|---|---|
| `line` | `from`, `to` | A straight line between two points. |
| `polyline` | `points`; `closed` | Straight segments joined end to end through the points, in order: at least 2, or 3 when closed. `closed: true` adds a segment from the last point back to the first; without it, `false`. When every point has the same `z`, it is AutoCAD's lightweight polyline at that elevation; otherwise it is a 3D polyline. |
| `arc` | `center`, `radius`, `start_angle`, `end_angle` | Part of a circle, drawn anticlockwise from its start angle to its end angle. The angles must differ once whole turns are taken off; a whole circle is a `circle`. |
| `circle` | `center`, `radius` | A circle. |
| `ellipse` | `center`, `major_axis`, `ratio`; `start_angle`, `end_angle` | An ellipse. `major_axis` runs from the centre to one end of the major axis, as `[dx, dy, dz]`, with `dz` 0. `ratio` is the minor axis over the major axis, more than 0 and at most 1. With `start_angle` and `end_angle`, measured anticlockwise from the major axis, only the part from one to the other is drawn; give both or neither. |
| `spline` | `fit_points` or `control_points`; `degree` | A smooth curve. With `fit_points`, at least 3 points it passes through, no two in a row the same; AutoCAD fits a cubic through them, as its SPLINE command does by default. With `control_points`, the control points of a B-spline that starts at the first and ends at the last, of `degree` 1 to 10 (without it, 3), with more control points than its degree. Not both. |
| `text` | `contents`, `position`, `height`; `style`, `width_factor`, `rotation`, `justify` | A single line of text, as AutoCAD's TEXT draws it. See [Text](#text). |
| `mtext` | `contents`, `position`, `height`; `width`, `style`, `width_factor`, `rotation`, `justify` | Text of one or more lines in a box, as AutoCAD's MTEXT draws it. See [Text](#text). |

Keys before the semicolon are required; keys after it are not. A `radius` is a length more than 0.

Arcs, circles and ellipses lie in the plane parallel to XY at their centre's `z`. Lines, polylines and splines may run anywhere in 3D.

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
| `contents` | both | What the text says, exactly as it shows. Not empty, and no control characters. A `text` is one line; in `mtext` a line break starts a new line, and line breaks at the end are dropped, so a YAML block (`\|`) can hold it. |
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

### Fonts

AutoCAD's SHX fonts are Autodesk's files and are never in this repository (ADR-0008). AgentDrafter reads fonts from the font folders a setting names: the folders `--fonts` gives, then those the environment variable `AGENTDRAFTER_FONTS` lists, separated as the system separates paths (`;` on Windows, `:` elsewhere). On a machine that has AutoCAD, name its `Fonts` folder for the SHX fonts and the system's fonts folder for the TrueType ones. There is no default folder. A font is found by its file name, ignoring case.

A font that is in no font folder, or cannot be read, is replaced by a look-alike for measuring. A TrueType font with an open twin of the same widths takes the twin when a font folder holds it: Liberation Sans or Arimo for Arial (`arial.ttf`, `arialbd.ttf`), Liberation Serif or Tinos for Times New Roman (`times.ttf`, `timesbd.ttf`), Liberation Mono or Cousine for Courier New (`cour.ttf`, `courbd.ttf`). Every other font takes Hershey's Roman Simplex, which is built in and in the public domain, and whose letters and digits have the same advances as AutoCAD's `romans.shx`. The DWG still names the style's own font. A build's result lists each substitution under `font_substitutions`: the style, its font, the look-alike used and why.

AgentDrafter reads SHX fonts in the `AutoCAD-86 shapes` and `AutoCAD-86 unifont` layouts, which AutoCAD's own text fonts use, and TrueType fonts through ttf-parser (MIT or Apache-2.0). Big fonts, for Asian scripts, are not read.

### Measuring text

`agentdrafter measure-text <contents> --font <file> --height <height> [--width-factor <factor>] [--fonts <folder>]...` measures one line of text as AutoCAD measures a text object (AutoLISP's `textbox`): `width` is from the left edge of what the text draws to the right edge, and `left`, `right`, `bottom` and `top` say where that is from the start of the line, in the unit of the height. `advance` is how far the pen moves. `--font` names a font as a text style does, by its file name alone, and is read only from the font folders. `font` says which font was used and, when it was a look-alike, why. A space at either end draws nothing, so it adds nothing to the width.

Measured against AutoCAD 2026 with its own fonts, on the machine that has them: the widths of 139 made-up strings in `romans.shx`, `simplex.shx`, `txt.shx`, `isocp.shx`, Arial and Times New Roman, 0.1 to 7 units high, match AutoCAD's to within 1e-12 of the unit, and the box of each of the 1,151 characters of the four SHX fonts, 100 high, matches to the eight decimal places AutoCAD printed, but for the ogonek letters Ą, ą, Ę and ę in `romans.shx` and Ą in `simplex.shx`, which AutoCAD boxes up to 0.12 of the height wider or deeper. AutoCAD does not kern.

## Values

A **point** is a list of exactly three lengths, `[x, y, z]`. Every coordinate is 3D; on a flat sheet `z` is `0`.

A **length** is a number, with or without its own unit:

- `100` is 100 of the declared unit.
- `4 in` or `4in` is four inches, converted to the declared unit.
- A number is an optional `-`, digits, an optional `.` followed by digits, and an optional exponent such as `e3` or `E-2`. Nothing else is a number: not `+1`, `.5`, `5.`, `1_000`, `0x10`, `inf` or `nan`.
- A unit is one of `mm`, `cm`, `m`, `in`, `ft`, in lower case, after the number and at most one space.
- A length that is too large to hold once converted to the declared unit, such as `1e308 m` on a sheet in `mm`, is an error.

An **angle** is a number of degrees, counted anticlockwise, with no unit. Whole turns are taken off, so `360` is `0`, `-90` is `270` and `450` is `90`.

A **ratio** is a number with no unit, and so is a **width factor**. A **degree** is a whole number, digits only. `closed` is `true` or `false`, in lower case. Text **contents** are text: `contents: 100` says 100.

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
7. **Every scalar is text; the sheet description gives it its meaning.** YAML's guessing of types is off: `yes`, `no`, `null`, `~`, `0x10` and `.inf` are not booleans, nulls or numbers. A coordinate is read as a length and nothing else, so `null` there is an error. Quoted and plain scalars mean the same: `'mm'`, `"mm"` and `mm` are one unit, and `layer: 0` and `layer: '0'` are one layer.

Comments, block style and flow style (`[0, 0, 0]`, `{from: ..., to: ...}`) are all allowed.

## The schema

`agentdrafter schema` prints one line of JSON, `{"ok":true,"result":{...}}`, whose result is a JSON Schema of the sheet description. JSON Schema describes JSON, and a YAML reader that guesses types turns `100` into a number and `4 in` into a string, so in the schema a length is a number or a string that reads as one. Quoted and plain scalars mean the same here, so the schema takes every value the reader takes in either form: `color: '42'`, `lineweight: '0.50 mm'`, `closed: 'true'` and `degree: '2'` pass both. What JSON Schema cannot see is left to the reader: such a YAML reader turns `+5` and `5` into one number, and some checks compare one value with another, such as a degree with the number of control points. The rules of the YAML subset are not JSON Schema either; the schema's top-level description states them. The schema is built from the same tables the reader checks keys against, and tests keep the two in step.

## Errors

`agentdrafter build` prints one line of JSON on standard output. On failure it is:

```json
{"ok":false,"error":{"code":"unknown_key","message":"unknown key `too`; a line takes from, to, layer","field":"objects[0].line.too","line":5,"column":7,"file":"bad.yaml"}}
```

`field` is the path to the value in the sheet description: keys joined with `.`, list items as `[index]` counted from 0, such as `layers.hidden.linetype` or `objects[2].arc.radius`. `line` and `column` count from 1. `field`, `line`, `column` and `file` appear when they are known. An error about a choice, such as a key, a unit, a layer, a linetype, a text style, a colour, a lineweight or a justification, lists the valid choices in its message.

| Code | Meaning |
|---|---|
| `usage` | The command was called wrongly, or the output path is the sheet description itself. |
| `read_failed` | The sheet description could not be read. |
| `yaml_syntax` | The text is not well-formed YAML. |
| `not_in_subset` | An anchor, alias, tag, directive, second document or key that is not text. |
| `duplicate_key` | A key appears twice in one mapping, or two layer or linetype names differ only in case. The message gives the line of the first. |
| `unknown_key` | A key the sheet description does not define, including an unknown object type. |
| `missing_key` | A required key is missing, or the file is empty. |
| `missing_unit` | The sheet declares no unit. |
| `unknown_unit` | A unit that is not one of `mm`, `cm`, `m`, `in`, `ft`. |
| `wrong_type` | A value of the wrong shape, such as a point without three coordinates, a polyline with too few points, or `closed: yes`. |
| `bad_number` | A value that should be a number is not one, carries a unit where none belongs, or is too large to hold in the declared unit. |
| `bad_value` | A value of the right shape that is not allowed: a colour outside 1 to 255, a lineweight that is not standard, a radius or text height that is not positive, a ratio outside 0 to 1, a width factor outside 0.01 to 100, an arc whose angles are equal, a linetype pattern that breaks AutoCAD's rules, a name AutoCAD would refuse, a text style that is `Standard` in another case, a font that is not an SHX or TrueType file name, a justification that does not exist, text contents that are empty or would not show as written. |
| `unknown_name` | A layer, linetype or text style that the sheet description does not define. |
| `output_exists` | A file is already where the DWG would go, and `--overwrite` was not given. |
| `not_a_build` | `--overwrite` was given, but the file where the DWG would go is not a DWG AgentDrafter built, or cannot be read to tell. It may be an original, and an original is never overwritten. |
| `write_failed` | The DWG could not be written. |
| `dwg_write` | The DWG library could not write the drawing. |
| `dwg_read` | The DWG library could not read a drawing. |

## How it is read

The YAML is read by [saphyr-parser](https://crates.io/crates/saphyr-parser) (MIT or Apache-2.0), a YAML 1.2 parser that reports each node with its anchor, tag and position. AgentDrafter builds its own tree from those events, refusing anything outside the subset as it goes, so a duplicate key or an anchor is caught where it is written rather than lost in a parsed value. The code is `crates/agentdrafter-core/src/description/`: `mod.rs` reads the top level and the values, `layers.rs` the linetypes and layers, `objects.rs` the objects, `text.rs` the text styles and text, and `schema.rs` writes the schema. Fonts and measuring are in `crates/agentdrafter-core/src/text/`, and text's side of the DWG in `src/dwg/text.rs`.
