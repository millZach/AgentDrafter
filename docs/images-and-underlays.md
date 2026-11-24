# Raster images and PDF underlays

AgentDrafter supports attached raster images (`IMAGE` / `IMAGEATTACH`) and PDF underlays (`PDFUNDERLAY` / `PDFATTACH`) on sheet descriptions, templates, rows, inside blocks (such as a company logo in a title block), and on original DWGs and DXFs.

## Sheet description syntax

### Raster image (`image`)

```yaml
units: mm
image_frame: off          # off (default) | on | display  (AutoCAD IMAGEFRAME)
objects:
  - image:
      id: logo
      path: assets/company-logo.png   # PNG, JPEG, BMP, GIF, or TIFF
      position: [12, 8, 0]            # lower-left corner in sheet units
      width: 35                       # displayed width (required, > 0)
      # height: 15                    # optional; defaults to width * height_px / width_px
      rotation: 0                     # anticlockwise degrees (default 0)
      brightness: 50                  # 0..=100 (default 50)
      contrast: 50                    # 0..=100 (default 50)
      fade: 0                         # 0..=100 towards background (default 0)
      transparent: true               # honour PNG/GIF/TIFF alpha (default true)
      show: true                      # draw image pixels (default true)
      clip: [[12, 8, 0], [47, 8, 0], [47, 23, 0], [12, 23, 0]]  # optional >=3-vertex polygon
      clip_inverted: false            # invert clip boundary (default false)
      layer: "0"
```

### PDF underlay (`pdf_underlay`)

```yaml
objects:
  - pdf_underlay:
      id: redline
      path: markups/site-markup.pdf
      page: "1"                       # 1-based page number (default "1")
      position: [0, 0, 0]             # lower-left corner in sheet units
      scale: 1.0                      # multiplier on PDF physical size (default 1.0)
      rotation: 0                     # anticlockwise degrees (default 0)
      contrast: 100                   # 0..=100 (default 100)
      fade: 0                         # 0..=80 towards background (default 0)
      monochrome: false               # grayscale rendering (default false)
      show: true                      # draw underlay (default true)
      unloaded: false                 # keep definition without loading (default false)
      clip: [[10, 10, 0], [200, 10, 0], [200, 140, 0], [10, 140, 0]]
      clip_inverted: false
      layer: "0"
```

Both `image` and `pdf_underlay` may also appear inside block definitions in a block library (for example, a raster company logo inside a title block).

## Drawing-wide frame (`image_frame`)

`image_frame` controls whether raster image and PDF underlay borders draw and plot, matching AutoCAD's `IMAGEFRAME` (`RASTERVARIABLES` in `ACAD_IMAGE_VARS`):

- `off` (default, code `0`): frames neither display nor plot.
- `on` (code `1`): frames display and plot.
- `display` (code `2`): frames display on screen (viewer / display list / raster render) and do not plot to PDF.

## Resolution, missing files and stamps

External image and PDF paths are resolved relative to the host sheet/DWG/DXF folder (and, for attachments inside a block library, the library's folder), with fallback to the plain file name beside the host folder so drawings moved together with their attachments still resolve, as AutoCAD does.

- **Missing or unreadable attachments never abort** `read`, `render`, `plot`, `display-list`, `query`, or `check`:
  - They are reported under `unresolved_media` and in top-level `warnings` with code `unresolved_media`.
  - Unresolved or unloaded attachments draw as a boundary outline frame so their placement remains visible.
- **Stale tracking**: every resolved image and PDF file read from disk is hashed into PNG and PDF output stamps (`source` kind `image` / `pdf_underlay`), so `agentdrafter stale` flags outputs whenever an attached image or PDF changes on disk.

## Single drawing pipeline (ADR-0011)

Image decoding (`image` crate: PNG, JPEG, BMP, GIF, TIFF) and PDF page rasterisation (pure-Rust `hayro`, transparent page background, capped at 150 DPI / 2048 px) happen at sheet load time outside the drawing pipeline. The display list carries canonical straight-RGBA8 PNG bytes in `DisplayList.images` and `Shape::Image` items in exact draw order:

- **PNG (`render`)**: composites adjusted RGBA pixels with bilinear filtering, affine rotation/scale, clip/inverted-clip masks, and viewport masks.
- **PDF (`plot`)**: embeds lossless FlateDecode RGB Image XObjects (plus `/SMask` alpha XObjects when transparent pixels exist) at exact physical coordinates with PDF clipping paths. Drawings with no images emit zero image XObjects, keeping existing PDFs byte-identical.
- **Browser viewer (WebGL2 / WASM)**: decodes the embedded PNG bytes in WebAssembly via `png`, applies brightness/contrast/fade/monochrome/transparency adjustments, uploads `WebGlTexture`s, and draws textured quads in draw order with stencil clipping.

## DWG and DXF representation

- **Raster images** write genuine AutoCAD `IMAGE` entities backed by `IMAGEDEF` (in the root dictionary's `ACAD_IMAGE_DICT`) and `IMAGEDEF_REACTOR` (with `owner = entity_handle`), plus `RASTERVARIABLES` (`ACAD_IMAGE_VARS`). Pixel-space polygonal clip boundaries round-trip faithfully. (`clip_inverted: true` renders, plots, and reads from existing AutoCAD DWGs/DXFs; writing the inverted-clip bit on newly built `IMAGE` entities awaits an upstream `acadrust` writer fix.)
- **PDF underlays** write genuine AutoCAD `PDFUNDERLAY` entities backed by `PDFDEFINITION` (in the root dictionary's `ACAD_PDFDEFINITIONS`), carrying `page`, `scale`, `rotation`, `contrast`, `fade`, `monochrome`, `clip`, and `clip_inverted`.
- Both round-trip through DWG 2018, ASCII DXF, and binary DXF, and are preserved untouched when revising existing plant DWGs (ADR-0004).

## Change-set operations

Every interface (CLI, HTTP API, MCP server, Python library) exposes:

- `add_image` (AutoCAD aliases: `IMAGEATTACH`, `IMAGE`)
- `add_pdf_underlay` (AutoCAD aliases: `PDFATTACH`, `PDFUNDERLAY`)

Both accept snapped `position` and `clip` points and optional `draw_order` (for example `"back"` to place a PDF markup underlay behind drawing geometry), and work on both drawing sets and original DWGs. Existing `move`, `rotate`, `scale`, `draw_order`, and `delete` operations also work on attached images and PDF underlays, with full journal undo.
