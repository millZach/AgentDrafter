# ADR-0012: 3D solids are stored as native ACIS `3DSOLID` in DWG and drawn as 2D wireframe

**Date:** 2026-11-24
**Status:** Accepted

## Context

`docs/plan.md` left how 3D solids are stored inside DWG as an open decision to be resolved when 3D solids (#96) were designed. Earlier in the project it was assumed no open-source library could write ACIS `3DSOLID` data for curved shapes.

Since then, our pinned DWG library (`acadrust`, ADR-0009) gained:
- Full native ACIS SAT and binary SAB (`AcDsPrototype_1b`) reading and writing for `3DSOLID` entities in R2018 DWG (`AC1032`) and DXF.
- Exact analytic ACIS B-rep builders for standard primitives (`box`, `cylinder`, `cone`, `sphere`, `torus`, `wedge`, `pyramid`) including analytic curved surfaces (`cone-surface`, `sphere-surface`, `torus-surface`, `ellipse-curve`).
- A general validated closed 2-manifold planar ACIS B-rep builder (`build_planar_body(vertices, faces)`) that emits genuine AutoCAD-auditable `3DSOLID` bodies for arbitrary closed polyhedra.

## Decision

1. **DWG/DXF storage:** Store every 3D solid as a native AutoCAD `3DSOLID` entity through `acadrust`, with zero new external dependencies and keeping the core pure Rust, Apache-2.0 compatible (ADR-0010), and WebAssembly-safe (ADR-0011):
   - Un-booleaned primitives (`box`, `cylinder`, `cone`, `sphere`, `torus`, `wedge`, `pyramid`) emit exact analytic ACIS B-reps via `acadrust`.
   - `extrude`, `revolve`, and CSG boolean results (`union`, `subtract`, `intersect`) are evaluated by a pure-Rust 3D solid & BSP CSG kernel into closed, T-junction-free 2-manifold planar polyhedra and written as native ACIS `3DSOLID` bodies via `build_planar_body`.
   - Sheet descriptions store the clean parametric recipe (`box`, `cylinder`, `extrude`, `revolve`, `union`/`subtract`/`intersect`, or `mesh`) in the strict YAML subset (ADR-0003).
2. **One drawing pipeline (ADR-0011):** Draw 3D solids across PNG, PDF, and the WebGL2 viewer as **2D Wireframe** (feature edges plus view-dependent silhouettes, matching AutoCAD's `2D Wireframe` plot/display), projecting top-down plan view (`[0, 0, 1]`) by default and supporting arbitrary orthographic/isometric `view_direction` on paper-space viewports (`Viewport::view_direction`).

## Consequences

- Every DWG containing 3D solids opens in AutoCAD as genuine `3DSOLID` entities with zero audit errors and supports AutoCAD's native 3D commands (`MASSPROP`, `SECTIONPLANE`, shading, orbiting).
- Curved boolean intersections and curved extrusions/revolves are represented as high-resolution faceted planar B-reps inside ACIS `3DSOLID`, rather than analytic NURBS trim curves, avoiding C++/LGPL kernel dependencies (such as OpenCASCADE) and preserving WASM32 compilation.
- Resolves the open "Real AutoCAD solids inside DWG" item in `docs/plan.md`.
