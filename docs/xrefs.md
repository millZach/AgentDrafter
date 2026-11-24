# External references (xrefs)

An external reference (`XREF`) links another DWG or DXF file into a host drawing without copying its geometry into the host file. A shared plant grid, title block, or background layout can be maintained in one file and referenced across every sheet that places it.

## Declaring external references in a sheet description or template

Declare external references under the top-level `xrefs` mapping, keyed by block name, and place them on the sheet with regular `insert` objects:

```yaml
units: mm
xrefs:
  PLANT_GRID:
    path: shared/plant-grid.dwg
    kind: attach      # optional: attach (default) or overlay
    unloaded: false   # optional: false (default)
  DRAFT_STAMP:
    path: shared/stamp.dxf
    kind: overlay
objects:
  - insert: {id: grid, block: PLANT_GRID, position: [0, 0, 0]}
  - insert: {id: stamp, block: DRAFT_STAMP, position: [10, 10, 0]}
```

- `path` is relative to the sheet description's or template's folder, with `/` separators.
- `kind` is `attach` (default, carried along when this drawing is itself referenced by another drawing) or `overlay` (drawn in this drawing only, skipped when this drawing is referenced by another).
- `unloaded` keeps the reference in the DWG without loading or drawing its geometry.
- An external reference name cannot collide with a block in the sheet's `block_library`.
- External reference inserts do not carry block attributes or dynamic block state; bind the reference into a local block first if you need them.

## Building to DWG and DXF

`agentdrafter build` writes each `xrefs` entry as a genuine AutoCAD xref `BLOCK_RECORD` (`is_xref`, `is_xref_overlay`, `is_xref_unloaded`, `xref_path`) with no embedded entities, and each placement as an `INSERT` referencing that block name. Both DWG (2018) and ASCII/binary DXF outputs preserve xref definitions and placements.

## Reading, drawing, querying and checking

When `render`, `plot`, `display-list`, `query`, `check`, or `read` opens a sheet description, drawing set, DWG, or DXF:

- Non-unloaded external references are resolved from disk relative to the host file's folder (normalising `\` to `/`, with a fallback to the plain file name beside the host file if a saved path moved).
- At the host level, both `attach` and `overlay` xrefs load. Inside a referenced drawing, only `attach` xrefs are followed recursively; nested `overlay` xrefs are skipped, matching AutoCAD.
- Circular reference chains are detected and broken cleanly without infinite recursion.
- Xref-dependent layers, linetypes, text styles, table styles, multileader styles, dimension styles, and nested blocks are prefixed `<xref>|` (leaving layer `0`, `Continuous`, `ByLayer`, `ByBlock`, and `Standard` unprefixed so layer `0` and `ByBlock` inherit from the host insert). Existing host entries with the same `<xref>|` name (AutoCAD `VISRETAIN` overrides) are preserved.
- If a referenced drawing's declared unit differs from the host's unit, its model-space geometry is scaled by the unit conversion ratio.
- Resolved xref files are recorded in PNG and PDF output stamps (`kind: "xref"`), so `agentdrafter stale` reports an output as stale whenever any referenced drawing changes.
- Any non-unloaded xref that cannot be found, cannot be read, or forms a cycle is reported under `unresolved_xrefs` in the command's result and under top-level `warnings` (`code: "unresolved_xref"`), while the rest of the drawing continues to draw and query.

## Revising originals and change-set operations

On an original DWG or DXF, untouched xref block records, xref-dependent tables, and xref inserts pass through `agentdrafter change` verbatim in the original's exact DWG version (ADR-0004, ADR-0006).

Three typed operations manage external references on both drawing sets (with full dry-run and journal undo) and original DWGs/DXFs:

- **`attach_xref`** (aliases: `XATTACH`, `XREF`): declares an external reference (`name`, `path`, optional `kind`, `unloaded`) and places an `insert` of it (`position` default `[0, 0, 0]`, optional `scale`, `rotation`, `layer`, `space`, `id`, `insert_name`; set `place: false` to declare the reference without placing an insert).
- **`detach_xref`** (alias: `XDETACH`): removes the external reference declaration and deletes every insert that places it.
- **`bind_xref`** (alias: `XBIND`): loads the referenced DWG or DXF, imports its dependent linetypes, layers, text styles, and nested blocks (`mode: "bind"` for AutoCAD's `<xref>$0$` prefixing, the default, or `mode: "insert"` for direct table merging), converts the xref into a local block definition (in the template's `block_library` for a drawing set, or in place inside the DWG/DXF for an original), and keeps all inserts intact.
