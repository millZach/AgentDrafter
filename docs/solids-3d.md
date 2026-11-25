# 3D solids (`solid3d`)

AgentDrafter builds, reads, revises, queries, checks, renders, and plots native AutoCAD 3D solids (`3DSOLID`, ACIS B-rep) without AutoCAD and without leaving its single drawing pipeline ([ADR-0011](adr/0011-one-drawing-pipeline.md), [ADR-0012](adr/0012-3d-solids-are-acis-3dsolid-in-dwg.md)).

## What a `solid3d` holds

In a sheet description, template, row, or block definition, a `solid3d` object takes `kind` plus the keys of that kind (and any standard object keys: `layer`, `color`, `linetype`, `lineweight`, `plot_style`, `transparency`, `space`, `id`, `name`):

### Analytic primitives

Written into DWG/DXF as exact analytic ACIS B-rep surfaces (`plane-surface`, `cylinder-surface`, `cone-surface`, `sphere-surface`, `torus-surface`):

| `kind` | Required keys | Notes |
|---|---|---|
| `box` | `origin: [x, y, z]`, `size: [length_x, width_y, height_z]` | Axis-aligned rectangular box from minimum corner `origin`. |
| `cylinder` | `base: [x, y, z]`, `radius`, `height` | Right circular cylinder standing along `+Z` from bottom centre `base`. |
| `cone` | `base: [x, y, z]`, `radius`, `height` | Right circular cone standing along `+Z` from base centre `base` to apex at `base.z + height`. |
| `sphere` | `center: [x, y, z]`, `radius` | Sphere centred at `center`. |
| `torus` | `center: [x, y, z]`, `major_radius`, `minor_radius` | Torus in the XY plane around `+Z` (`minor_radius < major_radius`). |
| `wedge` | `origin: [x, y, z]`, `size: [length_x, width_y, height_z]` | Axis-aligned right-triangular wedge sloping down from `origin.x` (`height`) to `origin.x + length_x` (`0`). |
| `pyramid` | `base: [x, y, z]`, `base_size`, `height` | Square-base right pyramid centred at `base` with apex at `base.z + height`. |

### Sweeps, CSG booleans, and planar polyhedra

Evaluated to a validated closed 2-manifold planar polyhedron and written into DWG/DXF as a native ACIS `3DSOLID` planar B-rep:

| `kind` | Required keys | Optional keys | Notes |
|---|---|---|---|
| `extrude` | `profile`, `height` | `direction` (`[0, 0, 1]`), `taper_angle` (`-85..85` deg) | Extrudes a closed planar profile (`>= 3` points, optionally with `bulge` or `arc_angle`). Non-zero `height`; negative extrudes opposite `direction`. |
| `revolve` | `profile`, `axis_origin`, `axis_direction` | `angle` (`360`, `(0, 360]`) | Revolves a closed planar profile around the 3D axis. Full 360° or partial sweep with start/end cap faces. |
| `union` | `operands` (`>= 2` solid bodies) | — | Pure-Rust 3D BSP CSG union. |
| `subtract` | `operands` (`>= 2` solid bodies) | — | Pure-Rust 3D BSP CSG difference (`operands[0]` minus `operands[1..]`). |
| `intersect` | `operands` (`>= 2` solid bodies) | — | Pure-Rust 3D BSP CSG intersection (must be non-empty). |
| `mesh` | `vertices` (`>= 4`), `faces` (`>= 4`, each `>= 3` indices) | — | Direct closed 2-manifold planar polyhedron with outward-CCW faces. |

## Change-set operations

All eight 3D solid operations work on **drawing sets** (rows and templates, with full byte-for-byte `undo`) and on **original DWGs** (revising in the original DWG version without touching unmodified entities):

| Operation | AutoCAD aliases | What it does |
|---|---|---|
| `add_solid3d` | `3DSOLID` | Adds any `solid3d` kind (`box`, `cylinder`, `cone`, `sphere`, `torus`, `wedge`, `pyramid`, `extrude`, `revolve`, `union`, `subtract`, `intersect`, `mesh`). |
| `add_box` | `BOX` | Shortcut for `solid3d` with `kind: box` (`origin`, and `size: [l, w, h]` or `length`, `width`, `height`). |
| `add_cylinder` | `CYLINDER` | Shortcut for `solid3d` with `kind: cylinder` (`base`, `radius`, `height`). |
| `extrude` | `EXTRUDE`, `EXT` | Extrudes an existing closed 2D `target` (closed `polyline`, `circle`, or whole `ellipse`) by `height` along optional `direction` and `taper_angle`. An untapered `+Z` extrusion of a `circle` yields an exact analytic ACIS `cylinder`. Erases the source profile by default (`erase_source: false` keeps it). |
| `revolve` | `REVOLVE`, `REV` | Revolves an existing closed 2D `target` around `axis_origin` (may be a snapped point) and `axis_direction` by `angle` degrees (default `360`). Erases the source profile by default. |
| `union` | `UNION`, `UNI` | Combines two or more `solid3d` objects under `targets` by CSG union. The first solid's ID carries on (unless `id` overrides it) and the other solids are removed. |
| `subtract` | `SUBTRACT`, `SU` | Subtracts one or more `solid3d` objects under `subtract` from `target`. The base solid's ID carries on and the tool solids are removed. |
| `intersect` | `INTERSECT`, `IN` | Intersects two or more `solid3d` objects under `targets`. Fails cleanly if the intersection is empty. |

Standard transform operations (`move`, `rotate`, `scale`, `mirror`, `copy`, `array`, `explode`) also support `solid3d` objects:
- `move` shifts the solid in 3D (`[dx, dy, dz]`).
- `rotate`, `scale`, and `mirror` transform both YAML sheet descriptions and ACIS `3DSOLID` entities on original DWGs.
- `explode` turns a `solid3d` into the 3D wireframe `line` objects that draw its edges.

## 2D wireframe projection and viewport `view_direction`

Every `solid3d` draws through the single drawing pipeline (`draw/` -> PNG, PDF, and WebGL2 display list) as **2D Wireframe** (feature edges + view-dependent silhouettes, matching AutoCAD's `2D Wireframe` visual style):

- Model space draws top-down plan (`[0, 0, 1]`) by default.
- Paper-space viewports accept an optional `view_direction` (read from and written to DWG `VIEWPORT` group 16/26/36):
  - Named presets: `top`, `bottom`, `front`, `back`, `left`, `right`, `sw_isometric`, `se_isometric`, `ne_isometric`, `nw_isometric`
  - Or any non-zero 3D vector `[x, y, z]` (for example `[1, -1, 1]` for SE isometric).

This allows a single sheet to show orthographic top, front, and right views alongside an isometric view of the same model-space 3D solids, identically across `render`, `plot`, and the browser viewer.

## Querying and measuring 3D solids

- `query geometry` reports exact 3D `extents` (`min` / `max` with Z), `volume`, and `surface_area` (analytic formulas for primitives; exact signed-tetrahedron divergence volume and face-area sum for sweeps, CSG booleans, and meshes).
- `query snap` finds `endpoint` snaps on solid vertices and `center` at the solid's centroid.
- `read` on an original DWG decodes ACIS `3DSOLID` records (SAB in R2013+/AC1027+ and SAT in older/DXF files), recognizes primitives and planar B-reps, and reports unsupported curved/NURBS ACIS bodies under `not_drawn` while preserving them untouched during revisions (ADR-0006).
