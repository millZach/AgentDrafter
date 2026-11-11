// The viewer's browser tests, against `agentdrafter serve` and the page it
// serves, in headless Chromium. Each check starts the servers it needs and
// stops them before it ends.
//
//   node e2e/run.mjs [--bin <agentdrafter>] [--chrome <chromium>] [--out <folder>] [--only <check>,...]
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
// - look:    screenshots at phone and laptop sizes.
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

/** Runs the command line in the scratch folder and gives its JSON reply. */
function agentdrafter(...args) {
  const line = execFileSync(bin, args, { cwd: scratch, encoding: 'utf8' });
  const reply = JSON.parse(line);
  if (!reply.ok) throw new Error(`agentdrafter ${args.join(' ')}: ${JSON.stringify(reply.error)}`);
  return reply.result;
}

/** `agentdrafter serve` in the scratch folder, with every line it printed. */
async function serve(listen) {
  const child = spawn(bin, ['serve', '--listen', listen], { cwd: scratch, stdio: ['ignore', 'pipe', 'pipe'] });
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
  const page = await context.newPage();
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
    // pixel of the other's and all of it within two; at most 0.2% of the
    // pixels differing by more than 64 levels in a channel, and the mean
    // difference under 2 levels in 255.
    for (const r of results) {
      for (const side of ['png', 'viewer']) {
        assert(r[`${side}_ink_within_1px`] >= 0.999, `${r.case}: ${side} ink within 1 px is ${r[`${side}_ink_within_1px`]}`);
        assert(r[`${side}_ink_within_2px`] >= 0.99999, `${r.case}: ${side} ink within 2 px is ${r[`${side}_ink_within_2px`]}`);
      }
      assert(r.pixels_differing_by_more_than_64 <= 0.002, `${r.case}: ${r.pixels_differing_by_more_than_64} of the pixels differ by more than 64`);
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
        window.agentdrafterViewer.takeDrawTimes();
      });
      const touch = (type, points) =>
        cdp.send('Input.dispatchTouchEvent', {
          type,
          touchPoints: points.map(([x, y], id) => ({ x, y, id, radiusX: 2, radiusY: 2, force: 1 })),
        });
      const toDrawing = (v, sx, sy) => [v.left + (sx - box.x) / v.scale, v.top - (sy - box.y) / v.scale];
      const toPage = (v, x, y) => [box.x + (x - v.left) * v.scale, box.y + (v.top - y) * v.scale];
      const view = () => page.evaluate(() => window.agentdrafterViewer.view());
      const cx = box.x + box.width / 2;
      const cy = box.y + box.height * 0.4;

      // Two fingers spread apart, their middle drifting left.
      const before = await view();
      const pair = (d, dx) => [
        [cx - d / 2 + dx, cy],
        [cx + d / 2 + dx, cy],
      ];
      await touch('touchStart', pair(80, 0));
      for (let i = 1; i <= 60; i++) {
        await touch('touchMove', pair(80 + i * 3, -i));
        await sleep(16);
      }
      const pinched = await view();
      const under = toDrawing(before, cx, cy);
      const [ux, uy] = toPage(pinched, under[0], under[1]);
      const drift = Math.hypot(ux - (cx - 60), uy - cy);
      await touch('touchEnd', []);
      // One finger drags, and lets go moving.
      await touch('touchStart', [[cx, cy]]);
      for (let i = 1; i <= 40; i++) {
        await touch('touchMove', [[cx - i * 3, cy - i * 2]]);
        await sleep(16);
      }
      await touch('touchEnd', []);
      await sleep(400);
      const panned = await view();
      // Two fingers close in.
      await touch('touchStart', pair(260, 0));
      for (let i = 1; i <= 50; i++) {
        await touch('touchMove', pair(260 - i * 3, 0));
        await sleep(16);
      }
      await touch('touchEnd', []);
      await sleep(300);
      const after = await view();
      const { frames, draws } = await page.evaluate(() => {
        window.__stop = true;
        const f = window.__frames;
        const gaps = [];
        for (let i = 1; i < f.length; i++) gaps.push(f[i] - f[i - 1]);
        return { frames: gaps, draws: window.agentdrafterViewer.takeDrawTimes() };
      });
      await cdp.send('Emulation.setCPUThrottlingRate', { rate: 1 });
      await page.screenshot({ path: join(out, 'look-phone-pinched.png') });
      assert(problems.length === 0, problems.join('; '));
      assert(pinched.scale > before.scale * 2.5, `the pinch zooms in: ${before.scale} to ${pinched.scale}`);
      assert(drift < 3, `the point under the fingers stays under them: ${drift} px`);
      assert(Math.abs(panned.left - pinched.left) > 1e-3, 'the drag pans');
      assert(after.scale < panned.scale, 'the pinch zooms out');
      const result = {
        device: `390x844 CSS px at 3 device px each, touch, CPU throttled ${THROTTLE}x`,
        webgl: await page.evaluate(() => { const gl = document.createElement('canvas').getContext('webgl2'); const ext = gl.getExtension('WEBGL_debug_renderer_info'); return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER); }),
        sheet: `${summary.sheet}: ${summary.segments} segments`,
        frame_intervals: stats(frames),
        draw_call_per_frame: stats(draws),
        pinch_drift_px: round(drift),
      };
      // Smooth: 95% of frames come at the display's pace (60 a second), 99%
      // within one refresh of it, and none stalls for 100 ms.
      const { p95_ms, p99_ms, max_ms } = result.frame_intervals;
      const smooth = p95_ms <= 20 && p99_ms <= 34 && max_ms <= 100;
      assert(smooth, `frames are not smooth: ${JSON.stringify(result)}`);
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
