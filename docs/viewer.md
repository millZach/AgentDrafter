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

- **In the display list's order**, as the PNG paints it: a later item covers an earlier one, a blue line over a red one where they cross, a fill over the line under it. Nothing is blended. Items next to one another that draw strokes, or fills, are drawn by one draw call, run after run, and the GPU applies each draw's triangles, and each draw, in order: a demo plant loop sheet, all strokes, is one draw call. A sheet whose strokes and fills alternate more than 64 times is drawn instead in two draws, every item at a depth by its place in the list, a later one nearer, so the depth test keeps the later item's colour in every sample.
- **Strokes**: each segment of each stroke is a quad exactly as wide as its pen and long enough for its round ends, so every pen is as wide as the PNG draws it. The samples the quad's sides cover are the pen's; at a round end the fragment shader works out the share of the pixel the pen covers, by its distance, and that share of the pixel's samples is written (alpha to coverage). Each sample is written opaque, so where one stroke's segments overlap at a corner they cover the same samples once, and the canvas's multisampling, four samples a pixel, smooths the edges.
- **Fills** (solid hatches, TrueType text) are cut into triangles once, by their fill rule, with [lyon](https://crates.io/crates/lyon_tessellation); the multisampling smooths their edges.
- **Colours** are the core's own: each colour index in the colour it plots in on paper, index 7 black, as the PNG draws them.
- **Pens are widths on paper**, as in the PNG and the PDF: a 0.5 mm pen is twice as wide when the sheet is shown twice as large. The finest pen is one CSS pixel wide, as a PNG's finest is one pixel.
- **The paper** is the sheet's declared paper, white, with a shadow, on the page's dark background; a sheet with no paper, such as a DWG, sits on its extents with the 5 mm margin a render leaves.
- **Panning and zooming** change two numbers the GPU is given; the sheet is laid out once, when it loads. Curves are drawn within a 100,000th of the sheet's larger side, finer than a pixel at the closest zoom.

It needs WebGL2 and nothing newer. It never asks for WebGPU, which Linux's system webview lacks and iPhones have only from iOS 26. A browser without WebGL2 shows that it has none.

## Pan and zoom

As the picked prototype does:

- **Touch**: one finger drags the sheet and flings it on release; two fingers pinch, keeping the point between them under them, and pan as they move; a double tap zooms in where it lands.
- **Mouse and trackpad**: drag to pan; the wheel zooms where the pointer is, as AutoCAD's wheel does; a trackpad's pinch zooms faster; a double click zooms in. Zach decided on 2026-11-11, after trying the viewer, that the wheel zooms.
- **Buttons**: zoom in, zoom out and fit at the top right; a phone shows only fit.

The sheet can zoom out to half its fitted size and in to 32 times it, and is never flung out of sight.

**The fit.** A sheet is fitted to the screen when it is first shown and on every change of tab: the whole paper, 10 pixels in from the edges, above a phone's bottom sheet. Until the reviewer pans or zooms, the view follows the fit: when the canvas changes size, as a browser may while it settles, or the pixel ratio changes, or the bottom sheet's peek does, the sheet is fitted again. Once the reviewer has moved it, a change of size keeps the point at the middle of the screen where it was, and the fit button fits it again. On a laptop the zoom buttons sit over the fitted sheet's top right corner, as they do in the picked prototype. A fit, or a change of size or tab that fits the sheet, stops a glide still running, so the glide's next frame cannot put back the view it was heading for.

**Line weights.** A line is drawn at its pen's width on paper, so it grows as the sheet is zoomed in, as the plot would look under a magnifier; the thinnest line is one CSS pixel, so it stays visible on a phone. Zach decided on 2026-11-11, after trying the viewer on his phone, to keep lines like paper rather than thin hairlines at every zoom.

## The look

The look follows variant A of the prototype the reviewer picked (#16): the drawing fills the screen, the set's sheets are tabs across the top, and the journal's sheet peeks up from the bottom on a phone, dragging up to half or nearly all of the screen, and is a panel on the right on a laptop (900 CSS pixels wide or more). Undo last and Accept are in the peek, always in reach. Until #18 they hold their places switched off, and the journal's sheet says what is coming; nothing in it does anything yet.

A tab shows its sheet at once when the page has drawn it before; the server is asked for each sheet once. Going back to a sheet already drawn shows it with nothing said of the sheet asked for meanwhile, whether that one is still coming (its reply is kept for later) or failed.

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

- `cargo test -p agentdrafter-viewer`: the mesh, natively: strokes into segments with their pens and colours on paper, fills into triangles by their rule, each with its item's place, the runs of one kind in order, and a demo plant sheet read from the core's own reply.
- `npm test` in `viewer/`: the view's arithmetic (fit, zoom about a point, pinch, keeping the sheet in sight), taking the token from the fragment and sending it as a bearer, and the image comparison below.
- `node e2e/run.mjs` in `viewer/` (after `npm run build` and `cargo build -p agentdrafter-cli`): the page in headless Chromium against `agentdrafter serve`, each check starting and stopping its own server. `--chrome` names the browser; `--gl gpu` draws with the machine's GPU instead of software WebGL. It writes `results.json`, the screenshots and the compared images into `target/viewer-e2e/`.
  - `png`: the canvas matches its PNG, in ink and in colour (below).
  - `token`: a server listening on `0.0.0.0` serves the page and its files without the token and refuses commands without it, with another or with it in a query string; the page with no token says how to open it; from the printed address it draws, takes the token off the address, keeps it for the tab and still draws after a reload; the server printed one line.
  - `frames`: draws and frame times at phone size (below).
  - `webgl2`: the built page never names WebGPU, and the viewer draws in a browser with WebGPU switched off and `navigator.gpu` taken away; in a browser with no WebGL at all, the page says it cannot draw there, and why.
  - `mouse`: the wheel zooms, a drag pans, a double click zooms in, fit and the zoom buttons work, and a tab shows its sheet and names it in the address.
  - `fit`: the first load shows the whole sheet, at phone and laptop sizes, and so does another tab; a page laid out at 1000 by 700 and then narrowed to 390 by 844, and then given 3 device pixels to a CSS pixel, is fitted to each; a view the reviewer zoomed keeps its scale through a change of size; and a change of size during the fit button's glide, from phone to laptop width, ends fitted to the new size.
  - `tabs`: with the server's replies held back or failed at will, going back to a sheet already drawn while another is still coming, or after another failed, shows it and says nothing more; the late reply changes nothing; a sheet drawn before is not asked for again.
  - `look`: screenshots at phone size (390 by 844 CSS pixels at 3 device pixels each, touch) and laptop size (1440 by 900).

### Measured against the PNG

The canvas is drawn at the view and size of `agentdrafter render --scale 1 --dpi <dpi>` of the same sheet, at one device pixel to a CSS pixel, white round the paper as a PNG's paper is, and read back. A demo plant sheet in the viewer is the set's sheet; its PNG is rendered from its DWG, which draws the same display list point for point. `viewer/e2e/overlap.yaml` is a made-up sheet of colours drawn over one another: a blue line over a red one, a black line over both, a green fill over the three, a red line over that and a yellow fill over part of it.

For every inked pixel of each image (its darkest channel below 192), the nearest pixel of the other image darker than 224 is looked for, within one pixel and within two, as ticket #11 measured the PNG against AutoCAD; and the nearest in the same colour, no channel more than 96 levels off. That is more than anti-aliasing moves an edge pixel, and less than red, blue or green differ from black, or from one another, even at half a pixel's coverage.

The tolerance the test holds:

- 99.9% of each image's ink within one pixel of the other's, and all of it within two;
- 99.7% of it within one pixel in the same colour;
- at most 0.5% of pixels differing by more than 64 levels in any channel;
- a mean difference under 2 levels in 255.

Software WebGL in headless Chromium on forge:

| Sheet | dpi | Image | Ink within 1 px, PNG's and viewer's | In its colour within 1 px, PNG's and viewer's | Pixels differing by more than 64 | Mean difference (of 255) |
|---|---|---|---|---|---|---|
| demo plant `DP-LD-101` | 100 | 1740 by 1140 | 100%, 100% | 100%, 99.83% | 0.22% | 0.58 |
| demo plant `DP-LD-301` | 200 | 3479 by 2279 | 100%, 100% | 99.98%, 100% | 0.028% | 0.26 |
| `examples/geometry.yaml` | 150 | 1506 by 969 | 100%, 100% | 100%, 100% | 0.068% | 0.18 |
| `examples/hatch.yaml` | 150 | 1359 by 1241 | 100%, 100% | 99.99%, 100% | 0.037% | 0.26 |
| `examples/text.yaml` | 150 | 1478 by 789 | 100%, 100% | 99.97%, 100% | 0.088% | 0.47 |
| `viewer/e2e/overlap.yaml` | 150 | 650 by 414 | 100%, 100% | 100%, 100% | 0.076% | 0.23 |

The viewer of the pull request's first commit took each pixel's darkest ink. On the overlap sheet it kept only 94.6% and 93.4% of the ink in its colour, and on `geometry.yaml` 99.5%, so this test fails it.

What still differs: anti-aliasing. tiny-skia covers each edge pixel by the area the shape covers; the viewer by four samples a pixel, so an edge the PNG covers by a third the viewer may cover by a quarter or a half. That puts DP-LD-101's 0.7 mm border, 2.76 pixels wide at 100 dpi, at 127 in the PNG and 128 or 192 in the viewer at its edges.

### Smooth on a phone

The `frames` check opens the demo plant's `DP-LD-101` (5,582 segments, one draw call) at phone size with touch and the CPU throttled four times (Chrome's `Emulation.setCPUThrottlingRate`). Through Chrome's own touch events, a finger moving every 8 ms, it pinches out with two fingers, drags with one and lets go moving, and pinches in. It keeps the time of every display frame and of every draw of the canvas in each gesture, and how long each draw call took. It holds:

- every gesture draws the canvas at least 20 times;
- the page's own draw takes a median of 1 ms at most and 95% within 4 ms;
- the point under the fingers stays under them.

With the GPU (`--gl gpu`) it also holds:

- the canvas drawn on at least 60% of each gesture's frames;
- 95% of the times between draws, and between frames, within 20 ms, 99% within 34 ms (one refresh late at most), and none over 100 ms.

In software WebGL (the default without a GPU, and CI's runner) filling the phone's 1170 by 2532 multisampled canvas on the CPU, beside Chrome's own handling of the test's touches, is slower than a phone's GPU, so those times are reported, not held. On forge:

| WebGL | Draws in each gesture's frames (pinch out, drag and fling, pinch in) | Between frames: median, 95%, 99%, longest | Between draws: median, 95%, 99%, longest | The page's draw: median, 95% | Drift under the fingers |
|---|---|---|---|---|---|
| forge's GPU (RTX 4060, through Vulkan), two runs | 90 of 93, 83 of 93, 77 of 78 | 16.7, 16.7, 16.8, 16.8 ms | 16.7, 17.5 to 17.7, 33.5 to 33.7, 33.6 to 33.9 ms | 0.1 ms, 0.7 to 0.8 ms | 0.66 px |
| Software (SwiftShader) | 90 of 108, 77 of 95, 78 of 90 | 16.7, 33.3, 50, 50.1 ms | 21.8, 36.1, 68.7, 78.1 ms | 0.2 ms, 0.9 ms | 0.66 px |

A frame of 16.7 ms is the display's 60 a second. On the GPU, the canvas is redrawn on nearly every frame of every gesture, and a draw comes a frame late only where the test's next touch did. A real phone has a GPU of its own; how it feels in the hand is the reviewer's to judge.
