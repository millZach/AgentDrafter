# The viewer

The viewer is the browser window where the reviewer looks at a drawing set (see [`CONTEXT.md`](../CONTEXT.md)). This first part draws a set's sheets in the page, with the same Rust drawing pipeline that makes the PNG and the PDF, compiled to WebAssembly, with TypeScript and React around it (ADR-0011); the reviewer pans and zooms on a laptop and on a phone. Watching change sets land, the journal, Undo and Accept come next (#18).

## Opening it

`agentdrafter serve` serves the viewer at `/`, beside the HTTP API ([servers.md](servers.md)). Start it in a drawing set's folder and open the address its start line gives under `viewer`:

```
cd examples/demo-plant
agentdrafter serve
{"ok":true,"result":{"address":"127.0.0.1:7316","listening":"http://127.0.0.1:7316","local_only":true,"token_required":false,"viewer":"http://127.0.0.1:7316/"}}
```

The page shows the drawing set in the folder the server started in. Its address can name another, and a sheet: `/?source=examples/demo-plant&sheet=DP-LD-102`, where `source` is a drawing set's folder, a sheet description or a DWG, as the server's folder names it, and `sheet` one of its sheets (the first without it). The address keeps the sheet shown, so a reload shows it again.

**From another machine or a phone**, such as over the tailnet, the server listens on an address beyond the local machine and needs its token:

```
agentdrafter serve --listen 100.101.102.103:7316
{"ok":true,"result":{"address":"100.101.102.103:7316", ... ,"token_required":true,"viewer":"http://100.101.102.103:7316/#token=3f9c…","token":"3f9c…"}}
```

Open the `viewer` address as printed. Listening on `0.0.0.0` prints `http://0.0.0.0:7316/#token=…`: put the machine's own name or address in its place.

## The token

A server listening beyond the local machine refuses every command without its token (`Authorization: Bearer <token>`). The viewer gets it this way:

- The start line's `viewer` address carries the token in its fragment, after `#`. A browser never sends a fragment to a server, so it reaches no server log, no proxy and no `Referer`.
- The page takes it from the fragment once, keeps it in the tab's session storage, and takes it off the address shown with `history.replaceState`, so it is not left in the address bar, the history or a link copied from it. A reload in the same tab keeps working; a new tab needs the address again; the server makes a new token each time it starts.
- The page sends it as `Authorization: Bearer` with every command, and never in a query string. A token in a query string is not taken.
- The start line is the only line the server writes the token in.

The page's own files are served without the token, since a browser cannot send one to open a page: `GET /` (or `/index.html`) and `GET /assets/<file>`, the files `npm run build` wrote, embedded in the binary. They are the page's code and nothing else, no drawing and nothing of the machine's; every drawing comes from a command, which needs the token. Only `GET` and `HEAD` reach them. On the local machine they are served, as commands are, only to a request addressed to the local machine by its `Host` and any `Origin`; the guards of the HTTP API (`Host`, `Origin`, JSON bodies) stay as they are for every command. The page is sent with a Content-Security-Policy that lets it run its own scripts and WebAssembly and talk to its own server only (`default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`), with `Referrer-Policy: no-referrer`. `index.html` is never cached; the assets, whose names carry a fingerprint of their bytes, are cached for good.

## What it draws, and how

The engine stays on the machine that holds the drawings. The page asks it for a sheet with the `display-list` command, which reads the record and lays the sheet out, text in that machine's fonts, and replies with the sheet's display list: the shapes the PNG and the PDF draw from, as JSON, with the set's sheets, the paper and how many millimetres of paper a drawing unit spans. A demo plant loop sheet's reply is about 290 KB: 116 items, 5,582 segments. The page keeps each sheet it has drawn, so going back to a tab asks the server nothing.

The page's renderer, `crates/agentdrafter-viewer`, is Rust compiled to WebAssembly. It reads the reply with the core's own types and draws it with WebGL2:

- **Strokes**: each segment of each stroke is a quad round its pen's capsule, and the fragment shader works out each pixel's distance from the segment, so ends and corners are round and every pen is exactly as wide as the PNG draws it, anti-aliased. All of a sheet's segments draw in one draw call.
- **Fills** (solid hatches, TrueType text) are cut into triangles once, by their fill rule, with [lyon](https://crates.io/crates/lyon_tessellation), and drawn in one draw call; the canvas's multisampling smooths their edges.
- **Colours** are the core's own: each colour index in the colour it plots in on paper, index 7 black, as the PNG draws them.
- **Pens are widths on paper**, as in the PNG and the PDF: a 0.5 mm pen is twice as wide when the sheet is shown twice as large. The finest pen is one CSS pixel wide, as a PNG's finest is one pixel.
- **Ink only darkens the paper.** Each pixel takes the darkest ink drawn on it (WebGL's `MIN` blending), so strokes that meet do not darken where their edges overlap, and the shapes' order does not matter. Where two colours cross, the darker shows, where the PNG shows the later one.
- **The paper** is the sheet's declared paper, white, with a shadow, on the page's dark background; a sheet with no paper, such as a DWG, sits on its extents with the 5 mm margin a render leaves.
- **Panning and zooming** change two numbers the GPU is given; the sheet is laid out once, when it loads. Curves are drawn within a 100,000th of the sheet's larger side, finer than a pixel at the closest zoom.

It needs WebGL2 and nothing newer. It never asks for WebGPU, which Linux's system webview lacks and iPhones have only from iOS 26. A browser without WebGL2 shows that it has none.

## Pan and zoom

As the picked prototype does:

- **Touch**: one finger drags the sheet and flings it on release; two fingers pinch, keeping the point between them under them, and pan as they move; a double tap zooms in where it lands.
- **Mouse and trackpad**: drag to pan; the wheel zooms where the pointer is, as AutoCAD's wheel does; a trackpad's pinch zooms faster; a double click zooms in.
- **Buttons**: zoom in, zoom out and fit at the top right; a phone shows only fit.

The sheet can zoom out to half its fitted size and in to 32 times it, and is never flung out of sight.

## The look

The look follows variant A of the prototype the reviewer picked (#16): the drawing fills the screen, the set's sheets are tabs across the top, and the journal's sheet peeks up from the bottom on a phone, dragging up to half or nearly all of the screen, and is a panel on the right on a laptop (900 CSS pixels wide or more). Undo last and Accept are in the peek, always in reach. Until #18 they hold their places switched off, and the journal's sheet says what is coming; nothing in it does anything yet.

## Building it

The page lives in `viewer/`: TypeScript and React, built with Vite. `npm run build` there builds the renderer for WebAssembly (`cargo build -p agentdrafter-viewer --target wasm32-unknown-unknown --profile wasm-release`), makes its JavaScript glue with `wasm-bindgen`, checks the types, and writes the page into `viewer/dist/`. It needs Node 24 or later, the Rust target `wasm32-unknown-unknown` (`rustup target add wasm32-unknown-unknown`) and `wasm-bindgen-cli` of the version `crates/agentdrafter-viewer` pins (`cargo install wasm-bindgen-cli --version 0.2.130 --locked`, or `WASM_BINDGEN` naming the binary).

```
cd viewer
npm ci
npm run build
cd ..
cargo build -p agentdrafter-cli
```

**How the page reaches the binary.** `crates/agentdrafter-cli/build.rs` embeds every file of `viewer/dist/` in `agentdrafter` when it has been built. The built page is not committed: CI's `Viewer` workflow builds it and tests the binary with it. A plain `cargo build` on a machine without Node works: with no `viewer/dist/`, the binary carries a one-page note in its place that says how to build the viewer, and serves every command as before. This keeps generated WebAssembly and JavaScript out of the repository and out of every pull request's diff, and the required CI jobs need no Node.

`npm run dev` serves the page from Vite on port 5173 and sends its commands on to `agentdrafter serve` on `127.0.0.1:7316`, for working on the page.

The page's packages, all permissive or MPL-2.0 (ADR-0010): `react`, `react-dom` and `scheduler` (MIT) go into the page; `vite` (MIT) with `rolldown`, `postcss`, `lightningcss` (MPL-2.0) and their helpers, `typescript` (Apache-2.0), `@types/*` (MIT) and `playwright-core` (Apache-2.0) only build and test it. The renderer's crates: `lyon_tessellation`, `lyon_path`, `lyon_geom`, `euclid` (MIT or Apache-2.0), `float_next_after` and `libm` (MIT), and `wasm-bindgen`, `js-sys` and `web-sys` (MIT or Apache-2.0).

## Tests

- `cargo test -p agentdrafter-viewer`: the mesh, natively: strokes into segments with their pens and colours on paper, fills into triangles by their rule, and a demo plant sheet read from the core's own reply.
- `npm test` in `viewer/`: the view's arithmetic (fit, zoom about a point, pinch, keeping the sheet in sight), taking the token from the fragment and sending it as a bearer, and the image comparison below.
- `node e2e/run.mjs` in `viewer/` (after `npm run build` and `cargo build -p agentdrafter-cli`): the page in headless Chromium against `agentdrafter serve`, each check starting and stopping its own server. `--chrome` names the browser; `--gl gpu` draws with the machine's GPU instead of software WebGL. It writes `results.json`, the screenshots and the compared images into `target/viewer-e2e/`.
  - `png`: the canvas matches its PNG (below).
  - `token`: a server listening on `0.0.0.0` serves the page and its files without the token and refuses commands without it, with another or with it in a query string; the page with no token says how to open it; from the printed address it draws, takes the token off the address, keeps it for the tab and still draws after a reload; the server printed one line.
  - `frames`: frame times at phone size (below).
  - `webgl2`: the built page never names WebGPU, and the viewer draws in a browser with WebGPU switched off and `navigator.gpu` taken away; in a browser with no WebGL at all, the page says it cannot draw there, and why.
  - `mouse`: the wheel zooms, a drag pans, a double click zooms in, fit and the zoom buttons work, and a tab shows its sheet and names it in the address.
  - `look`: screenshots at phone size (390 by 844 CSS pixels at 3 device pixels each, touch) and laptop size (1440 by 900).

### Measured against the PNG

The canvas is drawn at the view and size of `agentdrafter render --scale 1 --dpi <dpi>` of the same sheet, at one device pixel to a CSS pixel, white round the paper as a PNG's paper is, and read back. A demo plant sheet in the viewer is the set's sheet; its PNG is rendered from its DWG, which draws the same display list point for point. For every inked pixel of each image (its darkest channel below 192), the nearest pixel of the other image darker than 224 is looked for, within one pixel and within two, as ticket #11 measured the PNG against AutoCAD. The tolerance the test holds: 99.9% of each image's ink within one pixel of the other's, all of it within two, at most 0.2% of pixels differing by more than 64 levels in any channel, and a mean difference under 2 levels in 255. Software WebGL in headless Chromium on forge:

| Sheet | dpi | Image | PNG's ink within 1 px of the viewer's | Viewer's ink within 1 px of the PNG's | Pixels differing by more than 64 | Mean difference (of 255) |
|---|---|---|---|---|---|---|
| demo plant `DP-LD-101` | 100 | 1740 by 1140 | 100% | 100% | 0.038% | 0.52 |
| demo plant `DP-LD-301` | 200 | 3479 by 2279 | 100% | 100% | 0.015% | 0.23 |
| `examples/geometry.yaml` | 150 | 1506 by 969 | 100% | 100% | 0.034% | 0.23 |
| `examples/hatch.yaml` | 150 | 1359 by 1241 | 100% | 100% | 0.006% | 0.19 |
| `examples/text.yaml` | 150 | 1478 by 789 | 100% | 100% | 0.025% | 0.36 |

What still differs: anti-aliasing. tiny-skia covers each edge pixel by the area the shape covers, the viewer by its distance from the pen's centre line and, for fills, by four samples a pixel; and where a red and a black stroke cross, the viewer shows black where the PNG shows the later.

### Smooth on a phone

The `frames` check opens the demo plant's `DP-LD-101` (5,582 segments) at phone size with touch and the CPU throttled four times (Chrome's `Emulation.setCPUThrottlingRate`), then pinches out with two fingers, drags with one and lets go moving, and pinches in, through Chrome's own touch events, while every animation frame's time is kept. It holds 95% of frames within 20 ms, 99% within 34 ms (one refresh late at most) and none over 100 ms, and the point under the fingers under them. On forge, while other work kept its load average near 9:

| WebGL | Runs | Frames a run | Median | 95% within | 99% within | Longest | Over 33.3 ms | The page's draw per frame (median, 95%) | Drift under the fingers |
|---|---|---|---|---|---|---|---|---|---|
| Software (SwiftShader), as CI's runners have | 6 | 342 to 355 | 16.7 ms | 16.8 ms | 33.3 to 33.4 ms | 33.4 ms in four runs, 66.7 and 66.8 ms in two | 1 to 5 | 0.1 to 0.3 ms, 0.8 to 0.9 ms | 0 px |
| forge's GPU (RTX 4060, through Vulkan) | 2 | 355 and 356 | 16.7 ms | 16.8 ms | 16.8 ms | 16.8 ms | 0 | 0.1 ms, 0.7 to 0.8 ms | 0 px |

A frame of 16.7 ms is the display's 60 a second. Drawing each segment as an instance of one quad cost software WebGL about 33 ms a frame, which drawing the quads as plain triangles brought under a frame. A real phone has a GPU of its own; how it feels in the hand is the reviewer's to judge.
