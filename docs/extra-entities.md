# Points, construction lines, rays, multilines, tolerances, centre marks, centre lines and shapes

AgentDrafter supports eight additional 2D AutoCAD object types — `point`, `xline`, `ray`, `mline`, `tolerance`, `center_mark`, `center_line` and `shape` — across sheet descriptions, block libraries, the single drawing pipeline (PNG, PDF and WebGL2 display list), DWG 2018 and ASCII/binary DXF round-trips, object snaps and queries, change sets, and original-DWG revisions.

## Sheet description keys

### Drawing-wide settings

- **`pdmode`** (`PDMODE`, integer, default `0`): AutoCAD point display code. Base marker `0` (dot), `1` (blank), `2` (`+`), `3` (`×`), or `4` (tick `|`), optionally combined with `+32` (circle), `+64` (square), or `+96` (circle and square).
- **`pdsize`** (`PDSIZE`, length or number, default `0`): point display size. Positive values are absolute drawing units; `0` draws at 5% of the view height; negative values draw at `|pdsize|%` of the view height.
- **`mline_styles`** (`MLSTYLE` / `ACAD_MLINESTYLE`, mapping): named multiline styles. AutoCAD's `Standard` (`+0.5` and `-0.5`, 90° ends) is always available without declaring it. Each style takes:
  - `description` (optional string)
  - `elements` (non-empty list of `{offset, color?, linetype?}` where `offset` is the perpendicular distance from the multiline zero axis)
  - `fill_color` (optional colour; enables background fill between outermost elements)
  - `display_joints` (boolean, default `false`)
  - `start_angle`, `end_angle` (degrees in `10..=170`, default `90`)
  - `start_square_cap`, `start_round_cap`, `start_inner_arcs_cap`, `end_square_cap`, `end_round_cap`, `end_inner_arcs_cap` (booleans, default `false`)

### Objects

Every object accepts the standard `layer`, `color`, `linetype`, `lineweight`, `linetype_scale`, `plot_style`, `transparency`, `id`, `name`, and `space` keys.

- **`point`** (`POINT` / `PO`):
  - `position` (point, required)
  - `angle` (degrees, default `0`, rotates the marker)
- **`xline`** (`XLINE` / `XL`): infinite construction line.
  - `through` (point, required)
  - `direction` (non-zero direction vector `[dx, dy, dz]`, required; normalized on read)
- **`ray`** (`RAY`): semi-infinite construction ray.
  - `from` (point, required)
  - `direction` (non-zero direction vector `[dx, dy, dz]`, required; normalized on read)
- **`mline`** (`MLINE` / `ML`): parallel-element multiline.
  - `points` (list of 2+ points, required)
  - `style` (string, default `Standard`)
  - `scale` (finite non-zero number, default `1.0`)
  - `justification` (`top`, `zero`, or `bottom`, default `top`)
  - `closed` (boolean, default `false`)
  - `suppress_start_caps`, `suppress_end_caps` (booleans, default `false`)
- **`tolerance`** (`TOLERANCE` / `TOL`): GD&T feature control frame.
  - `position` (point, required)
  - `text` (AutoCAD tolerance text string, required — rows separated by `^J` or `\n`, cells by `%%v`, GD&T symbols via `{\Fgdt;<letter>}` such as `j` position, `n` diameter, `m` MMC, `l` LMC, `b` perpendicularity, `f` parallelism, `c` flatness, `r` circularity, `g` cylindricity, `d` profile of a surface, `k` profile of a line, `e` concentricity, `i` symmetry, `h` circular runout, `t` total runout, `a` angularity, `u` straightness, `s` projected tolerance zone)
  - `direction` (degrees, default `0`)
  - `dimension_style` (string, default sheet dimension style)
  - `text_height` (optional length override; otherwise taken from `dimension_style`)
- **`center_mark`** (`CENTERMARK`): crosshair with optional radial extension lines.
  - `center` (point, required)
  - `radius` (positive length, required)
  - `cross_size`, `cross_gap`, `extension` (optional lengths; default proportionally from `radius`)
  - `show_extensions` (boolean, default `true`)
  - `rotation` (degrees, default `0`)
- **`center_line`** (`CENTERLINE`): centre line segment with independent start/end extensions.
  - `from`, `to` (points, required)
  - `start_extension`, `end_extension` (lengths, default `0`)
- **`shape`** (`SHAPE`): compiled SHX shape placement.
  - `file` (`.shx` shape filename, required; searched in `AGENTDRAFTER_FONTS` / `--fonts`, never bundled from Autodesk)
  - `shape` (shape name inside the `.shx` file) and/or `number` (integer `1..=65535`) — at least one required
  - `position` (point, required)
  - `size` (positive length, required)
  - `rotation` (degrees, default `0`)
  - `width_factor` (positive number, default `1.0`)
  - `oblique` (degrees in `-85..=85`, default `0`)

## Drawing pipeline, DWG/DXF and snaps

- **Single drawing pipeline (ADR-0011):** `point` draws its `PDMODE`/`PDSIZE` symbol; `xline` and `ray` clip cleanly against model/viewport bounds via Liang–Barsky clipping so they never blow out sheet extents; `mline` computes exact mitered offset chains, background fill, joints, and square/round/inner-arc caps; `tolerance` draws crisp vector GD&T symbols inside boxed frames with searchable PDF text; `center_mark` and `center_line` draw their crosshair and extended segments; `shape` resolves strokes from compiled `.shx` shape files before drawing so `draw/` remains pure and WASM32-safe.
- **DWG 2018 and ASCII/binary DXF:** `POINT`, `XLINE`, `RAY`, `MLINE` (with `ACAD_MLINESTYLE` dictionary and exact vertex miters/segment parameters), `TOLERANCE`, and `SHAPE` (with `is_shape_file` `STYLE` entry) write natively. `CENTERMARK` writes an anonymous `*U<n>` block `INSERT` tagged with `OCS_CENTERMARK` XDATA; `CENTERLINE` writes a `LINE` spanning its extended endpoints tagged with `OCS_CENTERLINE` XDATA. All eight round-trip cleanly through `read`, `render`, `plot`, `display-list`, `query` and `check`.
- **Snaps and queries:** `point` exposes `node`; `ray` exposes `endpoint` at `from`; `mline` exposes `endpoint`/`midpoint` along its elements; `center_mark` exposes `center` and extension `endpoint`s; `center_line` exposes `endpoint`/`midpoint`; `tolerance` and `shape` expose `insertion`.

## Change-set operations

All ten operations work on both drawing sets (with byte-for-byte `undo`) and original DWGs (preserving untouched objects verbatim):

| Operation | AutoCAD alias(es) | Purpose |
|---|---|---|
| `add_point` | `POINT`, `PO` | Add a point object |
| `add_xline` | `XLINE`, `XL` | Add an infinite construction line |
| `add_ray` | `RAY` | Add a semi-infinite ray |
| `add_mline` | `MLINE`, `ML` | Add a multiline |
| `add_tolerance` | `TOLERANCE`, `TOL` | Add a GD&T feature control frame |
| `add_center_mark` | `CENTERMARK` | Add a centre mark |
| `add_center_line` | `CENTERLINE` | Add a centre line |
| `add_shape` | `SHAPE` | Place an SHX shape |
| `set_point_style` | `DDPTYPE`, `PTYPE`, `PDMODE`, `PDSIZE` | Set drawing-wide `pdmode` and/or `pdsize` |
| `add_mline_style` | `MLSTYLE` | Add or update a named multiline style |

Existing editing operations — `move`, `rotate`, `scale`, `mirror`, `stretch`, `copy`, `array`, `delete`, `change`, and `explode` (exploding `mline` into element/joint/cap `line`s and `center_mark` into crosshair/extension `line`s) — also support these objects.
