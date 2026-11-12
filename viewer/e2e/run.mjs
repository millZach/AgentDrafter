// The viewer's browser tests, against `agentdrafter serve` and the page it
// serves, in headless Chromium. Each check starts the servers it needs and
// stops them before it ends.
//
//   node e2e/run.mjs [--bin <agentdrafter>] [--chrome <chromium>] [--out <folder>] [--only <check>,...]
//                    [--gl software|gpu] [--throttle <rate>] [--smooth frames|draw]
//
// - png:     the canvas, drawn at a render's view and size, against the PNG
//            `agentdrafter render` draws of the same sheet at scale 1;
// - token:   a server listening beyond the local machine serves the page
//            without the token, refuses commands without it, and the page
//            takes it from the address's fragment;
// - frames:  frame times while pinching and panning a demo plant loop sheet
//            with touch, at phone size with the CPU throttled 4 times;
// - webgl2:  the viewer draws where there is WebGL2 and no WebGPU at all;
// - mouse:   wheel, drag, double click, the zoom buttons and the sheet tabs;
// - fit:     the first load, and a change of size or pixel ratio, shows the
//            whole sheet, at phone and laptop sizes;
// - tabs:    back to a sheet already drawn shows it, whatever the sheet
//            asked for meanwhile is doing;
// - look:    screenshots at phone and laptop sizes;
// - live:    a change set the command line sends, in another process,
//            shows in the open page without a reload, named by a toast,
//            its object marked, on the sheet shown and on another;
// - journal: stepping back through the journal shows the set as it was at
//            each entry, read-only; an entry whose undo state is pruned
//            says it cannot be shown; back to live returns;
// - pick:    a tap on an insert, at phone size, shows its ID, name, layer
//            and attributes in the inspector;
// - undo:    Undo last undoes the newest change set, as one;
// - accept:  refused while the set is inconsistent; warns of failed checks
//            and lists them; then issues the change sets, which can no
//            longer be undone, and the journal says the viewer accepted;
// - late:    a sheet's reply that comes after a change to that sheet landed
//            is not kept: going back to the sheet asks for it again;
// - rewind:  a journal cut back, as a commit rolled back cuts it, drops
//            the drawings kept and draws the set as it is;
// - follow:  a picked object stays picked as objects before it are
//            deleted, and the pick clears when it is deleted itself.
//
// It writes results.json, the screenshots and each compared image into
// --out (target/viewer-e2e by default), and exits 1 when any check fails.

import { execFileSync, spawn } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';
import { chromium } from 'playwright-core';

import { compare, decodePng, encodePng } from './compare.mjs';

const viewer = dirname(dirname(fileURLToPath(import.meta.url)));
const root = dirname(viewer);
const argv = process.argv.slice(2);
const option = (name) => {
  const at = argv.indexOf(name);
  return at >= 0 ? argv[at + 1] : undefined;
};
const exe = process.platform === 'win32' ? '.exe' : '';
const bin = resolve(option('--bin') ?? process.env.AGENTDRAFTER ?? join(root, 'target', 'debug', `agentdrafter${exe}`));
const chrome =
  option('--chrome') ??
  process.env.CHROME ??
  ['/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome', '/usr/bin/google-chrome-stable'].find(
    existsSync,
  );
const out = resolve(option('--out') ?? join(root, 'target', 'viewer-e2e'));
const only = option('--only')?.split(',');
if (!existsSync(bin)) throw new Error(`no agentdrafter at ${bin}: build it, or name it with --bin`);
if (!chrome) throw new Error('no Chromium found: name it with --chrome or CHROME');

// `--gl software` (the default) draws with software WebGL, as a machine
// without a GPU has, so every run draws alike; `--gl gpu` with the machine's
// GPU through Vulkan, as a phone draws with its own.
const GL = option('--gl') ?? 'software';
const BROWSER_ARGS = [
  '--no-sandbox',
  ...(GL === 'gpu'
    ? ['--use-angle=vulkan', '--enable-features=Vulkan', '--ignore-gpu-blocklist']
    : ['--use-angle=swiftshader', '--enable-unsafe-swiftshader']),
];
const THROTTLE = Number(option('--throttle') ?? 4);
// What the `frames` check holds besides that every gesture is drawn and
// the page's own draw is small: `frames`, also the times between draws and
// between the display's frames, by default with `--gl gpu`; or `draw`,
// nothing more, by default with software WebGL, where filling the phone's
// canvas on the CPU, beside Chrome's own handling of the test's touches,
// is slower than any phone's GPU. The times are reported either way.
const SMOOTH = option('--smooth') ?? (GL === 'gpu' ? 'frames' : 'draw');
const PHONE = { viewport: { width: 390, height: 844 }, deviceScaleFactor: Number(process.env.E2E_RATIO ?? 3), isMobile: true, hasTouch: true };
const LAPTOP = { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 };

rmSync(out, { recursive: true, force: true });
const scratch = join(out, 'scratch');
mkdirSync(scratch, { recursive: true });
cpSync(join(root, 'examples', 'demo-plant'), join(scratch, 'demo-plant'), {
  recursive: true,
  filter: (path) => !path.includes(`${join('demo-plant', 'dwg')}`),
});
for (const sheet of ['geometry.yaml', 'hatch.yaml', 'text.yaml']) {
  cpSync(join(root, 'examples', sheet), join(scratch, sheet));
}
cpSync(join(viewer, 'e2e', 'overlap.yaml'), join(scratch, 'overlap.yaml'));

/** Runs the command line in the scratch folder and gives its JSON reply. */
function agentdrafter(...args) {
  const line = execFileSync(bin, args, { cwd: scratch, encoding: 'utf8' });
  const reply = JSON.parse(line);
  if (!reply.ok) throw new Error(`agentdrafter ${args.join(' ')}: ${JSON.stringify(reply.error)}`);
  return reply.result;
}

/**
 * Runs the command line in `cwd`, as an agent in another process does, and
 * gives its whole reply, ok or not.
 */
function agentIn(cwd, ...args) {
  try {
    return JSON.parse(execFileSync(bin, args, { cwd, encoding: 'utf8' }));
  } catch (e) {
    // A reply that is not ok exits 1, its line still printed.
    return JSON.parse(String(e.stdout));
  }
}

/** Sends a change set from the command line in `cwd`, from a file, and gives its reply. */
function sendChange(cwd, changeSet) {
  const file = join(cwd, 'change-set.json');
  writeFileSync(file, JSON.stringify(changeSet));
  return agentIn(cwd, 'change', 'demo-plant', file);
}

/** A copy of the demo plant, built, for one check to change: its folder, which holds `demo-plant`. */
function freshPlant(name) {
  const folder = join(scratch, name);
  rmSync(folder, { recursive: true, force: true });
  cpSync(join(root, 'examples', 'demo-plant'), join(folder, 'demo-plant'), {
    recursive: true,
    filter: (path) => !path.includes(`${join('demo-plant', 'dwg')}`) && !path.includes('.agentdrafter'),
  });
  const built = agentIn(folder, 'build', 'demo-plant');
  if (!built.ok) throw new Error(`the demo plant did not build: ${JSON.stringify(built.error)}`);
  return folder;
}

/** Moves an object of the demo plant by `dx` inches. */
const move = (id, dx) => ({ operations: [{ op: 'move', target: { id }, by: [dx, 0, 0] }] });

/** The object at this ID on the sheet shown: its place and its box in drawing units. */
async function objectOf(page, id) {
  return page.evaluate((wanted) => {
    const probe = window.agentdrafterViewer;
    const place = probe.summary.objects.findIndex((o) => o.id === wanted);
    return { place, bounds: place >= 0 ? probe.bounds([place]) : [] };
  }, id);
}

/** Waits until the page's live stream is open. */
async function live(page) {
  await page.waitForSelector('.va-agent.working', { timeout: 30000 });
}

/** `agentdrafter serve` in the scratch folder, or `cwd`, with every line it printed. */
async function serve(listen, cwd = scratch) {
  const child = spawn(bin, ['serve', '--listen', listen], { cwd, stdio: ['ignore', 'pipe', 'pipe'] });
  const lines = [];
  const errors = [];
  child.stderr.on('data', (d) => errors.push(String(d)));
  const reader = createInterface({ input: child.stdout });
  const first = new Promise((done, fail) => {
    reader.on('line', (line) => {
      lines.push(line);
      if (lines.length === 1) done(line);
    });
    child.on('exit', (code) => fail(new Error(`serve stopped (${code}): ${errors.join('')}`)));
  });
  const started = JSON.parse(await first);
  if (!started.ok) throw new Error(`serve did not start: ${JSON.stringify(started)}`);
  const port = started.result.address.split(':').pop();
  return {
    started: started.result,
    lines,
    errors,
    base: `http://127.0.0.1:${port}`,
    async stop() {
      if (child.exitCode === null) {
        const gone = new Promise((done) => child.on('exit', done));
        child.kill();
        await gone;
      }
    },
  };
}

async function browser(extra = []) {
  return chromium.launch({ executablePath: chrome, args: [...BROWSER_ARGS, ...extra] });
}

/** Opens the viewer and waits until a sheet is drawn. */
async function open(context, url, sheet) {
  return openPage(await context.newPage(), url, sheet);
}

async function openPage(page, url, sheet) {
  const problems = [];
  page.on('pageerror', (e) => problems.push(`page error: ${e.message}`));
  page.on('console', (m) => m.type() === 'error' && problems.push(`console: ${m.text()}`));
  await page.goto(url);
  await page.waitForFunction(
    (wanted) => {
      const s = window.agentdrafterViewer?.summary;
      return s && (!wanted || s.sheet === wanted);
    },
    sheet ?? null,
    { timeout: 30000 },
  );
  // Two frames for the first draw to land.
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  return { page, problems };
}

const quantile = (sorted, q) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))];
const round = (n, places = 2) => Math.round(n * 10 ** places) / 10 ** places;
const stats = (values) => {
  // Nothing measured is not a measurement of nothing: it fails.
  assert(values.length > 0, 'there is nothing to measure');
  const sorted = [...values].sort((a, b) => a - b);
  return {
    count: sorted.length,
    median_ms: round(quantile(sorted, 0.5) ?? 0),
    p95_ms: round(quantile(sorted, 0.95) ?? 0),
    p99_ms: round(quantile(sorted, 0.99) ?? 0),
    max_ms: round(sorted.at(-1) ?? 0),
    over_33ms: sorted.filter((v) => v > 1000 / 30).length,
    over_50ms: sorted.filter((v) => v > 50).length,
  };
};
const assert = (ok, message) => {
  if (!ok) throw new Error(message);
};

// ---------------------------------------------------------------- checks

const checks = {
  /** The canvas against the PNG of the same sheet, at scale 1. */
  async png() {
    agentdrafter('build', 'demo-plant');
    const cases = [
      { name: 'DP-LD-101', source: 'demo-plant', sheet: 'DP-LD-101', png: 'demo-plant/dwg/DP-LD-101.dwg', dpi: 100 },
      { name: 'DP-LD-301', source: 'demo-plant', sheet: 'DP-LD-301', png: 'demo-plant/dwg/DP-LD-301.dwg', dpi: 200 },
      { name: 'geometry', source: 'geometry.yaml', png: 'geometry.yaml', dpi: 150 },
      { name: 'hatch', source: 'hatch.yaml', png: 'hatch.yaml', dpi: 150 },
      { name: 'text', source: 'text.yaml', png: 'text.yaml', dpi: 150 },
      // Colours drawn over one another, in order.
      { name: 'overlap', source: 'overlap.yaml', png: 'overlap.yaml', dpi: 150 },
    ];
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    const results = [];
    try {
      const context = await b.newContext(LAPTOP);
      for (const c of cases) {
        const file = join(out, `png-${c.name}.png`);
        const render = agentdrafter('render', c.png, '--output', file, '--overwrite', '--scale', '1', '--dpi', String(c.dpi));
        const query = new URLSearchParams({ source: c.source, ...(c.sheet ? { sheet: c.sheet } : {}) });
        const { page, problems } = await open(context, `${server.base}/?${query}`, c.sheet);
        const base64 = await page.evaluate(
          (view) => window.agentdrafterViewer.capture(view),
          { left: render.top_left[0], top: render.top_left[1], scale: render.pixels_per_unit, width: render.width, height: render.height },
        );
        await page.close();
        const drawn = new Uint8Array(Buffer.from(base64, 'base64'));
        const png = decodePng(readFileSync(file));
        assert(png.width === render.width && png.height === render.height, `${c.name}: the PNG is its reply's size`);
        writeFileSync(join(out, `viewer-${c.name}.png`), encodePng(drawn, render.width, render.height));
        const result = { case: c.name, png_source: c.png, dpi: c.dpi, ...compare(png.rgba, drawn, render.width, render.height) };
        results.push(result);
        assert(problems.length === 0, `${c.name}: ${problems.join('; ')}`);
      }
    } finally {
      await b.close();
      await server.stop();
    }
    // The tolerance (docs/viewer.md): 99.9% of each image's ink within one
    // pixel of the other's and all of it within two, 99.7% of it within one
    // pixel of the other's in the same colour; at most 0.5% of the pixels
    // differing by more than 64 levels in a channel, and the mean
    // difference under 2 levels in 255.
    for (const r of results) {
      for (const side of ['png', 'viewer']) {
        assert(r[`${side}_ink_within_1px`] >= 0.999, `${r.case}: ${side} ink within 1 px is ${r[`${side}_ink_within_1px`]}`);
        assert(r[`${side}_ink_within_2px`] >= 0.99999, `${r.case}: ${side} ink within 2 px is ${r[`${side}_ink_within_2px`]}`);
        assert(r[`${side}_colour_within_1px`] >= 0.997, `${r.case}: ${side} ink in its colour within 1 px is ${r[`${side}_colour_within_1px`]}`);
      }
      assert(r.pixels_differing_by_more_than_64 <= 0.005, `${r.case}: ${r.pixels_differing_by_more_than_64} of the pixels differ by more than 64`);
      assert(r.mean_channel_difference < 2, `${r.case}: the mean difference is ${r.mean_channel_difference}`);
    }
    return { cases: results };
  },

  /** Beyond the local machine: the page needs no token, commands do. */
  async token() {
    const server = await serve('0.0.0.0:0');
    const b = await browser();
    try {
      const { started } = server;
      assert(started.token_required === true && /^[0-9a-f]{64}$/.test(started.token), 'a token is made');
      assert(started.viewer.endsWith(`/#token=${started.token}`), 'the viewer address carries the token in its fragment');
      const page = await fetch(`${server.base}/`);
      const html = await page.text();
      assert(page.status === 200 && html.includes('<div id="app">'), 'the page is served without the token');
      assert(page.headers.get('content-security-policy')?.includes("script-src 'self' 'wasm-unsafe-eval'"), 'the page says what it may do');
      const asset = /src="(\/assets\/[^"]+\.js)"/.exec(html)?.[1];
      assert(asset && (await fetch(`${server.base}${asset}`)).status === 200, 'the page\'s files are served without the token');
      const command = (headers) =>
        fetch(`${server.base}/commands/display-list`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', ...headers },
          body: JSON.stringify({ source: 'demo-plant' }),
        });
      assert((await command({})).status === 401, 'a command without the token is refused');
      assert((await command({ Authorization: `Bearer ${'0'.repeat(64)}` })).status === 401, 'a command with another token is refused');
      assert((await command({ Authorization: `Bearer ${started.token}` })).status === 200, 'a command with the token runs');
      const inQuery = await fetch(`${server.base}/commands/display-list?token=${started.token}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ source: 'demo-plant' }),
      });
      assert(inQuery.status === 401, 'a token in the query string is not taken');

      // The page with no token says how to open it.
      const fresh = await b.newContext(LAPTOP);
      const bare = await fresh.newPage();
      await bare.goto(`${server.base}/?source=demo-plant`);
      await bare.waitForSelector('.va-note.failed');
      const said = await bare.textContent('.va-note.failed');
      assert(said.includes('#token='), `the page without the token says how to open it: ${said}`);
      await fresh.close();

      // The page from the address serve printed.
      const context = await b.newContext(LAPTOP);
      const address = started.viewer.replace(/^http:\/\/[^/]+/, server.base).replace('/#', '/?source=demo-plant#');
      const { page: viewerPage, problems } = await open(context, address, 'DP-LD-101');
      const after = await viewerPage.evaluate(() => ({
        href: location.href,
        kept: sessionStorage.getItem('agentdrafter-token'),
      }));
      assert(!after.href.includes(started.token), `the token is taken off the address: ${after.href}`);
      assert(after.kept === started.token, 'the token is kept for this tab');
      await viewerPage.click('.tab:nth-child(2)');
      await viewerPage.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-102');
      await viewerPage.reload();
      await viewerPage.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-102');
      assert(problems.length === 0, problems.join('; '));
      await context.close();
      assert(server.lines.length === 1, `serve printed only its start line: ${server.lines.length}`);
      assert(server.errors.join('') === '', 'serve printed nothing on standard error');
      return { token_in: 'the start line only', viewer: started.viewer.replace(started.token, '<token>') };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /** Frame times while a finger pinches and pans, at phone size, CPU throttled 4 times. */
  async frames() {
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    try {
      const context = await b.newContext(PHONE);
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
      const cdp = await context.newCDPSession(page);
      await cdp.send('Emulation.setCPUThrottlingRate', { rate: THROTTLE });
      await sleep(500);
      const box = await page.evaluate(() => {
        const r = document.querySelector('.stage').getBoundingClientRect();
        return { x: r.left, y: r.top, width: r.width, height: r.height };
      });
      const summary = await page.evaluate(() => window.agentdrafterViewer.summary);
      await page.evaluate(() => {
        window.__frames = [];
        window.__stop = false;
        const tick = (t) => {
          window.__frames.push(t);
          if (!window.__stop) requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
        window.agentdrafterViewer.takeDraws();
      });
      const touch = (type, points) =>
        cdp.send('Input.dispatchTouchEvent', {
          type,
          touchPoints: points.map(([x, y], id) => ({ x, y, id, radiusX: 2, radiusY: 2, force: 1 })),
        });
      const toDrawing = (v, sx, sy) => [v.left + (sx - box.x) / v.scale, v.top - (sy - box.y) / v.scale];
      const toPage = (v, x, y) => [box.x + (x - v.left) * v.scale, box.y + (v.top - y) * v.scale];
      const view = () => page.evaluate(() => window.agentdrafterViewer.view());
      const now = () => page.evaluate(() => performance.now());
      const cx = box.x + box.width / 2;
      const cy = box.y + box.height * 0.4;
      // Each gesture's span of the page's clock. A finger moves every 8 ms,
      // faster than the display's frames, so every frame has a move to draw.
      const phases = [];
      const gesture = async (name, run) => {
        const began = await now();
        await run();
        phases.push({ name, began, ended: await now() });
      };

      // Two fingers spread apart, their middle drifting left.
      const before = await view();
      const pair = (d, dx) => [
        [cx - d / 2 + dx, cy],
        [cx + d / 2 + dx, cy],
      ];
      let pinched;
      await gesture('pinch out', async () => {
        await touch('touchStart', pair(80, 0));
        for (let i = 1; i <= 90; i++) {
          await touch('touchMove', pair(80 + i * 2, -i * 0.66));
          await sleep(8);
        }
        pinched = await view();
        await touch('touchEnd', []);
      });
      const under = toDrawing(before, cx, cy);
      const [ux, uy] = toPage(pinched, under[0], under[1]);
      const drift = Math.hypot(ux - (cx - 90 * 0.66), uy - cy);
      // One finger drags, and lets go moving: the sheet glides on.
      await gesture('drag and fling', async () => {
        await touch('touchStart', [[cx, cy]]);
        for (let i = 1; i <= 60; i++) {
          await touch('touchMove', [[cx - i * 2, cy - i * 1.3]]);
          await sleep(8);
        }
        await touch('touchEnd', []);
        await sleep(400);
      });
      const panned = await view();
      // Two fingers close in.
      await gesture('pinch in', async () => {
        await touch('touchStart', pair(260, 0));
        for (let i = 1; i <= 75; i++) {
          await touch('touchMove', pair(260 - i * 2, 0));
          await sleep(8);
        }
        await touch('touchEnd', []);
      });
      const after = await view();
      const { frames, draws } = await page.evaluate(() => {
        window.__stop = true;
        return { frames: window.__frames, draws: window.agentdrafterViewer.takeDraws() };
      });
      await cdp.send('Emulation.setCPUThrottlingRate', { rate: 1 });
      await page.screenshot({ path: join(out, 'look-phone-pinched.png') });
      assert(problems.length === 0, problems.join('; '));
      assert(pinched.scale > before.scale * 2.5, `the pinch zooms in: ${before.scale} to ${pinched.scale}`);
      assert(drift < 3, `the point under the fingers stays under them: ${drift} px`);
      assert(Math.abs(panned.left - pinched.left) > 1e-3, 'the drag pans');
      assert(after.scale < panned.scale, 'the pinch zooms out');

      // Within each gesture: the display's frames, the canvas's draws, the
      // time between one draw and the next, and how long each draw took.
      const gaps = (times) => times.slice(1).map((t, i) => t - times[i]);
      const each = phases.map(({ name, began, ended }) => {
        const shown = frames.filter((t) => t >= began && t <= ended);
        const drawn = draws.filter((d) => d.at >= began && d.at <= ended);
        assert(drawn.length > 0, `${name}: the canvas was never drawn`);
        return {
          gesture: name,
          frames: shown.length,
          draws: drawn.length,
          frame_intervals: stats(gaps(shown)),
          draw_intervals: stats(gaps(drawn.map((d) => d.at))),
          draw_ms: stats(drawn.map((d) => d.ms)),
        };
      });
      const during = (t) => phases.some(({ began, ended }) => t >= began && t <= ended);
      const allDraws = draws.filter((d) => during(d.at));
      const allDrawIntervals = phases.flatMap(({ began, ended }) =>
        gaps(draws.filter((d) => d.at >= began && d.at <= ended).map((d) => d.at)),
      );
      const allFrameIntervals = phases.flatMap(({ began, ended }) => gaps(frames.filter((t) => t >= began && t <= ended)));
      const result = {
        device: `390x844 CSS px at 3 device px each, touch, CPU throttled ${THROTTLE}x`,
        webgl: await page.evaluate(() => { const gl = document.createElement('canvas').getContext('webgl2'); const ext = gl.getExtension('WEBGL_debug_renderer_info'); return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER); }),
        sheet: `${summary.sheet}: ${summary.segments} segments`,
        gestures: each,
        frame_intervals: stats(allFrameIntervals),
        draw_intervals: stats(allDrawIntervals),
        draw_ms: stats(allDraws.map((d) => d.ms)),
        pinch_drift_px: round(drift),
      };
      // Every gesture draws the canvas, again and again as the fingers
      // move; held to the display's pace, on most of its frames.
      for (const g of each) {
        assert(g.draws >= 20, `${g.gesture}: ${g.draws} draws in ${g.frames} frames`);
        if (SMOOTH !== 'draw') {
          assert(g.draws >= g.frames * 0.6, `${g.gesture}: ${g.draws} draws in ${g.frames} frames`);
        }
      }
      // The page's own work each draw, under the throttle, is a sliver of a
      // frame, whatever draws the pixels.
      const draw = result.draw_ms;
      assert(draw.median_ms <= 1 && draw.p95_ms <= 4, `the page's draw is not small: ${JSON.stringify(result)}`);
      // Smooth: 95% of the draws during gestures come at the display's pace
      // (60 a second), 99% within one refresh of it, and none stalls for
      // 100 ms; and so do the display's frames.
      result.held_to = SMOOTH === 'draw' ? "the draws' count and the page's draw" : 'the draws and the frames';
      if (SMOOTH !== 'draw') {
        for (const name of ['draw_intervals', 'frame_intervals']) {
          const { p95_ms, p99_ms, max_ms } = result[name];
          assert(p95_ms <= 20 && p99_ms <= 34 && max_ms <= 100, `${name} are not smooth: ${JSON.stringify(result)}`);
        }
      }
      return result;
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /** WebGL2 and no WebGPU at all; and with no WebGL, a page that says so. */
  async webgl2() {
    const built = readdirSync(join(viewer, 'dist', 'assets'))
      .filter((f) => f.endsWith('.js'))
      .map((f) => readFileSync(join(viewer, 'dist', 'assets', f), 'utf8'))
      .join('\n');
    assert(!/navigator\.gpu|requestAdapter|["']webgpu["']/.test(built), 'the page never asks for WebGPU');
    const server = await serve('127.0.0.1:0');
    const b = await browser(['--disable-features=WebGPU']);
    try {
      const context = await b.newContext(LAPTOP);
      await context.addInitScript(() => {
        delete Navigator.prototype.gpu;
        if (typeof WorkerNavigator !== 'undefined') delete WorkerNavigator.prototype.gpu;
      });
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant`, 'DP-LD-101');
      const seen = await page.evaluate(() => {
        const gl = document.createElement('canvas').getContext('webgl2');
        const viewer = window.agentdrafterViewer;
        const s = viewer.summary;
        const scale = 1000 / (s.paper[2] - s.paper[0]);
        const pixels = atob(viewer.capture({ left: s.paper[0], top: s.paper[3], scale, width: 1000, height: 660 }));
        let ink = 0;
        for (let i = 0; i < pixels.length; i += 4) if (pixels.charCodeAt(i) < 128) ink++;
        return { gpu: 'gpu' in navigator, webgl2: !!gl, version: gl?.getParameter(gl.VERSION), ink };
      });
      await page.screenshot({ path: join(out, 'look-webgl2-only.png') });
      assert(!seen.gpu, 'there is no WebGPU');
      assert(seen.webgl2, 'there is WebGL2');
      assert(seen.ink > 10000, `the sheet is drawn: ${seen.ink} inked pixels`);
      assert(problems.length === 0, problems.join('; '));

      // With no WebGL at all, the page says it cannot draw, and why.
      const bare = await browser(['--disable-webgl', '--disable-3d-apis']);
      try {
        const page = await (await bare.newContext(LAPTOP)).newPage();
        await page.goto(`${server.base}/?source=demo-plant`);
        await page.waitForSelector('.va-note.failed');
        seen.without_webgl = await page.textContent('.va-note.failed');
        assert(seen.without_webgl.includes('no WebGL2'), `the page says there is no WebGL2: ${seen.without_webgl}`);
      } finally {
        await bare.close();
      }
      return seen;
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /** Wheel, drag, double click, the zoom buttons and the tabs, with a mouse. */
  async mouse() {
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    try {
      const context = await b.newContext(LAPTOP);
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant`, 'DP-LD-101');
      const view = () => page.evaluate(() => window.agentdrafterViewer.view());
      const fitted = await view();
      await page.mouse.move(500, 400);
      await page.mouse.wheel(0, -400);
      await sleep(100);
      const wheeled = await view();
      assert(wheeled.scale > fitted.scale * 1.5, `the wheel zooms in: ${fitted.scale} to ${wheeled.scale}`);
      await page.mouse.down();
      await page.mouse.move(300, 300, { steps: 10 });
      await page.mouse.up();
      await sleep(600);
      const dragged = await view();
      assert(dragged.left > wheeled.left, 'a drag pans the sheet with the pointer');
      await page.mouse.dblclick(400, 400);
      await sleep(400);
      const doubled = await view();
      assert(doubled.scale > dragged.scale * 1.8, 'a double click zooms in');
      await page.click('[aria-label="Fit the sheet"]');
      await sleep(600);
      const back = await view();
      assert(Math.abs(back.scale - fitted.scale) / fitted.scale < 1e-6, 'fit shows the whole sheet again');
      await page.click('[aria-label="Zoom in"]');
      await sleep(400);
      assert((await view()).scale > fitted.scale * 1.5, 'the zoom button zooms in');
      await page.click('.tab:has-text("DP-LD-202")');
      await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-202');
      assert(page.url().includes('sheet=DP-LD-202'), 'the address names the sheet shown');
      assert(problems.length === 0, problems.join('; '));
      return { fitted, wheeled, dragged, doubled };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * The first load shows the whole sheet, at phone and laptop sizes; so it
   * does when the screen changes size or pixel ratio under the fitted view,
   * as the reviewer has not moved it; a view the reviewer moved stays put.
   */
  async fit() {
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    // Where the paper is on the canvas, in CSS pixels, and the room the
    // canvas shows, above a phone's bottom sheet.
    const where = (page) =>
      page.evaluate(() => {
        const probe = window.agentdrafterViewer;
        const [left, bottom, right, top] = probe.summary.paper;
        const v = probe.view();
        const canvas = document.querySelector('.stage');
        const c = canvas.getBoundingClientRect();
        const sheet = document.querySelector('.va-sheet').getBoundingClientRect();
        const below = sheet.top > c.top + 10 && sheet.left < c.right - 10 ? c.bottom - sheet.top : 0;
        return {
          paper: [(left - v.left) * v.scale, (v.top - top) * v.scale, (right - v.left) * v.scale, (v.top - bottom) * v.scale],
          room: [c.width, c.height - below],
          backing: [canvas.width, canvas.height],
          ratio: window.devicePixelRatio,
          scale: v.scale,
        };
      });
    const inSight = (w, what) => {
      const [x0, y0, x1, y1] = w.paper;
      const [width, height] = w.room;
      assert(x0 >= -0.5 && y0 >= -0.5 && x1 <= width + 0.5 && y1 <= height + 0.5, `${what}: the sheet is not all in sight: ${JSON.stringify(w)}`);
      // Fitted, not shrunk: one side spans the room but its padding.
      assert(x1 - x0 >= width - 21 || y1 - y0 >= height - 21, `${what}: the sheet does not fill the room: ${JSON.stringify(w)}`);
    };
    const settle = (page) => page.evaluate(() => new Promise((done) => setTimeout(() => requestAnimationFrame(() => done()), 300)));
    const seen = {};
    try {
      for (const [name, device] of [
        ['phone', PHONE],
        ['laptop', LAPTOP],
      ]) {
        const context = await b.newContext(device);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant`, 'DP-LD-101');
        await settle(page);
        seen[name] = await where(page);
        inSight(seen[name], `first load at ${name} size`);
        await page.click('.tab:nth-child(3)');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-103');
        await settle(page);
        inSight(await where(page), `another sheet at ${name} size`);
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }

      // Laid out at one size, then the screen another, as a browser may
      // before it settles: the sheet is fitted to the screen it ends at.
      const context = await b.newContext({ viewport: { width: 1000, height: 700 }, deviceScaleFactor: 1 });
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant`, 'DP-LD-101');
      await page.setViewportSize({ width: 390, height: 844 });
      await settle(page);
      seen.resized = await where(page);
      inSight(seen.resized, 'after the screen narrowed');
      // And the pixel ratio changing under it, as on another screen.
      const cdp = await context.newCDPSession(page);
      await cdp.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 3, mobile: true });
      await settle(page);
      seen.ratio = await where(page);
      inSight(seen.ratio, 'after the pixel ratio changed');
      assert(seen.ratio.backing[0] === Math.round(seen.ratio.room[0] * 3), `the canvas has 3 device pixels to a CSS pixel: ${JSON.stringify(seen.ratio)}`);
      // Zoomed by the reviewer: a change of size keeps their view's scale.
      await page.mouse.move(150, 300);
      await page.mouse.wheel(0, -300);
      await settle(page);
      const zoomed = await where(page);
      await cdp.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 700, deviceScaleFactor: 3, mobile: true });
      await settle(page);
      const kept = await where(page);
      assert(Math.abs(kept.scale - zoomed.scale) < 1e-9 && zoomed.scale > seen.ratio.scale * 1.1, `a view the reviewer zoomed is not fitted again: ${JSON.stringify({ fitted: seen.ratio.scale, zoomed: zoomed.scale, kept: kept.scale })}`);
      // The fit button's glide still running when the screen changes size:
      // the sheet ends fitted to the new screen, not where the glide was going.
      // Phone to laptop width, so the new fit's scale differs from the glide's.
      await page.click('[aria-label="Fit the sheet"]');
      await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1000, height: 700, deviceScaleFactor: 1, mobile: false });
      await settle(page);
      await page.evaluate(() => new Promise((done) => setTimeout(done, 600)));
      seen.glide = await where(page);
      inSight(seen.glide, 'after a change of size during the fit glide');
      assert(problems.length === 0, problems.join('; '));
      return seen;
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * Going back to a sheet already drawn shows it, and says nothing of the
   * sheet asked for meanwhile, whether that one is still coming or failed.
   */
  async tabs() {
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    try {
      const context = await b.newContext(LAPTOP);
      let release;
      const held = new Promise((done) => {
        release = done;
      });
      const asked = [];
      await context.route('**/commands/display-list', async (route) => {
        const sheet = route.request().postDataJSON()?.sheet ?? null;
        asked.push(sheet);
        if (sheet === 'DP-LD-102') {
          await held;
          await route.continue();
        } else if (sheet === 'DP-LD-103') {
          await route.fulfill({
            status: 422,
            contentType: 'application/json',
            body: JSON.stringify({ ok: false, error: { code: 'read_failed', message: 'a made-up failure of DP-LD-103' } }),
          });
        } else {
          await route.continue();
        }
      });
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant`, 'DP-LD-101');
      const note = () => page.evaluate(() => document.querySelector('.va-note')?.textContent ?? null);
      const shown = () => page.evaluate(() => window.agentdrafterViewer.summary.sheet);

      // B is asked for and still coming; back to A before it comes.
      await page.click('.tab:has-text("DP-LD-102")');
      await page.waitForFunction(() => document.querySelector('.va-note')?.textContent?.includes('DP-LD-102'));
      await page.click('.tab:has-text("DP-LD-101")');
      await page.waitForFunction(() => !document.querySelector('.va-note'));
      assert((await shown()) === 'DP-LD-101', 'A is shown');
      // B comes after all: A stays, with nothing said.
      release();
      await sleep(500);
      assert((await note()) === null && (await shown()) === 'DP-LD-101', `B coming late changes nothing: ${await note()}`);
      // B, kept when it came, shows at once when asked for again.
      const before = asked.length;
      await page.click('.tab:has-text("DP-LD-102")');
      await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-102');
      assert(asked.length === before, 'a sheet drawn before is not asked for again');

      // C fails; back to B.
      await page.click('.tab:has-text("DP-LD-103")');
      await page.waitForFunction(() => document.querySelector('.va-note.failed')?.textContent?.includes('made-up failure'));
      await page.click('.tab:has-text("DP-LD-102")');
      await page.waitForFunction(() => !document.querySelector('.va-note'));
      assert((await shown()) === 'DP-LD-102', 'B is shown again');
      assert(problems.filter((p) => !p.includes('422')).length === 0, problems.join('; '));
      return { asked };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * A change set the command line sends, in another process, shows in the
   * open page without a reload: a toast names it, the sheet shown is drawn
   * again with its object marked, and a sheet it landed on that is not
   * shown is drawn anew when its tab is opened.
   */
  async live() {
    const folder = freshPlant('live');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    try {
      const shots = [];
      for (const [name, device] of [
        ['laptop', LAPTOP],
        ['phone', PHONE],
      ]) {
        const context = await b.newContext(device);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        const before = await objectOf(page, 'DP-LD-101/device');
        assert(before.place >= 0 && before.bounds.length === 4, 'the device is on the sheet');
        const sent = performance.now();
        const reply = sendChange(folder, move('DP-LD-101/device', 0.5));
        assert(reply.ok, `the change set lands: ${JSON.stringify(reply.error)}`);
        const n = reply.result.journal_entry;
        await page.waitForFunction((title) => document.querySelector('.va-toast')?.textContent?.includes(title), `Change set ${n} landed on DP-LD-101`, { timeout: 15000 });
        await page.waitForFunction(
          ([place, left]) => {
            const probe = window.agentdrafterViewer;
            return probe.bounds([place])[0] > left + 0.4 && probe.marked?.places.includes(place);
          },
          [before.place, before.bounds[0]],
          { timeout: 15000 },
        );
        const shown = performance.now() - sent;
        const marked = await page.evaluate(() => window.agentdrafterViewer.marked);
        assert(marked.added[marked.places.indexOf(before.place)] === false, 'the moved device is marked changed');
        await sleep(400);
        await page.screenshot({ path: join(out, `live-${name}.png`) });
        shots.push(`live-${name}.png`);

        // A change set on another sheet: named by the toast, and drawn anew
        // when its tab is opened.
        const other = sendChange(folder, {
          operations: [{ op: 'insert_block', sheet: 'DP-LD-102', block: 'TRANSMITTER', position: [2, 3, 0], attributes: { FUNC: 'LI', LOOP: '102' }, id: `added-${name}`, name: `LI-102-${name}` }],
        });
        assert(other.ok, `the second change set lands: ${JSON.stringify(other.error)}`);
        await page.waitForFunction((title) => document.querySelector('.va-toast')?.textContent?.includes(title), `landed on DP-LD-102`, { timeout: 15000 });
        await page.click('.tab:has-text("DP-LD-102")');
        await page.waitForFunction(
          (id) => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-102' && window.agentdrafterViewer.summary.objects.some((o) => o.id === id),
          `DP-LD-102/added-${name}`,
          { timeout: 15000 },
        );
        assert(problems.length === 0, problems.join('; '));
        await context.close();
        // The next size starts from the set as it is now.
        if (name === 'laptop') {
          assert(agentIn(folder, 'undo', 'demo-plant', String(n)).ok, 'the set is put back');
        }
        if (name === 'laptop') results_live.ms_to_show = Math.round(shown);
      }
      return { screenshots: shots, ms_from_change_to_drawn: results_live.ms_to_show };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * Stepping through the journal shows the set as it was at each entry,
   * read-only; an entry whose undo state is pruned says it cannot be
   * shown; back to live returns to the set as it is.
   */
  async journal() {
    const folder = freshPlant('journal');
    // 51 change sets on another sheet's row, then two moves of the device:
    // 53 change sets, so the oldest three have had their undo states pruned.
    for (let i = 0; i < 51; i++) {
      const reply = sendChange(folder, move('DP-LD-104/io-channel', 0.001));
      assert(reply.ok, `filler ${i}: ${JSON.stringify(reply.error)}`);
    }
    assert(sendChange(folder, move('DP-LD-101/device', 0.5)).ok, 'entry 52');
    assert(sendChange(folder, move('DP-LD-101/device', 0.5)).ok, 'entry 53');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    try {
      const context = await b.newContext(LAPTOP);
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
      await live(page);
      const now = await objectOf(page, 'DP-LD-101/device');
      const left = (entry) =>
        page.waitForFunction(
          ([entry, place]) => {
            const probe = window.agentdrafterViewer;
            return probe.summary.entry === entry ? probe.bounds([place])[0] : false;
          },
          [entry, now.place],
          { timeout: 15000 },
        ).then((h) => h.jsonValue());
      // Back one: entry 53, the newest, as it was; then entry 52.
      await page.click('button[aria-label="Older"]');
      const at53 = await left(53);
      assert(Math.abs(at53 - now.bounds[0]) < 1e-6, 'entry 53 is the set as it is');
      assert((await page.textContent('.va-banner')).includes('Entry 53, as it was'), 'the banner says which entry is shown');
      await page.click('button[aria-label="Older"]');
      const at52 = await left(52);
      assert(Math.abs(at53 - at52 - 0.5) < 1e-6, `entry 52 has the device half an inch left of 53: ${at52}, ${at53}`);
      const marked = await page.evaluate(() => window.agentdrafterViewer.marked);
      assert(marked.places.includes(now.place), 'the entry\'s change is marked');
      await sleep(700);
      await page.screenshot({ path: join(out, 'journal-step.png') });
      // A journal of 53 entries scrolls in its panel: the drawing keeps the
      // screen's height, and the entry's change is in sight.
      const shown = await page.evaluate((place) => {
        const r = document.querySelector('.stage').getBoundingClientRect();
        const v = window.agentdrafterViewer.view();
        const [x0, y0, x1, y1] = window.agentdrafterViewer.bounds([place]);
        return { height: r.height, inner: innerHeight, x: ((x0 + x1) / 2 - v.left) * v.scale, y: (v.top - (y0 + y1) / 2) * v.scale, width: r.width };
      }, now.place);
      assert(shown.height < shown.inner, `the drawing is no taller than the screen: ${shown.height}`);
      assert(shown.x > 0 && shown.x < shown.width && shown.y > 0 && shown.y < shown.height, `the change is in sight: ${JSON.stringify(shown)}`);
      // The oldest entry, a change set on DP-LD-104: its undo state is
      // pruned, and it cannot be shown.
      await page.click('li[data-entry="1"] .je-head');
      await page.waitForSelector('.va-cannot', { timeout: 15000 });
      const said = await page.textContent('.va-cannot');
      assert(said.includes('Entry 1 cannot be shown') && said.includes('pruned'), `the pruned entry says so: ${said}`);
      await page.screenshot({ path: join(out, 'journal-pruned.png') });
      // Back to live.
      await page.click('.va-banner button');
      await page.waitForFunction(() => window.agentdrafterViewer.summary.entry === null && !document.querySelector('.va-cannot') && !document.querySelector('.va-banner'));
      await page.click('.tab:has-text("DP-LD-101")');
      await page.waitForFunction(() => window.agentdrafterViewer.summary.sheet === 'DP-LD-101' && window.agentdrafterViewer.summary.entry === null);
      const back = await objectOf(page, 'DP-LD-101/device');
      assert(Math.abs(back.bounds[0] - now.bounds[0]) < 1e-6, 'back to the set as it is');
      // Nothing was written: the journal still holds 53 entries.
      const journal = agentIn(folder, 'journal', 'demo-plant');
      assert(journal.result.entries.length === 53, 'stepping writes nothing');
      assert(problems.filter((p) => !p.includes('422')).length === 0, problems.join('; '));
      return { entries: 53, pruned: journal.result.entries.filter((e) => e.undo_state === 'pruned').map((e) => e.entry), screenshots: ['journal-step.png', 'journal-pruned.png'] };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /** A tap on an insert, at phone size, shows its ID, name, layer and attributes. */
  async pick() {
    const folder = freshPlant('pick');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    try {
      const context = await b.newContext(PHONE);
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
      const device = await objectOf(page, 'DP-LD-101/device');
      const [x0, y0, x1, y1] = device.bounds;
      const at = await page.evaluate(
        ([x, y]) => {
          const v = window.agentdrafterViewer.view();
          const r = document.querySelector('.stage').getBoundingClientRect();
          return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
        },
        [(x0 + x1) / 2, (y0 + y1) / 2],
      );
      await page.touchscreen.tap(at[0], at[1]);
      await page.waitForSelector('.va-inspect', { timeout: 15000 });
      const shown = await page.evaluate(() => {
        const rows = {};
        for (const tr of document.querySelectorAll('.va-inspect .kv tr')) {
          const th = tr.querySelector('th')?.textContent;
          if (th) rows[th] = tr.querySelector('td')?.textContent;
        }
        return rows;
      });
      assert(shown.ID === 'DP-LD-101/device', `the inspector shows its ID: ${JSON.stringify(shown)}`);
      assert(shown.Name === 'FT-101', 'its name');
      assert(shown.Layer === '0', 'its layer');
      assert(shown.Type === 'Insert of TRANSMITTER', 'its block');
      assert(shown.FUNC === 'FT' && shown.LOOP === '101', 'its attributes');
      const marked = await page.evaluate(() => window.agentdrafterViewer.marked);
      assert(marked.picked === device.place, 'the picked object is marked');
      await sleep(300);
      await page.screenshot({ path: join(out, 'pick-phone.png') });
      // A tap where nothing is drawn, above the paper, picks nothing, and
      // the journal comes back.
      const bare = await page.evaluate(() => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + (8.5 - v.left) * v.scale, r.top + (v.top - 11) * v.scale - 40];
      });
      await page.touchscreen.tap(bare[0], bare[1]);
      await page.waitForFunction(() => !document.querySelector('.va-inspect'));
      assert(problems.length === 0, problems.join('; '));
      return { picked: shown, screenshot: 'pick-phone.png' };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /** Undo last, from the page, undoes the newest change set as one, through the undo command. */
  async undo() {
    const folder = freshPlant('undo');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    try {
      const context = await b.newContext(LAPTOP);
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
      await live(page);
      const before = await objectOf(page, 'DP-LD-101/device');
      // A template edit, which lands on every sheet built from it, as one.
      const reply = sendChange(folder, {
        operations: [
          { op: 'move', target: { id: 'DP-LD-101/device' }, by: [0.5, 0, 0] },
          { op: 'move', target: { id: 'DP-LD-101/border' }, by: [0.05, 0, 0] },
        ],
      });
      assert(reply.ok, JSON.stringify(reply.error));
      await page.waitForSelector('button[data-act="undo"]:not([disabled])');
      assert((await page.textContent('button[data-act="undo"]')).includes('#1'), 'Undo last names the change set');
      await page.click('button[data-act="undo"]');
      await page.waitForFunction(() => document.querySelector('.va-toast')?.textContent?.includes('Undid change set 1'), null, { timeout: 15000 });
      await page.waitForFunction(
        ([place, left]) => Math.abs(window.agentdrafterViewer.bounds([place])[0] - left) < 1e-6,
        [before.place, before.bounds[0]],
        { timeout: 15000 },
      );
      const journal = agentIn(folder, 'journal', 'demo-plant').result;
      const last = journal.entries.at(-1);
      assert(last.kind === 'undo' && JSON.stringify(last.undid) === '[1]', `the journal records the undo: ${JSON.stringify(last)}`);
      const restored = new Set(last.files.map((f) => f.path));
      assert(restored.has('rows.yaml') && restored.has('templates/loop-2-wire.yaml') && restored.has('dwg/DP-LD-302.dwg'), 'every file of the change set went back, every sheet with it');
      await page.waitForSelector('button[data-act="undo"][disabled]');
      await page.screenshot({ path: join(out, 'undo-laptop.png') });
      assert(problems.length === 0, problems.join('; '));
      return { undid: last.undid, files: last.files.length, screenshot: 'undo-laptop.png' };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * Accept from the page: refused while the set is inconsistent; asks,
   * listing the failed checks; then issues every change set in place, which
   * the journal records as the viewer's, and undo can no longer take.
   */
  async accept() {
    const folder = freshPlant('accept');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    try {
      const context = await b.newContext(PHONE);
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
      await live(page);
      const toast = (text) => page.waitForFunction((t) => document.querySelector('.va-toast')?.textContent?.includes(t), text, { timeout: 15000 });

      // Inconsistent: lands on one sheet, fails on another.
      const partial = sendChange(folder, {
        operations: [
          { op: 'move', target: { id: 'DP-LD-101/device' }, by: [0.2, 0, 0] },
          { op: 'move', target: { id: 'DP-LD-102/nothing-here' }, by: [0.2, 0, 0] },
        ],
      });
      assert(!partial.ok && partial.result?.landed === 1, `lands on one sheet only: ${JSON.stringify(partial.error)}`);
      await toast('failed on DP-LD-102');
      await page.click('button[data-act="accept"]');
      await toast('Can’t accept yet');
      assert((await page.textContent('.va-toast')).includes('inconsistent'), 'the refusal says why');
      assert(!(await page.$('.va-modal')), 'no dialog: nothing to confirm');
      await page.screenshot({ path: join(out, 'accept-refused.png') });

      // Undone from the page, then a change set that fails a check.
      await page.click('button[data-act="undo"]');
      await toast('Undid change set 1');
      const off = sendChange(folder, {
        operations: [{ op: 'add_circle', sheet: 'DP-LD-101', center: [17.2, 5, 0], radius: 0.4, layer: 'notes', id: 'past-the-edge' }],
      });
      assert(off.ok, JSON.stringify(off.error));
      await toast('check');
      await page.click('button[data-act="accept"]');
      await page.waitForSelector('.va-modal .dialog', { timeout: 15000 });
      const dialog = await page.textContent('.va-modal .dialog');
      assert(dialog.includes('Accept the working revision?'), 'Accept asks for confirmation');
      assert(dialog.includes('change set') && dialog.includes('#3'), `it names what it issues: ${dialog}`);
      assert(dialog.includes('failing') && dialog.includes('DP-LD-101/past-the-edge'), `it lists the failed checks: ${dialog}`);
      await page.screenshot({ path: join(out, 'accept-dialog.png') });
      const entries = () => agentIn(folder, 'journal', 'demo-plant').result.entries;
      assert(!entries().some((e) => e.kind === 'accept'), 'nothing is issued before the reviewer confirms');
      await page.click('.va-modal [data-ok]');
      await toast('Accepted through the viewer');
      const accept = entries().at(-1);
      assert(accept.kind === 'accept' && accept.via === 'viewer', `the journal says the viewer accepted: ${JSON.stringify(accept)}`);
      assert(JSON.stringify(accept.issued) === '[3]' && accept.failed_checks.length > 0, 'it issued change set 3, despite the failed checks it lists');
      await page.waitForSelector('button[data-act="undo"][disabled]');
      const refused = agentIn(folder, 'undo', 'demo-plant', '3');
      assert(refused.error?.code === 'issued', `undo refuses an issued change set: ${JSON.stringify(refused)}`);
      await sleep(300);
      await page.screenshot({ path: join(out, 'accept-issued.png') });
      assert(problems.filter((p) => !p.includes('422')).length === 0, problems.join('; '));
      return { journal_entry: accept.entry, issued: accept.issued, via: accept.via, screenshots: ['accept-refused.png', 'accept-dialog.png', 'accept-issued.png'] };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * The exact order that once kept an old drawing for good: sheet B asked
   * for, the reviewer back on A before it comes, a change to B lands, and
   * then B's old reply comes. Going back to B asks for it again and shows
   * the change.
   */
  async late() {
    const folder = freshPlant('late');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    try {
      const context = await b.newContext(LAPTOP);
      let release;
      const held = new Promise((done) => {
        release = done;
      });
      let fetched;
      const old = new Promise((done) => {
        fetched = done;
      });
      const askedForB = [];
      await context.route('**/commands/display-list', async (route) => {
        const sheet = route.request().postDataJSON()?.sheet ?? null;
        if (sheet !== 'DP-LD-102') return route.continue();
        askedForB.push(performance.now());
        if (askedForB.length > 1) return route.continue();
        // B's reply as the set is now, held until after the change lands.
        const response = await route.fetch();
        fetched();
        await held;
        await route.fulfill({ response });
      });
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
      await live(page);
      await page.click('.tab:has-text("DP-LD-102")');
      await old;
      await page.click('.tab:has-text("DP-LD-101")');
      await page.waitForFunction(() => window.agentdrafterViewer.summary.sheet === 'DP-LD-101' && !document.querySelector('.va-note'));
      const landed = sendChange(folder, {
        operations: [{ op: 'insert_block', sheet: 'DP-LD-102', block: 'TRANSMITTER', position: [2, 3, 0], attributes: { FUNC: 'LI', LOOP: '102' }, id: 'came-late' }],
      });
      assert(landed.ok, JSON.stringify(landed.error));
      await page.waitForFunction(() => document.querySelector('.va-toast')?.textContent?.includes('landed on DP-LD-102'), null, { timeout: 15000 });
      // B's old reply comes now.
      release();
      await sleep(500);
      await page.click('.tab:has-text("DP-LD-102")');
      await page.waitForFunction(
        () => window.agentdrafterViewer.summary.sheet === 'DP-LD-102' && window.agentdrafterViewer.summary.objects.some((o) => o.id === 'DP-LD-102/came-late'),
        null,
        { timeout: 15000 },
      );
      assert(askedForB.length === 2, `B is asked for again, not shown from the old reply: asked ${askedForB.length} times`);
      assert(problems.length === 0, problems.join('; '));
      return { asked_for_b: askedForB.length };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * A journal cut back, as a commit rolled back cuts it, with the set's
   * files put back: the page drops what it kept and draws the set as it is.
   */
  async rewind() {
    const folder = freshPlant('rewind');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    try {
      const context = await b.newContext(LAPTOP);
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
      await live(page);
      const start = await objectOf(page, 'DP-LD-101/device');
      const at = (left) =>
        page.waitForFunction(
          ([place, wanted]) => Math.abs(window.agentdrafterViewer.bounds([place])[0] - wanted) < 1e-6,
          [start.place, left],
          { timeout: 15000 },
        );
      const set = join(folder, 'demo-plant');
      const journalFile = join(set, '.agentdrafter', 'journal.jsonl');
      assert(sendChange(folder, move('DP-LD-101/device', 0.5)).ok, 'change set 1');
      await at(start.bounds[0] + 0.5);
      const rowsAfterFirst = readFileSync(join(set, 'rows.yaml'));
      const journalAfterFirst = readFileSync(journalFile);
      assert(sendChange(folder, move('DP-LD-101/device', 0.5)).ok, 'change set 2');
      await at(start.bounds[0] + 1);
      await page.waitForFunction(() => document.querySelector('.va-jhead')?.textContent?.includes('2 entries'));
      // Change set 2 rolled back: its files as they were, and the journal
      // cut back to its length before.
      writeFileSync(join(set, 'rows.yaml'), rowsAfterFirst);
      writeFileSync(journalFile, journalAfterFirst);
      await page.waitForFunction(() => document.querySelector('.va-jhead')?.textContent?.includes('1 entry'), null, { timeout: 15000 });
      await at(start.bounds[0] + 0.5);
      assert(problems.length === 0, problems.join('; '));
      return { cut_back_to: 1 };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * A pick follows its object: with Y picked in [X, Y, Z], deleting X
   * through the command line keeps Y picked, and deleting Y clears it.
   */
  async follow() {
    const folder = freshPlant('follow');
    // Z, after the row's X (the device) and Y (the I/O channel).
    assert(
      sendChange(folder, { operations: [{ op: 'add_circle', sheet: 'DP-LD-101', center: [9, 3, 0], radius: 0.3, layer: 'notes', id: 'z' }] }).ok,
      'Z is added',
    );
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    try {
      const context = await b.newContext(PHONE);
      const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
      await live(page);
      const y = await objectOf(page, 'DP-LD-101/io-channel');
      const x = await objectOf(page, 'DP-LD-101/device');
      const z = await objectOf(page, 'DP-LD-101/z');
      assert(x.place < y.place && y.place < z.place, `X, Y and Z in order: ${x.place}, ${y.place}, ${z.place}`);
      const [x0, y0, x1, y1] = y.bounds;
      const tapAt = await page.evaluate(
        ([px, py]) => {
          const v = window.agentdrafterViewer.view();
          const r = document.querySelector('.stage').getBoundingClientRect();
          return [r.left + (px - v.left) * v.scale, r.top + (v.top - py) * v.scale];
        },
        [(x0 + x1) / 2, (y0 + y1) / 2],
      );
      await page.touchscreen.tap(tapAt[0], tapAt[1]);
      await page.waitForSelector('.va-inspect');
      const shownId = () =>
        page.evaluate(() => {
          for (const tr of document.querySelectorAll('.va-inspect .kv tr')) {
            if (tr.querySelector('th')?.textContent === 'ID') return tr.querySelector('td')?.textContent;
          }
          return null;
        });
      assert((await shownId()) === 'DP-LD-101/io-channel', 'Y is picked');

      // X deleted: Y keeps its pick, at its new place.
      assert(sendChange(folder, { operations: [{ op: 'delete', target: { id: 'DP-LD-101/device' } }] }).ok, 'X is deleted');
      await page.waitForFunction(() => !window.agentdrafterViewer.summary.objects.some((o) => o.id === 'DP-LD-101/device'), null, { timeout: 15000 });
      await sleep(200);
      assert((await shownId()) === 'DP-LD-101/io-channel', `Y is still picked: ${await shownId()}`);
      const now = await objectOf(page, 'DP-LD-101/io-channel');
      const marked = await page.evaluate(() => window.agentdrafterViewer.marked);
      assert(now.place === y.place - 1 && marked.picked === now.place, `the halo follows Y to its new place: ${JSON.stringify(marked)}, ${now.place}`);

      // Y deleted: the pick clears.
      assert(sendChange(folder, { operations: [{ op: 'delete', target: { id: 'DP-LD-101/io-channel' } }] }).ok, 'Y is deleted');
      await page.waitForFunction(() => !document.querySelector('.va-inspect') && window.agentdrafterViewer.marked?.picked === null, null, { timeout: 15000 });
      assert(problems.length === 0, problems.join('; '));
      return { followed: 'DP-LD-101/io-channel', cleared: true };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /** Screenshots at phone and laptop sizes. */
  async look() {
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    try {
      for (const [name, device] of [
        ['phone', PHONE],
        ['laptop', LAPTOP],
      ]) {
        const context = await b.newContext(device);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant`, 'DP-LD-101');
        await page.screenshot({ path: join(out, `look-${name}.png`) });
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      return { screenshots: ['look-phone.png', 'look-laptop.png'] };
    } finally {
      await b.close();
      await server.stop();
    }
  },
};

const results = {};
const results_live = {};
let failed = 0;
for (const [name, check] of Object.entries(checks)) {
  if (only && !only.includes(name)) continue;
  const began = performance.now();
  try {
    results[name] = { ok: true, ...(await check()) };
    console.log(`ok   ${name} (${round((performance.now() - began) / 1000, 1)} s)`);
  } catch (e) {
    failed++;
    results[name] = { ok: false, error: String(e?.stack ?? e) };
    console.log(`FAIL ${name}: ${e?.message ?? e}`);
  }
}
writeFileSync(join(out, 'results.json'), `${JSON.stringify(results, null, 2)}\n`);
console.log(JSON.stringify(results, null, 2));
process.exit(failed ? 1 : 0);
