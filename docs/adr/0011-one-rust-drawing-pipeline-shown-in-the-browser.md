# One Rust drawing pipeline, shown in the browser through WebAssembly, with React around it

The viewer opens in a browser on the laptop, on forge and on a phone, and the later drafting app has to grow from it. The drawing is drawn by Rust code compiled to WebAssembly: the same pipeline that makes the PNG and the PDF, so the screen and the plot agree. Panels, lists and buttons are TypeScript and React. The engine stays on the machine that holds the drawings and streams changes to the page. The drafting app, later, is the same page in a desktop shell.

## Considered options

- **An all-Rust UI (egui or iced)**, the route Open CAD Studio took. Rejected: typing and touch on a phone are weaker, and both toolkits described themselves as unfinished in November 2026.
- **A TypeScript renderer on WebGL.** The fastest route to a first viewer. Rejected: it is a second renderer beside the Rust one that makes PNG and PDF, so the screen and the plot could disagree, and the drafting app would rest on it.
- **The engine sends images.** One renderer and the simplest page. Rejected: every pan, zoom and pick is a round trip, and it is not a base for a drafting app.
- **A Rust renderer in the page with React around it (chosen).**

## Consequences

- Text, linetypes and hatches are laid out once, in Rust, for every output.
- The page carries a WebAssembly module, so its first download is larger than a plain web page. The module holds the renderer, not the whole engine.
- WebGPU is not everywhere: Linux's system webview has none, and iPhones have it from iOS 26. The renderer needs a WebGL2 path.
- Which desktop shell wraps the page for the drafting app is decided when that app is designed.
- How the viewer looks is chosen from a prototype on a phone before it is built.
