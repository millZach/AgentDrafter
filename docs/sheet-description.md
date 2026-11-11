# The sheet description

A sheet description is the text file that states what is on a new sheet. It is the sheet's record: the DWG is built from it (ADR-0003). It is written in a strict subset of YAML, set out below, so that an agent can write it, a person can read it, and a mistake is an error that names its field rather than a guess.

So far a sheet description holds lines, polylines, arcs, circles, ellipses, splines and hatches, on layers that carry a colour, a linetype and a lineweight. Text, blocks, attributes and templates come in later tickets, and this page grows with them.

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

`agentdrafter build examples/one-line.yaml` writes `examples/one-line.dwg` in the 2018 format and prints what it wrote as JSON. `examples/geometry.yaml` holds every kind of line and curve, on layers with linetypes, and `examples/hatch.yaml` holds solid and patterned hatches.

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
| `hatch_patterns` | no | The sheet's hatch patterns: a mapping from each pattern's name to its lines. |
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

### Hatch patterns

A hatch pattern is one or more families of parallel lines that a hatch fills its area with, defined the way an AutoCAD `.pat` file defines one. `SOLID`, AutoCAD's solid fill, needs no definition; every other pattern a hatch names is defined here. AutoCAD's own pattern files are not copied into AgentDrafter, so a pattern such as `ANSI31` is defined here too if a sheet wants it.

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
| `hatch` | `pattern`, `boundary`; `scale`, `angle` | An area filled solid or with a pattern, inside a boundary; see below. |

Keys before the semicolon are required; keys after it are not. A `radius` is a length more than 0.

Arcs, circles and ellipses lie in the plane parallel to XY at their centre's `z`, and so does a hatch, at the `z` its boundary's points share. Lines, polylines and splines may run anywhere in 3D.

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
| `pattern` | yes | `SOLID` for a solid fill, or the name of a pattern under `hatch_patterns`, written exactly as it is there. |
| `scale` | no | How much the pattern is enlarged: a number more than 0, with no unit. Without it, 1. Not given with `SOLID`. |
| `angle` | no | How far the pattern is turned, an angle. Without it, 0. Not given with `SOLID`. |
| `boundary` | yes | The closed loops that bound the area: a list of at least one. |
| `layer` | no | The layer the hatch sits on, as for every object. |

Each loop is a mapping with one key:

- `polyline`, which takes `points`: straight segments through at least 3 points, in order, always closed back to the first.
- `edges`: a list of at least 2 lines and arcs joined end to end, the last ending where the first starts. An edge is a mapping with one key: `line`, which takes `from` and `to`, or `arc`, which takes `center`, `radius`, `start_angle` and `end_angle` and runs anticlockwise from its start angle to its end angle, as an `arc` object does. Each line starts where the edge before it ends. An arc may be walked either way: forwards when it starts where the edge before it ends, backwards when it ends there. A loop's first arc is walked whichever way lets the second edge join it. A whole circle is two arcs. Two ends join when they are within a billionth of the largest coordinate or radius in the loop, or of one unit if that is more.

A point inside an odd number of loops is filled, as AutoCAD's normal island detection reads a hatch: a loop inside another is a hole, and a loop inside that hole is filled again. Every point of every loop has the same `z`. A hatch draws no outline; a sheet that wants one draws its boundary again as lines, polylines and arcs.

A hatch is written as AutoCAD's HATCH: not associative, so it does not follow its outline when the outline is edited, with every loop marked as one drawn by selecting objects. A pattern from `hatch_patterns` is written as a custom pattern, its lines stored in the hatch, as a DWG stores every hatch's pattern.

## Values

A **point** is a list of exactly three lengths, `[x, y, z]`. Every coordinate is 3D; on a flat sheet `z` is `0`.

A **length** is a number, with or without its own unit:

- `100` is 100 of the declared unit.
- `4 in` or `4in` is four inches, converted to the declared unit.
- A number is an optional `-`, digits, an optional `.` followed by digits, and an optional exponent such as `e3` or `E-2`. Nothing else is a number: not `+1`, `.5`, `5.`, `1_000`, `0x10`, `inf` or `nan`.
- A unit is one of `mm`, `cm`, `m`, `in`, `ft`, in lower case, after the number and at most one space.
- A length that is too large to hold once converted to the declared unit, such as `1e308 m` on a sheet in `mm`, is an error.

An **angle** is a number of degrees, counted anticlockwise, with no unit. Whole turns are taken off, so `360` is `0`, `-90` is `270` and `450` is `90`.

A **ratio** and a **scale** are numbers with no unit. A **degree** is a whole number, digits only. `closed` is `true` or `false`, in lower case.

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
7. **Every scalar is text; the sheet description gives it its meaning.** YAML's guessing of types is off: `yes`, `no`, `null`, `~`, `0x10` and `.inf` are not booleans, nulls or numbers. A coordinate is read as a length and nothing else, so `null` there is an error. Quoted and plain scalars mean the same: `'mm'`, `"mm"` and `mm` are one unit, and `layer: 0` and `layer: '0'` are one layer.

Comments, block style and flow style (`[0, 0, 0]`, `{from: ..., to: ...}`) are all allowed.

## The schema

`agentdrafter schema` prints one line of JSON, `{"ok":true,"result":{...}}`, whose result is a JSON Schema of the sheet description. JSON Schema describes JSON, and a YAML reader that guesses types turns `100` into a number and `4 in` into a string, so in the schema a length is a number or a string that reads as one. Quoted and plain scalars mean the same here, so the schema takes every value the reader takes in either form: `color: '42'`, `lineweight: '0.50 mm'`, `closed: 'true'` and `degree: '2'` pass both. What JSON Schema cannot see is left to the reader: such a YAML reader turns `+5` and `5` into one number, and some checks compare one value with another, such as a degree with the number of control points. The rules of the YAML subset are not JSON Schema either; the schema's top-level description states them. The schema is built from the same tables the reader checks keys against, and tests keep the two in step.

## Errors

`agentdrafter build` prints one line of JSON on standard output. On failure it is:

```json
{"ok":false,"error":{"code":"unknown_key","message":"unknown key `too`; a line takes from, to, layer","field":"objects[0].line.too","line":5,"column":7,"file":"bad.yaml"}}
```

`field` is the path to the value in the sheet description: keys joined with `.`, list items as `[index]` counted from 0, such as `layers.hidden.linetype` or `objects[2].arc.radius`. `line` and `column` count from 1. `field`, `line`, `column` and `file` appear when they are known. An error about a choice, such as a key, a unit, a layer, a linetype, a colour or a lineweight, lists the valid choices in its message.

| Code | Meaning |
|---|---|
| `usage` | The command was called wrongly, or the output path is the sheet description itself. |
| `read_failed` | The sheet description could not be read. |
| `yaml_syntax` | The text is not well-formed YAML. |
| `not_in_subset` | An anchor, alias, tag, directive, second document or key that is not text. |
| `duplicate_key` | A key appears twice in one mapping, or two layer, linetype or hatch pattern names differ only in case. The message gives the line of the first. |
| `unknown_key` | A key the sheet description does not define, including an unknown object type. |
| `missing_key` | A required key is missing, or the file is empty. |
| `missing_unit` | The sheet declares no unit. |
| `unknown_unit` | A unit that is not one of `mm`, `cm`, `m`, `in`, `ft`. |
| `wrong_type` | A value of the wrong shape, such as a point without three coordinates, a polyline with too few points, a hatch with no loops, or `closed: yes`. |
| `bad_number` | A value that should be a number is not one, carries a unit where none belongs, or is too large to hold in the declared unit. |
| `bad_value` | A value of the right shape that is not allowed: a colour outside 1 to 255, a lineweight that is not standard, a radius that is not positive, a ratio outside 0 to 1, an arc whose angles are equal, a linetype pattern that breaks AutoCAD's rules, a name AutoCAD would refuse, a hatch pattern line whose lines would lie on one another, a hatch scale or angle given with `SOLID`, a boundary that does not lie flat or whose edges do not join up. |
| `unknown_name` | A layer, linetype or hatch pattern that the sheet description does not define. |
| `output_exists` | A file is already where the DWG would go, and `--overwrite` was not given. |
| `not_a_build` | `--overwrite` was given, but the file where the DWG would go is not a DWG AgentDrafter built, or cannot be read to tell. It may be an original, and an original is never overwritten. |
| `write_failed` | The DWG could not be written. |
| `dwg_write` | The DWG library could not write the drawing. |
| `dwg_read` | The DWG library could not read a drawing. |

## How it is read

The YAML is read by [saphyr-parser](https://crates.io/crates/saphyr-parser) (MIT or Apache-2.0), a YAML 1.2 parser that reports each node with its anchor, tag and position. AgentDrafter builds its own tree from those events, refusing anything outside the subset as it goes, so a duplicate key or an anchor is caught where it is written rather than lost in a parsed value. The code is `crates/agentdrafter-core/src/description/`: `mod.rs` reads the top level and the values, `layers.rs` the linetypes and layers, `objects.rs` the objects, `hatch.rs` the hatch patterns and hatches, and `schema.rs` writes the schema, with `hatch_schema.rs` writing its hatch parts.
