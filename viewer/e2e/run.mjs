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
//            and attributes in quick properties;
// - snaps:   object snaps: the marker and tag at a real snap point, as
//            `query snap` gives it, the status bar's exact point, Tab, a
//            click's read-out and Copy as snapped point, which a change set
//            takes, the Object snap menu and F3 changing the running snaps,
//            remembered across a reload, the search's time per pointer
//            move, and on a phone the magnifier a held finger brings up
//            and the snap it takes when lifted;
// - frame:   the drafting app's frame at laptop and phone sizes: the ribbon's
//            tools and tips, the command bar's prompts, quick
//            properties beside the selected object, the Layer box, the
//            status bar's coordinates, paper and toggles, Grid (F7) drawing
//            and taking away the grid, the side panel's tabs, and on a
//            phone the ribbon folded behind Tools;
// - undo:    Undo last undoes the newest change set, as one;
// - accept:  refused while the set is inconsistent; warns of failed checks
//            and lists them; then issues the change sets, which can no
//            longer be undone, and the journal says the viewer accepted;
// - late:    a sheet's reply that comes after a change to that sheet landed
//            is not kept: going back to the sheet asks for it again;
// - rewind:  a journal cut back, as a commit rolled back cuts it, drops
//            the drawings kept and draws the set as it is;
// - follow:  a picked object stays picked as objects before it are
//            deleted, and the pick clears when it is deleted itself;
// - tools:   the drawing tools, by mouse, by alias and typed coordinates,
//            and by touch: each sends one change set through `change`,
//            which lands in the journal and builds the
//            sheet's DWG again; the template warning's Cancel and Change
//            all; and Undo last of a drafter's edit.
// - copy:    the Copy tool (#66), at laptop and phone sizes: copies placed
//            by a typed displacement, a length and a click, dashed and
//            tagged, taken back by Ctrl+Z, U and Undo; Enter landing them
//            as one change set, one journal entry; Esc landing them; and a
//            template's object warned of and asked about once, Change all
//            landing on every sheet built from it.
// - array:   the Array tool (#66), at laptop and phone sizes: rectangular
//            and polar arrays set up in the panel and typed in the command
//            bar, the copies drawn dashed and tagged, the total toggle, Esc
//            and Cancel sending nothing, Enter and Array landing one change
//            set, one journal entry; and a template's object warned of and
//            asked about in the band, Change all landing on its seven sheets.
// - props:   editing in quick properties, by mouse and keyboard and by
//            touch: a layer, a text's contents in the panel and on the
//            drawing, an attribute, a radius and a diameter, and the Layer
//            box moving the selected object, each one change set; Esc and
//            Cancel landing nothing, Change all landing on every sheet, a
//            row value on its sheet alone; and Undo last of an edit.
// - preview: a change set drawn before it lands (#154): a template's Move
//            drawn as it would leave each sheet it reaches, stepped through
//            by the arrows, the keys and the tabs, with a check it would
//            fail clouded and named; cancelled, writing nothing; changed,
//            landing on every sheet; a sheet whose DWG cannot be written
//            refusing it until it can; at laptop and phone sizes.
// - proposals: proposals (#155): an agent proposes through the HTTP API
//            and the command line while the page is open, and each is drawn
//            dashed with a card at the change and counted on its sheets'
//            tabs; the set changes under them, which the cards say, and
//            one is refused now; opened in the band's PROPOSAL form, closed
//            leaving it waiting, discarded, and kept from a card and from
//            the band, the journal reading J1; Undo last of a kept one;
//            and on a phone, chips that open into cards, and the peek.
// - assistant: the assistant's panel (#156), on the fake agent, which a
//            shim named claude and one named codex start, first on serve's
//            PATH: the picker, the intro and the chip; a message about the
//            object selected, its command lines, one opened, its words and
//            its proposal's card in the chat and on the drawing, kept from
//            the chat; words from the command bar, its hint and Space; a
//            message without what the drafter looks at; what the core holds
//            of it through `looking-at`; a change the agent cannot make;
//            the picker moving to Codex; no agent found; and on a phone,
//            the peek, the sheet raised on the chat, and Keep. Linux and
//            macOS only: the shims are shell scripts.
// - guides:  guides (#157), at laptop and phone sizes: a guide given
//            through the HTTP API, Start from its toast, the banner's words,
//            the tool pulsing and the ring with its tag; each step by hand,
//            another layer, another tool, a wrong point and Esc each with
//            its note and what takes it back; Do it for me, Skip and Stop;
//            the journal's marks; and the one Undo of the guide's steps.
//
// It writes results.json, the screenshots and each compared image into
// --out (target/viewer-e2e by default), and exits 1 when any check fails.

import { execFileSync, spawn } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { createHash } from 'node:crypto';
import { chmodSync, statSync } from 'node:fs';
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
for (const sheet of ['geometry.yaml', 'hatch.yaml', 'text.yaml', 'layout.yaml', 'layouts.yaml']) {
  cpSync(join(root, 'examples', sheet), join(scratch, sheet));
}
cpSync(join(viewer, 'e2e', 'overlap.yaml'), join(scratch, 'overlap.yaml'));
cpSync(join(viewer, 'e2e', 'viewport-clip.yaml'), join(scratch, 'viewport-clip.yaml'));
cpSync(join(viewer, 'e2e', 'viewport-boundary.yaml'), join(scratch, 'viewport-boundary.yaml'));
cpSync(join(viewer, 'e2e', 'ten-layouts.yaml'), join(scratch, 'ten-layouts.yaml'));

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
/**
 * Each sheet's tab and its mark while a preview is shown: `✓`, `!`, `✕` or
 * `–`, `off` for a sheet it does not reach, or empty.
 */
function tabMarks(page) {
  return page.evaluate(() =>
    Object.fromEntries(
      [...document.querySelectorAll('.sheettabs .tab')].map((t) => [
        t.querySelector('b').textContent,
        t.classList.contains('pvoff') ? 'off' : (t.querySelector('.vd')?.textContent ?? ''),
      ]),
    ),
  );
}

/** Every file under `folder`, the journal's own among them, by its path, with a fingerprint of its bytes. */
function filesIn(folder) {
  const found = {};
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const at = join(dir, name);
      // What the drafter is looking at is kept beside the journal, not in
      // the set's records, and the page writes it as the selection changes.
      if (dir.endsWith('.agentdrafter') && (name === 'looking-at.json' || name.startsWith('.looking-at.json.'))) continue;
      if (statSync(at).isDirectory()) walk(at);
      else found[at.slice(folder.length)] = createHash('sha256').update(readFileSync(at)).digest('hex');
    }
  };
  walk(folder);
  return found;
}

async function live(page) {
  await page.waitForSelector('.live.open', { timeout: 30000 });
}

/**
 * Opens the side panel's tab `side`, `chat` or `journal`, by a click or a
 * tap, and waits until it is on; a slow runner's first press that does not
 * take is pressed once more.
 */
async function sideTab(page, side, how = 'click') {
  const tab = `.stab[data-side="${side}"]`;
  for (let attempt = 1; ; attempt += 1) {
    if (how === 'tap') await page.tap(tab);
    else await page.click(tab);
    try {
      await page.waitForSelector(`${tab}.on`, { timeout: 5000 });
      return;
    } catch (e) {
      if (attempt >= 2) throw new Error(`the ${side} tab did not come on after two presses: ${e.message}`);
    }
  }
}

/**
 * On a phone, waits until the sheet has come to rest, no longer sliding or
 * moved by its peek changing height; `raised`, above its peek too. Gives
 * whether it came to rest so within `timeout`.
 */
async function sheetAtRest(page, raised, timeout = 30000) {
  await page.evaluate(() => {
    window.__sheetTop = undefined;
  });
  return page
    .waitForFunction(
      (raised) => {
        const sheet = document.querySelector('.va-sheet');
        if (!sheet || (raised && sheet.dataset.snap === 'peek')) return false;
        const top = sheet.getBoundingClientRect().top;
        const was = window.__sheetTop;
        window.__sheetTop = top;
        return was !== undefined && Math.abs(top - was) < 0.5;
      },
      raised,
      { timeout, polling: 150 },
    )
    .then(
      () => true,
      () => false,
    );
}

/**
 * Waits until the view has come to rest: no glide, fling or reveal moving
 * it, the same for 5 frames and 300 ms in a row. Gives the view it rests at.
 * The deadline is kept by a timer in the page, not by the frames, which a
 * page that stops drawing stops giving, and by one in Node past it, for a
 * page that answers nothing at all.
 */
async function viewAtRest(page, timeout = 15000) {
  const resting = page.evaluate(
    (timeout) =>
      new Promise((done, fail) => {
        const now = () => JSON.stringify(window.agentdrafterViewer.view());
        let last = now();
        let since = performance.now();
        let frames = 0;
        let over = false;
        const giveUp = setTimeout(() => {
          over = true;
          fail(new Error(`the view did not come to rest in ${timeout} ms: ${now()}, ${frames} frames still`));
        }, timeout);
        const step = () => {
          if (over) return;
          const view = now();
          const t = performance.now();
          if (view !== last) [last, since, frames] = [view, t, 0];
          else frames += 1;
          if (frames >= 5 && t - since >= 300) {
            clearTimeout(giveUp);
            done(JSON.parse(view));
          } else requestAnimationFrame(step);
        };
        requestAnimationFrame(step);
      }),
    timeout,
  );
  // The page's answer, should it come after Node has given up, is let go.
  resting.catch(() => {});
  let timer;
  const deadline = new Promise((_, fail) => {
    timer = setTimeout(() => fail(new Error(`the view did not come to rest in ${timeout} ms: the page gave no answer in ${timeout + 5000} ms`)), timeout + 5000);
  });
  try {
    return await Promise.race([resting, deadline]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Whether the journal's `entry` finished: marked done, or written whole, as a
 * failed attempt or a discard is, with no state. One pending has not
 * finished yet. One rolled back changed nothing, so a wait for it to finish
 * fails at once and says so, rather than taking it for done (`journal.rs`).
 */
const finished = (entry) => {
  if (entry.state === 'rolled_back') throw new Error(`entry ${entry.entry} was rolled back, not done: ${JSON.stringify(entry)}`);
  return entry.state === undefined || entry.state === 'done';
};

/**
 * Whether the journal's `entries` hold more than `before` and the newest
 * has finished. A change set's entry is written pending before its files
 * are put in place (`commit.rs`), and `journal` can read it so: its files
 * are in place only once it is done (#254).
 */
const grownAndDone = (entries, before) => entries.length > before && finished(entries.at(-1));

/** Opens the side panel's Journal tab: it opens on the assistant's Chat (#156). */
async function journalTab(page) {
  await sideTab(page, 'journal');
  await page.waitForSelector('.va-jhead');
}

/**
 * Every server a check started, stopped when the run ends: at its end, on
 * an error, or on SIGINT, SIGTERM, SIGHUP (its terminal closed), SIGQUIT,
 * SIGUSR2 or SIGALRM, or on Windows SIGBREAK (Ctrl+Break). A kill no
 * process can catch, SIGKILL, or a signal not listed here, can leave one
 * running.
 */
const servers = new Set();
const stopAll = () => {
  for (const child of servers) if (child.exitCode === null) child.kill();
};
process.on('exit', stopAll);
const signals =
  process.platform === 'win32' ? ['SIGINT', 'SIGTERM', 'SIGHUP', 'SIGBREAK'] : ['SIGINT', 'SIGTERM', 'SIGHUP', 'SIGQUIT', 'SIGUSR2', 'SIGALRM'];
for (const signal of signals) {
  process.on(signal, () => {
    stopAll();
    process.exit(1);
  });
}

/** `agentdrafter serve` in the scratch folder, or `cwd`, with every line it printed; `env` its environment, by default this one's. */
async function serve(listen, cwd = scratch, env = process.env) {
  const child = spawn(bin, ['serve', '--listen', listen], { cwd, env, stdio: ['ignore', 'pipe', 'pipe'] });
  servers.add(child);
  child.on('exit', () => servers.delete(child));
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
      // A paper-space layout, model space through its viewports, and thick
      // lines cut at a viewport's edge.
      { name: 'layout', source: 'layout.yaml', png: 'layout.yaml', dpi: 100 },
      { name: 'viewport-clip', source: 'viewport-clip.yaml', png: 'viewport-clip.yaml', dpi: 200 },
      // Viewports clipped to a circle and to a notched polygon, cutting
      // thick lines and a fill at their boundaries.
      { name: 'viewport-boundary', source: 'viewport-boundary.yaml', png: 'viewport-boundary.yaml', dpi: 200 },
      // The second of a sheet's two layouts, asked for by its name.
      { name: 'layouts-Wiring', source: 'layouts.yaml', layout: 'Wiring', png: 'layouts.yaml', dpi: 100 },
    ];
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    const results = [];
    try {
      const context = await b.newContext(LAPTOP);
      for (const c of cases) {
        const asked = join(out, `png-${c.name}.png`);
        const rendered = agentdrafter('render', c.png, '--output', asked, '--overwrite', '--scale', '1', '--dpi', String(c.dpi));
        // A sheet drawn in several layouts renders each to a PNG of its own.
        const render = c.layout
          ? [rendered, ...(rendered.other_layouts ?? [])].find((r) => r.layout === c.layout)
          : rendered;
        assert(render, `${c.name}: the render gives layout ${c.layout}`);
        const file = render.png;
        const query = new URLSearchParams({
          source: c.source,
          ...(c.sheet ? { sheet: c.sheet } : {}),
          ...(c.layout ? { layout: c.layout } : {}),
        });
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

  /**
   * A sheet drawn in several layouts shows each as a tab under the sheet's,
   * the first shown first; another is drawn when its tab is picked, and the
   * address names it, so a reload shows it again.
   */
  async layouts() {
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    try {
      const context = await b.newContext(PHONE);
      const { page, problems } = await open(context, `${server.base}/?source=layouts.yaml`, null);
      const layoutTabs = () => page.evaluate(() => [...document.querySelectorAll('.va-layouts .tab')].map((t) => [t.textContent, t.classList.contains('on')]));
      const first = await layoutTabs();
      assert(JSON.stringify(first) === JSON.stringify([['Assembly', true], ['Wiring', false]]), `the layout tabs: ${JSON.stringify(first)}`);
      assert((await page.evaluate(() => window.agentdrafterViewer.summary.layout)) === 'Assembly', 'the first layout is shown first');
      await page.click('.va-layouts .tab:has-text("Wiring")');
      await page.waitForFunction(() => window.agentdrafterViewer?.summary?.layout === 'Wiring');
      const second = await layoutTabs();
      assert(JSON.stringify(second) === JSON.stringify([['Assembly', false], ['Wiring', true]]), `the layout tabs: ${JSON.stringify(second)}`);
      assert(new URL(page.url()).searchParams.get('layout') === 'Wiring', `the address names the layout: ${page.url()}`);
      await page.screenshot({ path: join(out, 'layouts-phone.png') });
      // A sheet of one layout has no layout tabs.
      const one = await open(context, `${server.base}/?source=layout.yaml`, null);
      assert((await one.page.evaluate(() => document.querySelectorAll('.va-layouts').length)) === 0, 'one layout, no layout tabs');
      // On a laptop they sit at the left of the status bar, as AutoCAD's do.
      const laptop = await open(await b.newContext(LAPTOP), `${server.base}/?source=layouts.yaml`, null);
      const inStatus = await laptop.page.evaluate(() => [...document.querySelectorAll('.status .va-layouts .tab')].map((t) => t.textContent));
      assert(JSON.stringify(inStatus) === JSON.stringify(['Assembly', 'Wiring']), `the layout tabs in the status bar: ${JSON.stringify(inStatus)}`);
      // A snap taken in Assembly, on a-frame's corner: another layout
      // drawn takes the read-out and its Copy away, though the sheet still
      // holds a-frame.
      {
        const listed = agentIn(scratch, 'display-list', 'layouts.yaml', '--layout', 'Assembly', '--snaps').result;
        const frame = listed.objects.findIndex((o) => o.id?.endsWith('/a-frame'));
        const corner = listed.snaps.find((s) => s.object === frame && s.snap === 'endpoint');
        assert(corner, `a-frame has an endpoint: ${frame}`);
        await laptop.page.waitForFunction(() => window.agentdrafterViewer?.summary?.snaps > 0);
        const [fx, fy] = await laptop.page.evaluate(([x, y]) => {
          const v = window.agentdrafterViewer.view();
          const r = document.querySelector('.stage').getBoundingClientRect();
          return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
        }, corner.at);
        await laptop.page.mouse.move(fx + 1, fy + 1);
        await laptop.page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
        await laptop.page.mouse.click(fx + 1, fy + 1);
        await laptop.page.waitForSelector('.opt[data-opt="copy"]');
      }
      await laptop.page.click('.status .va-layouts .tab:has-text("Wiring")');
      await laptop.page.waitForFunction(() => window.agentdrafterViewer?.summary?.layout === 'Wiring');
      assert(!(await laptop.page.$('.opt[data-opt="copy"]')), 'Copy goes with the layout its point was taken in');
      await laptop.page.screenshot({ path: join(out, 'layouts-laptop.png') });
      // Ten layouts at 900 CSS pixels: the strip scrolls; Grid, Object snap,
      // Ortho and the connection stay on the screen.
      const narrow = await open(await b.newContext({ viewport: { width: 900, height: 700 }, deviceScaleFactor: 1 }), `${server.base}/?source=ten-layouts.yaml`, null);
      const strip = await narrow.page.evaluate(() => {
        const s = document.querySelector('.status .va-layouts');
        const inSight = (sel) => {
          const r = document.querySelector(sel).getBoundingClientRect();
          return r.left >= 0 && r.right <= innerWidth + 0.5 && r.width > 0;
        };
        return {
          tabs: s.querySelectorAll('.tab').length,
          scrolls: s.scrollWidth > s.clientWidth,
          controls: ['[data-toggle="grid"]', '[data-toggle="osnap"]', '[data-toggle="ortho"]', '.status .live'].map((sel) => [sel, inSight(`.status ${sel.startsWith('.status') ? sel.slice(8) : sel}`)]),
        };
      });
      await narrow.page.screenshot({ path: join(out, 'layouts-ten-at-900.png') });
      assert(strip.tabs === 10 && strip.scrolls, `ten layout tabs, scrolling in their strip: ${JSON.stringify(strip)}`);
      assert(strip.controls.every(([, ok]) => ok), `the status controls stay on the screen: ${JSON.stringify(strip)}`);
      assert(narrow.problems.length === 0, narrow.problems.join('; '));
      // At 900 CSS pixels, in a wider face (letters spaced 0.5 px apart, as
      // DejaVu Sans is wider than this machine's), the read-out of a snap
      // with a long name stays whole: the paper and the layouts give way.
      {
        const wide = await b.newContext({ viewport: { width: 900, height: 700 }, deviceScaleFactor: 1 });
        await wide.addInitScript(() =>
          localStorage.setItem('agentdrafter-object-snaps', JSON.stringify({ on: true, running: ['endpoint', 'midpoint', 'center', 'quadrant', 'intersection'] })),
        );
        const quad = await open(wide, `${server.base}/?source=layouts.yaml`, null);
        await quad.page.waitForFunction(() => window.agentdrafterViewer?.summary?.snaps > 0);
        await quad.page.evaluate(() => {
          for (const el of document.querySelectorAll('.status, .status *')) el.style.letterSpacing = '0.5px';
        });
        const [qx, qy] = await quad.page.evaluate(() => {
          const v = window.agentdrafterViewer.view();
          const r = document.querySelector('.stage').getBoundingClientRect();
          return [r.left + (175 - v.left) * v.scale, r.top + (v.top - 195) * v.scale];
        });
        await quad.page.mouse.move(qx + 1, qy + 1);
        await quad.page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
        const readout = await quad.page.evaluate(() => {
          const c = document.querySelector('.status .coords');
          const r = c.getBoundingClientRect();
          return { text: c.textContent, whole: c.scrollWidth <= c.clientWidth + 0.5, right: r.right, live: document.querySelector('.status .live').getBoundingClientRect().right };
        });
        await quad.page.screenshot({ path: join(out, 'layouts-quadrant-at-900.png') });
        assert(
          readout.text === '175.0000, 195.0000 mmQuadrant' && readout.whole && readout.live <= 900.5,
          `the read-out is whole and the connection on the screen: ${JSON.stringify(readout)}`,
        );
        assert(quad.problems.length === 0, quad.problems.join('; '));
        await wide.close();
      }
      assert(problems.length === 0 && one.problems.length === 0 && laptop.problems.length === 0, [...problems, ...one.problems, ...laptop.problems].join('; '));
      return { first, second, laptop_status_bar: inStatus, ten_at_900: strip };
    } finally {
      await b.close();
      await server.stop();
    }
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
      // Each event carries the time the finger made it, as a real finger's
      // does: a gesture's moves 8 ms apart from its start, however long the
      // page takes to handle each. Without them, a slow page acknowledges
      // the finger going down so late that, by its events, it rested before
      // moving, as a held finger does.
      let finger = 0;
      const touch = (type, points) => {
        finger = type === 'touchStart' ? Date.now() / 1000 : finger + 0.008;
        return cdp.send('Input.dispatchTouchEvent', {
          type,
          timestamp: finger,
          touchPoints: points.map(([x, y], id) => ({ x, y, id, radiusX: 2, radiusY: 2, force: 1 })),
        });
      };
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
      // The drag flings on release, as far as the pointer's speed takes it.
      const dragged = await viewAtRest(page);
      assert(dragged.left > wheeled.left, 'a drag pans the sheet with the pointer');
      // A double click on a text opens it to edit, not to zoom (#261): it
      // goes where the renderer picks nothing, the nearest such point to
      // (400, 400) on the canvas, wherever the fling left the sheet.
      const empty = await page.evaluate(() => {
        const probe = window.agentdrafterViewer;
        const v = probe.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        for (let reach = 0; reach <= 300; reach += 8) {
          for (let turn = 0; turn < 360; turn += 15) {
            const x = 400 + reach * Math.cos((turn * Math.PI) / 180);
            const y = 400 + reach * Math.sin((turn * Math.PI) / 180);
            const onStage = document.elementFromPoint(x, y)?.closest('.stage');
            if (onStage && probe.pickAt(v.left + (x - r.left) / v.scale, v.top - (y - r.top) / v.scale, 16) < 0) return [x, y];
          }
        }
        return null;
      });
      assert(empty, 'a point near (400, 400) picks nothing');
      await page.mouse.dblclick(...empty);
      await page.waitForFunction((was) => window.agentdrafterViewer.view().scale > was * 1.8, dragged.scale, { timeout: 15000 }).catch(() => {});
      const doubled = await viewAtRest(page);
      assert(doubled.scale > dragged.scale * 1.8, `a double click zooms in: ${dragged.scale} to ${doubled.scale}, at ${empty}`);
      await page.click('[aria-label="Fit the sheet"]');
      const back = await viewAtRest(page);
      assert(Math.abs(back.scale - fitted.scale) / fitted.scale < 1e-6, 'fit shows the whole sheet again');
      await page.click('[aria-label="Zoom in"]');
      assert((await viewAtRest(page)).scale > fitted.scale * 1.5, 'the zoom button zooms in');
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
        // The command bar under the drawing, taken one line tall, with a
        // phone's peek under it; and on a laptop the zoom buttons at the
        // right, beside the middle of the drawing.
        const bar = document.querySelector('.cmdbar').getBoundingClientRect();
        const below = c.bottom - (bar.bottom - 32);
        const zoom = document.querySelector('.zoombtns').getBoundingClientRect();
        const beside = zoom.top < c.top + c.height / 2 && zoom.bottom > c.top + c.height / 2;
        return {
          paper: [(left - v.left) * v.scale, (v.top - top) * v.scale, (right - v.left) * v.scale, (v.top - bottom) * v.scale],
          room: [beside ? zoom.left - c.left : c.width, c.height - below],
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
      await journalTab(page);
      await page.click('button[aria-label="Older"]');
      const at53 = await left(53);
      assert(Math.abs(at53 - now.bounds[0]) < 1e-6, 'entry 53 is the set as it is');
      // The header's status chip, amber, says which entry is shown; nothing
      // is drawn over the drawing, which looks a little warmer; Undo and
      // Accept are off.
      const stepping = await page.evaluate(() => ({
        chip: document.querySelector('.pill.past')?.textContent ?? null,
        over: [...document.querySelectorAll('.va-overlays > :not(.va-toast)')].length,
        filter: getComputedStyle(document.querySelector('.stage')).filter,
        undo: document.querySelector('button[data-act="undo"]').disabled,
        accept: document.querySelector('button[data-act="accept"]').disabled,
      }));
      assert(stepping.chip === 'Entry 53 of 53 · Live ›', `the chip says which entry is shown: ${stepping.chip}`);
      assert(stepping.over === 0, 'nothing but a toast is over the drawing');
      assert(stepping.filter.includes('sepia'), `the drawing looks warmer: ${stepping.filter}`);
      assert(stepping.undo && stepping.accept, 'Undo and Accept are off while stepping');
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
      // The command bar's way out works over the overlay: Newer, here to
      // entry 2, pruned too.
      await page.click('.opt[data-opt="newer"]', { timeout: 5000 });
      await page.waitForFunction(() => document.querySelector('.pill.past')?.textContent?.startsWith('Entry 2 of 53'), null, { timeout: 15000 });
      // Back to live, from the chip.
      await page.click('.pill.past');
      await page.waitForFunction(() => window.agentdrafterViewer.summary.entry === null && !document.querySelector('.va-cannot') && !document.querySelector('.pill.past'));
      assert(
        await page.evaluate(() => !document.querySelector('button[data-act="accept"]').disabled && !getComputedStyle(document.querySelector('.stage')).filter.includes('sepia')),
        'live again: Accept is on and the drawing as it is',
      );
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

  /** A tap on an insert, at phone size, shows its ID, name, layer and attributes in quick properties. */
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
      await page.waitForSelector('.qp', { timeout: 15000 });
      const shown = await page.evaluate(() => {
        const rows = {};
        for (const tr of document.querySelectorAll('.qp .kv tr')) {
          // A row's label, without the chip under it; an editable row's value is its field's.
          const th = tr.querySelector('th')?.firstChild?.textContent;
          const field = tr.querySelector('td input, td select, td textarea');
          if (th) rows[th] = field ? field.value : tr.querySelector('td')?.textContent;
        }
        return rows;
      });
      assert(shown.ID === 'DP-LD-101/device', `quick properties show its ID: ${JSON.stringify(shown)}`);
      assert(shown.Name === 'FT-101', 'its name');
      assert(shown.Layer === '0', 'its layer');
      assert(shown.Type === 'Insert of TRANSMITTER', 'its block');
      assert(shown.FUNC === 'FT' && shown.LOOP === '101', 'its attributes');
      const marked = await page.evaluate(() => window.agentdrafterViewer.marked);
      assert(marked.picked === device.place, 'the picked object is marked');
      await sleep(300);
      await page.screenshot({ path: join(out, 'pick-phone.png') });
      // On a phone, the device is brought up above the card.
      const placed = await page.evaluate((place) => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        const [, y0, , y1] = window.agentdrafterViewer.bounds([place]);
        return { device: r.top + (v.top - (y0 + y1) / 2) * v.scale, card: document.querySelector('.qp').getBoundingClientRect().top };
      }, device.place);
      assert(placed.device < placed.card, `the device is above quick properties: ${JSON.stringify(placed)}`);
      // A tap where nothing is drawn, above the paper, picks nothing, and
      // quick properties close.
      const bare = await page.evaluate(() => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + 20, Math.max(r.top + 12, r.top + (v.top - 11) * v.scale - 40)];
      });
      await page.touchscreen.tap(bare[0], bare[1]);
      await page.waitForFunction(() => !document.querySelector('.qp'));
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
      // At 320 CSS pixels the chips wrap: none is clipped.
      await page.setViewportSize({ width: 320, height: 700 });
      await sleep(300);
      const chips = await page.evaluate(() =>
        [...document.querySelectorAll('.va-status .pill')].map((c) => {
          const r = c.getBoundingClientRect();
          return { text: c.textContent, left: r.left, right: r.right, fits: c.scrollWidth <= c.clientWidth + 1 };
        }),
      );
      await page.screenshot({ path: join(out, 'chips-at-320.png') });
      assert(chips.length >= 2 && chips.every((c) => c.left >= 0 && c.right <= 320.5 && c.fits), `no chip is clipped at 320: ${JSON.stringify(chips)}`);
      await page.setViewportSize({ width: 390, height: 844 });

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
      // Each time the set as it is now is asked for its DP-LD-101.
      const drawnLive = [];
      await context.route('**/commands/display-list', async (route) => {
        const input = route.request().postDataJSON() ?? {};
        if (input.sheet === 'DP-LD-101' && input.entry === undefined) drawnLive.push(performance.now());
        await route.continue();
      });
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

      // A journal whose first entry is taken away, its newest still the
      // same number: [1, 2] to [2]. The drawings kept are dropped too, and
      // the sheet shown is asked for again.
      assert(sendChange(folder, move('DP-LD-101/device', 0.5)).ok, 'change set 2, again');
      await at(start.bounds[0] + 1);
      await page.waitForFunction(() => document.querySelector('.va-jhead')?.textContent?.includes('2 entries'));
      await sleep(500);
      const asked = drawnLive.length;
      // The first entry's lines: the entry, written pending, and the line
      // that marks it done.
      const lines = readFileSync(journalFile, 'utf8').split('\n');
      const first = JSON.parse(lines[0]).entry;
      writeFileSync(journalFile, lines.filter((line) => line === '' || JSON.parse(line).entry !== first).join('\n'));
      await page.waitForFunction(() => document.querySelector('.va-jhead')?.textContent?.includes('1 entry'), null, { timeout: 15000 });
      for (let waited = 0; drawnLive.length <= asked && waited < 15000; waited += 100) await sleep(100);
      assert(drawnLive.length > asked, `the sheet shown is asked for again once the journal's first entry is gone: ${drawnLive.length} after ${asked}`);
      assert(problems.length === 0, problems.join('; '));
      return { cut_back_to: 1, first_entry_taken_away: true };
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
      await page.waitForSelector('.qp');
      const shownId = () =>
        page.evaluate(() => {
          for (const tr of document.querySelectorAll('.qp .kv tr')) {
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
      await page.waitForFunction(() => !document.querySelector('.qp') && window.agentdrafterViewer.marked?.picked === null, null, { timeout: 15000 });
      assert(problems.length === 0, problems.join('; '));
      return { followed: 'DP-LD-101/io-channel', cleared: true };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * The drafting app's frame, at laptop and phone sizes: what each part
   * shows and does before anything edits.
   */
  async frame() {
    const server = await serve('127.0.0.1:0');
    const b = await browser();
    const seen = {};
    // Where an object of the sheet shown is on the page.
    const at = (page, id) =>
      page.evaluate((wanted) => {
        const probe = window.agentdrafterViewer;
        const place = probe.summary.objects.findIndex((o) => o.id === wanted);
        const [x0, y0, x1, y1] = probe.bounds([place]);
        const v = probe.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return {
          place,
          box: [r.left + (x0 - v.left) * v.scale, r.top + (v.top - y1) * v.scale, r.left + (x1 - v.left) * v.scale, r.top + (v.top - y0) * v.scale],
        };
      }, id);
    const middle = (box) => [(box[0] + box[2]) / 2, (box[1] + box[3]) / 2];
    const bar = (page) => page.evaluate(() => document.querySelector('.cmdbar').textContent);
    try {
      // A laptop.
      {
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        const ribbon = await page.evaluate(() =>
          [...document.querySelectorAll('.ribbon .panel')].map((p) => [p.querySelector('h3').textContent.trim(), [...p.querySelectorAll('.rb')].map((t) => t.textContent)]),
        );
        assert(
          JSON.stringify(ribbon) === JSON.stringify([['Draw', ['Line', 'Circle', 'Rectangle', 'Text']], ['Modify', ['Move', 'Copy', 'Array', 'Erase']], ['Layer', []]]),
          `the ribbon: ${JSON.stringify(ribbon)}`,
        );
        assert(
          (await bar(page)).includes('Type a command, such as L for Line, or select an object, or click a snap to read its point'),
          `the command bar prompts: ${await bar(page)}`,
        );
        // A tool: its tip on hover, and its prompt in the command bar when clicked, until Esc.
        await page.hover('.rb[data-tool="rectangle"]');
        await page.waitForSelector('.tip');
        const tip = await page.textContent('.tip');
        assert(tip.includes('REC') && tip.includes('opposite corner'), `the tip names the alias and says what it does: ${tip}`);
        await page.click('.rb[data-tool="rectangle"]', { force: true });
        assert((await bar(page)).includes('RECTANGon 0 Click the first corner'), `the command bar prompts for the tool: ${await bar(page)}`);
        assert((await page.getAttribute('.rb[data-tool="rectangle"]', 'aria-pressed')) === 'true', 'the tool shows pressed');
        await page.keyboard.press('Escape');
        assert(!(await bar(page)).includes('RECTANG'), `Esc ends it: ${await bar(page)}`);
        // From the keyboard, with no hover: Enter on a tool starts it.
        await page.focus('.rb[data-tool="circle"]');
        await page.keyboard.press('Enter');
        assert((await bar(page)).includes('CIRCLEon 0 Click the centre'), `Enter on a tool starts it: ${await bar(page)}`);
        await page.click('.opt[data-opt="tool:cancel"]');
        // The status bar: the cursor's place, the paper, and the toggles.
        const stage = await page.evaluate(() => {
          const r = document.querySelector('.stage').getBoundingClientRect();
          return [r.left + 300, r.top + 200];
        });
        // Snapping off, so the cursor's own place is read, not a snap's.
        await page.keyboard.press('F3');
        await page.mouse.move(stage[0], stage[1]);
        await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
        const coords = await page.textContent('.coords');
        const expected = await page.evaluate(([x, y]) => {
          const v = window.agentdrafterViewer.view();
          const r = document.querySelector('.stage').getBoundingClientRect();
          return `${(v.left + (x - r.left) / v.scale).toFixed(4)}, ${(v.top - (y - r.top) / v.scale).toFixed(4)} in`;
        }, stage);
        assert(coords === expected, `the status bar says where the cursor is: ${coords}, not ${expected}`);
        // The view moves under a still pointer, as a fling does: the coordinates follow.
        await page.evaluate(() => {
          const v = window.agentdrafterViewer.view();
          window.agentdrafterViewer.show({ ...v, left: v.left - 1.5 });
        });
        await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
        const followed = await page.textContent('.coords');
        const underPointer = await page.evaluate(([x, y]) => {
          const v = window.agentdrafterViewer.view();
          const r = document.querySelector('.stage').getBoundingClientRect();
          return `${(v.left + (x - r.left) / v.scale).toFixed(4)}, ${(v.top - (y - r.top) / v.scale).toFixed(4)} in`;
        }, stage);
        assert(followed === underPointer && followed !== coords, `the coordinates follow a view moved under the pointer: ${followed}, not ${underPointer}`);
        await page.keyboard.press('F3');
        assert((await page.textContent('.paperlabel')) === 'ANSI B landscape, inches', 'the paper and its units');
        const pressed = (k) => page.getAttribute(`.status [data-toggle="${k}"]`, 'aria-pressed');
        assert((await pressed('grid')) === 'true' && (await pressed('osnap')) === 'true' && (await pressed('ortho')) === 'false', 'Grid and Object snap start on, Ortho off');
        // Grid: F7 takes the grid away from the drawing, and puts it back.
        const gridPixels = () =>
          page.evaluate(() => {
            const p = window.agentdrafterViewer;
            const s = p.summary;
            const raw = atob(p.capture({ left: s.paper[0] - 1, top: s.paper[3] + 1, scale: 40, width: 400, height: 300, white: false }));
            let grid = 0;
            for (let i = 0; i < raw.length; i += 4) {
              const rgb = [raw.charCodeAt(i), raw.charCodeAt(i + 1), raw.charCodeAt(i + 2)].join();
              if (rgb === '39,47,57' || rgb === '49,58,70') grid++;
            }
            return grid;
          });
        const gridOn = await gridPixels();
        await page.keyboard.press('F7');
        const gridOff = await gridPixels();
        assert(gridOn > 5000 && gridOff === 0 && (await pressed('grid')) === 'false', `F7 takes the grid away: ${gridOn} then ${gridOff}`);
        await page.click('.status [data-toggle="grid"]');
        assert((await gridPixels()) === gridOn && (await pressed('grid')) === 'true', 'Grid puts it back');
        // The paper's dashed edge is one CSS pixel wide: one row on top, one
        // column at the left. On a made-up sheet drawn on its extents, where
        // no ink lies on the edge, as the demo plant's own does.
        const bare = await open(context, `${server.base}/?source=overlap.yaml`, null);
        const edge = await bare.page.evaluate(() => {
          const p = window.agentdrafterViewer;
          const s = p.summary;
          const scale = 40;
          const raw = atob(p.capture({ left: s.paper[0] - 20 / scale, top: s.paper[3] + 20 / scale, scale, width: 60, height: 60, white: false }));
          const isEdge = (x, y) => {
            const i = (y * 60 + x) * 4;
            return [raw.charCodeAt(i), raw.charCodeAt(i + 1), raw.charCodeAt(i + 2)].join() === '86,96,110';
          };
          let rows = 0;
          let columns = 0;
          for (let y = 10; y < 30; y++) if (isEdge(22, y)) rows++;
          for (let x = 10; x < 30; x++) if (isEdge(x, 22)) columns++;
          return { rows, columns, top: isEdge(22, 20), left: isEdge(20, 22) };
        });
        assert(edge.rows === 1 && edge.columns === 1 && edge.top && edge.left, `the paper's edge is one pixel wide, on the paper's edge: ${JSON.stringify(edge)}`);
        assert(bare.problems.length === 0, bare.problems.join('; '));
        await bare.page.close();
        await page.keyboard.press('F3');
        await page.keyboard.press('F8');
        assert((await pressed('osnap')) === 'false' && (await pressed('ortho')) === 'true', 'F3 and F8 turn Object snap and Ortho');
        // Model space is dark, and colour 7 draws white on it, the paper's edge a dashed outline.
        const screen = await page.evaluate(() => {
          const p = window.agentdrafterViewer;
          const s = p.summary;
          const width = 600;
          const scale = width / (s.paper[2] - s.paper[0] + 2);
          const height = Math.round((s.paper[3] - s.paper[1] + 2) * scale);
          const raw = atob(p.capture({ left: s.paper[0] - 1, top: s.paper[3] + 1, scale, width, height, white: false }));
          let white = 0;
          let black = 0;
          for (let i = 0; i < raw.length; i += 4) {
            const [r, g, bl] = [raw.charCodeAt(i), raw.charCodeAt(i + 1), raw.charCodeAt(i + 2)];
            if (r > 240 && g > 240 && bl > 240) white++;
            if (r < 16 && g < 16 && bl < 16) black++;
          }
          const corner = [raw.charCodeAt(0), raw.charCodeAt(1), raw.charCodeAt(2)];
          return { white, black, corner };
        });
        assert(screen.white > 2000 && screen.black === 0, `colour 7 draws white on dark model space: ${JSON.stringify(screen)}`);
        // Selecting: quick properties beside the object, the Layer box, the command bar.
        const device = await at(page, 'DP-LD-101/device');
        await page.mouse.click(...middle(device.box));
        await page.waitForSelector('.qp');
        await page.waitForFunction(() => getComputedStyle(document.querySelector('.qp')).visibility === 'visible');
        const card = await page.evaluate(() => {
          const r = document.querySelector('.qp').getBoundingClientRect();
          return [r.left, r.top, r.right, r.bottom];
        });
        const apart = card[0] >= device.box[2] || card[2] <= device.box[0];
        assert(apart && card[1] < device.box[3] + 1, `quick properties sit beside the device: ${JSON.stringify({ card, device: device.box })}`);
        assert((await page.inputValue('select[data-layer]')) === '0', 'the Layer box shows its layer');
        assert((await bar(page)).includes('FT-101 selected [ Move Copy Array Erase Clear ]'), `the command bar names it: ${await bar(page)}`);
        assert((await page.textContent('.qp')).includes('Comes from'), 'quick properties say where it comes from');
        await page.screenshot({ path: join(out, 'frame-laptop-selected.png') });
        // They follow it as the view moves.
        await page.click('[aria-label="Zoom in"]');
        await sleep(500);
        const moved = await at(page, 'DP-LD-101/device');
        const card2 = await page.evaluate(() => {
          const r = document.querySelector('.qp').getBoundingClientRect();
          return [r.left, r.top, r.right, r.bottom];
        });
        assert(card2[0] >= moved.box[2] - 1 || card2[2] <= moved.box[0] + 1, `quick properties follow the device: ${JSON.stringify({ card2, device: moved.box })}`);
        await page.click('.opt[data-opt="clear"]');
        await page.waitForFunction(() => !document.querySelector('.qp'));
        assert((await page.inputValue('select[data-layer]')) === '0', 'with nothing selected, the Layer box sets the current layer, layer 0');
        // The side panel: the assistant's Chat, first, and the Journal.
        assert(await page.isVisible('.stab[data-side="chat"].on'), 'the panel opens on the assistant’s Chat');
        assert((await page.textContent('.ahrow')).startsWith('Assistant'), 'the panel is the assistant’s');
        await sideTab(page, 'journal');
        await page.waitForSelector('.va-jhead');
        assert(await page.isVisible('.titlebar button[data-act="undo"]') && await page.isVisible('.titlebar button[data-act="accept"]'), 'Undo last and Accept are in the title bar');
        // Last, as it may leave no renderer: a lost WebGL context, restored,
        // with Grid off. The renderer made anew draws with Grid off, as the
        // page has it. A browser that never restores a lost context, as
        // headless Chromium with SwiftShader does not, is recorded, not failed.
        await page.keyboard.press('F7');
        const restore = await page.evaluate(async () => {
          const within = (promise, what) =>
            Promise.race([promise.then(() => null), new Promise((done) => setTimeout(() => done(`no ${what} within 5 s`), 5000))]);
          const canvas = document.querySelector('.stage');
          const lose = canvas.getContext('webgl2')?.getExtension('WEBGL_lose_context');
          if (!lose) return 'no WEBGL_lose_context';
          const lost = new Promise((done) => canvas.addEventListener('webglcontextlost', done, { once: true }));
          lose.loseContext();
          const notLost = await within(lost, 'webglcontextlost');
          if (notLost) return notLost;
          // WebGL allows a restore only on a later task than the loss.
          await new Promise((done) => setTimeout(done, 0));
          const back = new Promise((done) => canvas.addEventListener('webglcontextrestored', done, { once: true }));
          try {
            lose.restoreContext();
          } catch (e) {
            return `restoreContext threw: ${e}`;
          }
          const notBack = await within(back, 'webglcontextrestored');
          if (notBack) return notBack;
          await new Promise((done) => setTimeout(done, 300));
          return null;
        });
        let contextRestored = `not restored by this browser: ${restore}`;
        if (restore === null) {
          const afterRestore = await gridPixels();
          assert(afterRestore === 0, `after a lost context the grid stays off: ${afterRestore}`);
          contextRestored = 'restored, the grid still off';
        } else {
          assert(restore === 'no webglcontextrestored within 5 s', `the context is lost: ${restore}`);
        }
        assert(problems.length === 0, problems.join('; '));
        seen.laptop = { ribbon, coords, grid_pixels: [gridOn, gridOff], screen, card, context_restored: contextRestored };
        await context.close();
      }
      // A phone.
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        assert(!(await page.isVisible('.ribbon')) && !(await page.$('.status')), 'the ribbon is folded and there is no status bar');
        await page.tap('.toolsbtn');
        assert(await page.isVisible('.ribbon'), 'Tools opens the ribbon');
        const toggles = await page.evaluate(() => [...document.querySelectorAll('.ribbon .tog2')].map((t) => t.textContent));
        assert(JSON.stringify(toggles) === JSON.stringify(['Grid', 'Object snap', 'Ortho', 'Snaps…']), `Tools holds the toggles and the snaps' menu: ${toggles}`);
        await page.screenshot({ path: join(out, 'frame-phone-tools.png') });
        await page.tap('.rb[data-tool="erase"]', { force: true });
        assert(!(await page.isVisible('.ribbon')), 'a tool tapped folds the ribbon');
        assert((await bar(page)).includes('ERASETap the objects to erase'), `the command bar prompts for Erase: ${await bar(page)}`);
        await page.tap('.opt[data-opt="tool:cancel"]');
        const device = await at(page, 'DP-LD-101/device');
        await page.touchscreen.tap(...middle(device.box));
        await page.waitForSelector('.qp');
        await sleep(500);
        const placed = await page.evaluate(() => {
          const qp = document.querySelector('.qp').getBoundingClientRect();
          const cmd = document.querySelector('.cmdbar').getBoundingClientRect();
          const peek = document.querySelector('.va-peek').getBoundingClientRect();
          return { qp: [qp.left, qp.top, qp.right, qp.bottom], cmd: [cmd.top, cmd.bottom], peek: peek.top, width: innerWidth };
        });
        assert(placed.qp[3] <= placed.cmd[0] && placed.cmd[1] <= placed.peek && placed.qp[0] >= 0 && placed.qp[2] <= placed.width, `on a phone the card sits above the command bar, above the peek: ${JSON.stringify(placed)}`);
        await page.screenshot({ path: join(out, 'frame-phone-selected.png') });
        // The selected object's line is one line, whatever the font: four
        // phone edit options (Clear dropped on phone; QP × clears) and the
        // name on one row, the name giving way first (#66, #260, #247).
        const line = await page.evaluate(() => {
          const tops = [...document.querySelectorAll('.cmdline.oneline > *')]
            .filter((el) => getComputedStyle(el).display !== 'none')
            .map((el) => Math.round(el.getBoundingClientRect().top + el.getBoundingClientRect().height / 2));
          return { tops, options: document.querySelectorAll('.cmdline.oneline .opt').length };
        });
        assert(line.options === 4 && Math.max(...line.tops) - Math.min(...line.tops) <= 3, `the selected object's line is one line: ${JSON.stringify(line)}`);
        assert(await page.isVisible('.va-peek button[data-act="undo"]') && await page.isVisible('.va-peek button[data-act="accept"]'), 'Undo last and Accept are in the peek');
        // Fit stays in reach while something is selected: nothing covers it, and it fits.
        const fitButton = await page.evaluate(() => {
          const b = document.querySelector('[aria-label="Fit the sheet"]');
          const r = b.getBoundingClientRect();
          const top = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
          const qp = document.querySelector('.qp').getBoundingClientRect();
          return { reached: top === b || b.contains(top), apart: r.top >= qp.bottom || r.bottom <= qp.top || r.left >= qp.right || r.right <= qp.left };
        });
        assert(fitButton.reached && fitButton.apart, `Fit is reachable with quick properties open: ${JSON.stringify(fitButton)}`);
        await page.tap('[aria-label="Fit the sheet"]');
        await sleep(600);
        assert(await page.isVisible('.qp'), 'Fit leaves the selection');
        const covered = (box, qp) => box[0] < qp[2] && box[2] > qp[0] && box[1] < qp[3] && box[3] > qp[1];
        // The journal raised to half: quick properties sit above it, not behind.
        const grab = await page.evaluate(() => {
          const r = document.querySelector('.va-grab').getBoundingClientRect();
          return [r.left + r.width / 2, r.top + r.height / 2];
        });
        await page.touchscreen.tap(grab[0], grab[1]);
        await sleep(600);
        const raised = await page.evaluate(() => {
          const qp = document.querySelector('.qp').getBoundingClientRect();
          const sheet = document.querySelector('.va-sheet').getBoundingClientRect();
          const stage = document.querySelector('.stage').getBoundingClientRect();
          return { qp: [qp.top, qp.bottom], sheetTop: sheet.top, stageTop: stage.top };
        });
        await page.screenshot({ path: join(out, 'frame-phone-journal-raised.png') });
        assert(raised.qp[1] <= raised.sheetTop + 0.5 && raised.qp[0] >= raised.stageTop - 0.5 && raised.qp[1] - raised.qp[0] > 40, `quick properties sit above the raised journal: ${JSON.stringify(raised)}`);
        // The raised card has less room: the device is brought out from under it again.
        const raisedCard = await page.evaluate(() => {
          const r = document.querySelector('.qp').getBoundingClientRect();
          return [r.left, r.top, r.right, r.bottom];
        });
        const deviceRaised = (await at(page, 'DP-LD-101/device')).box;
        assert(!covered(deviceRaised, raisedCard), `the device is not under the raised card: ${JSON.stringify({ device: deviceRaised, card: raisedCard })}`);
        raised.device = deviceRaised;
        assert(problems.length === 0, problems.join('; '));
        seen.phone = { toggles, placed, raised };
        await context.close();
      }
      // A small phone, 320 by 568: even a long object name stays on one line
      // with ellipsis (#260, #247), QP × clears selection, and a long error
      // prompt wraps the command bar while quick properties stay above it.
      {
        const context = await b.newContext({ ...PHONE, viewport: { width: 320, height: 568 } });
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        // Long object name (`home-run-cable`, 14 chars) on a 320 px phone stays on 1 line with ellipsis (#260, #247).
        const longAt = await page.evaluate(() => {
          const p = window.agentdrafterViewer;
          const place = p.summary.objects.findIndex((o) => o.id === 'DP-LD-101/home-run-cable');
          const [x0, y0, x1, y1] = p.bounds([place]);
          const v = p.view();
          const r = document.querySelector('.stage').getBoundingClientRect();
          for (let j = 1; j < 8; j++) {
            for (let i = 1; i < 12; i++) {
              const x = x0 + ((x1 - x0) * i) / 12;
              const y = y0 + ((y1 - y0) * j) / 8;
              if (p.pickAt(x, y, 16) === place) return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
            }
          }
          return null;
        });
        assert(longAt, 'a point picks home-run-cable on a 320 px phone');
        await page.touchscreen.tap(longAt[0], longAt[1]);
        await page.waitForSelector('.qp');
        await sleep(300);
        const narrowLine = await page.evaluate(() => {
          const els = [...document.querySelectorAll('.cmdline.oneline > *')].filter((el) => getComputedStyle(el).display !== 'none');
          const tops = els.map((el) => Math.round(el.getBoundingClientRect().top + el.getBoundingClientRect().height / 2));
          const name = document.querySelector('.cmdline.oneline > span:first-child');
          return {
            tops,
            options: document.querySelectorAll('.cmdline.oneline .opt').length,
            clipped: !!name && name.scrollWidth > name.clientWidth,
            text: name?.textContent?.trim() ?? '',
          };
        });
        assert(
          narrowLine.options === 4 && Math.max(...narrowLine.tops) - Math.min(...narrowLine.tops) <= 3 && narrowLine.clipped && narrowLine.text === 'home-run-cable',
          `320 px phone keeps a long object name on one line with ellipsis: ${JSON.stringify(narrowLine)}`,
        );
        // Quick properties × clears the selection on a phone.
        await page.tap('.qp [data-close]');
        await page.waitForFunction(() => !document.querySelector('.qp'), null, { timeout: 10000 });
        const device = await at(page, 'DP-LD-101/device');
        await page.touchscreen.tap(...middle(device.box));
        await page.waitForSelector('.qp');
        // A selected object's options fill the bar; a keyboard types into the field all the same.
        await page.keyboard.type('12');
        await page.keyboard.press('Enter');
        await sleep(600);
        assert((await bar(page)).includes('is not a drawing tool'), `the word is not a tool: ${await bar(page)}`);
        const small = await page.evaluate(() => {
          const qp = document.querySelector('.qp').getBoundingClientRect();
          const cmd = document.querySelector('.cmdbar').getBoundingClientRect();
          return { qp: [qp.left, qp.top, qp.right, qp.bottom], cmd: [cmd.top, cmd.bottom] };
        });
        const deviceSmall = (await at(page, 'DP-LD-101/device')).box;
        await page.screenshot({ path: join(out, 'frame-phone-320-prompt-wrapped.png') });
        assert(small.cmd[1] - small.cmd[0] > 40, `the prompt wraps: ${JSON.stringify(small)}`);
        assert(small.qp[3] <= small.cmd[0] + 0.5, `quick properties stay above the wrapped command bar: ${JSON.stringify(small)}`);
        const [l, t, r, bt] = deviceSmall;
        assert(!(l < small.qp[2] && r > small.qp[0] && t < small.qp[3] && bt > small.qp[1]), `the device is not under the card: ${JSON.stringify({ device: deviceSmall, ...small })}`);
        assert(problems.length === 0, problems.join('; '));
        seen.small = { ...small, device: deviceSmall };
        await context.close();
      }
      // The drafter pans the selected object away while a long prompt is up;
      // when the prompt goes and the command bar shrinks, the view stays
      // where the drafter put it.
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        const device = await at(page, 'DP-LD-101/device');
        await page.touchscreen.tap(...middle(device.box));
        await page.waitForSelector('.qp');
        await sleep(500);
        // A selected object's options fill the bar; a keyboard types into the field all the same.
        await page.keyboard.type('12');
        await page.keyboard.press('Enter');
        await sleep(400);
        const stage = await page.evaluate(() => {
          const r = document.querySelector('.stage').getBoundingClientRect();
          return [r.left, r.top, r.width];
        });
        // Drag the drawing to the left until FT-101 is off the screen.
        const y = stage[1] + 60;
        await page.mouse.move(stage[0] + stage[2] - 20, y);
        await page.mouse.down();
        await page.mouse.move(stage[0] + 20, y, { steps: 6 });
        await sleep(30);
        await page.mouse.move(stage[0] + 10, y, { steps: 2 });
        // Held still before letting go, so the drag ends where it is, with no
        // fling to run on while the test reads the view; then the view is
        // read once it has stopped.
        await sleep(200);
        await page.mouse.up();
        // A view that never comes to rest fails the check, with what it
        // last showed, within 5 seconds, so the run goes on and stops its
        // servers.
        {
          const samples = [];
          const until = Date.now() + 5000;
          for (let still = 0, last = ''; still < 3; ) {
            assert(Date.now() < until, `the view comes to rest after the drag: ${samples.slice(-4).join(' ')}`);
            await sleep(100);
            const now = JSON.stringify(await page.evaluate(() => window.agentdrafterViewer.view()));
            samples.push(now);
            still = now === last ? still + 1 : 0;
            last = now;
          }
        }
        const away = await at(page, 'DP-LD-101/device');
        assert(away.box[2] < stage[0], `FT-101 is panned off the screen: ${JSON.stringify(away.box)}`);
        const panned = await page.evaluate(() => window.agentdrafterViewer.view());
        assert((await bar(page)).includes('is not a drawing tool'), 'the prompt is still up');
        // OK takes the prompt away; the bar shrinks.
        await page.tap('.opt[data-opt="tool:ok"]');
        await sleep(800);
        assert(!(await bar(page)).includes('is not a drawing tool'), `the prompt has gone: ${await bar(page)}`);
        const after = await page.evaluate(() => window.agentdrafterViewer.view());
        assert(
          Math.abs(after.left - panned.left) < 1e-9 && Math.abs(after.top - panned.top) < 1e-9 && after.scale === panned.scale,
          `the view stays where the drafter panned it: ${JSON.stringify({ panned, after })}`,
        );
        assert(problems.length === 0, problems.join('; '));
        seen.panned_away = { device: away.box, view_kept: true };
        await context.close();
      }
      // A phone held on its side: the card does not cover the selected object.
      {
        const context = await b.newContext({ ...PHONE, viewport: { width: 844, height: 390 } });
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        const device = await at(page, 'DP-LD-101/device');
        await page.touchscreen.tap(...middle(device.box));
        await page.waitForSelector('.qp');
        await sleep(700);
        const after = await at(page, 'DP-LD-101/device');
        const qp = await page.evaluate(() => {
          const r = document.querySelector('.qp').getBoundingClientRect();
          return [r.left, r.top, r.right, r.bottom];
        });
        const [l, t, r, bt] = after.box;
        const overlaps = l < qp[2] && r > qp[0] && t < qp[3] && bt > qp[1];
        await page.screenshot({ path: join(out, 'frame-phone-landscape-selected.png') });
        assert(!overlaps, `on its side, quick properties do not cover the device: ${JSON.stringify({ device: after.box, qp })}`);
        assert(problems.length === 0, problems.join('; '));
        seen.landscape = { device: after.box, qp };
        await context.close();
      }
      return {
        ...seen,
        screenshots: ['frame-laptop-selected.png', 'frame-phone-tools.png', 'frame-phone-selected.png', 'frame-phone-journal-raised.png', 'frame-phone-320-prompt-wrapped.png', 'frame-phone-landscape-selected.png'],
      };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * The drawing tools: each by mouse, by its alias and typed coordinates,
   * and by touch, against a copy of the demo plant. Each sends one change
   * set through `change`, which the journal records and
   * which builds the sheet's DWG again; the template warning's Cancel lands
   * nothing and its Change all lands on every sheet named; Undo last takes
   * a drafter's edit back.
   */
  async tools() {
    const folder = freshPlant('tools');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    const seen = { laptop: {}, phone: {} };
    const journal = () => agentIn(folder, 'journal', 'demo-plant').result.entries;
    const dwg = (sheet) => readFileSync(join(folder, 'demo-plant', 'dwg', `${sheet}.dwg`)).toString('base64');
    const template = () => readFileSync(join(folder, 'demo-plant', 'templates', 'loop-2-wire.yaml'), 'utf8');
    const bar = (page) => page.evaluate(() => document.querySelector('.cmdbar').textContent);
    const frames = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    // Where the drawing point `at` is on the page.
    const onPage = (page, at) =>
      page.evaluate(([x, y]) => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
      }, at);
    const snapOf = (id, snap, which) => {
      const args = ['query', 'snap', 'demo-plant', '--id', id, '--snap', snap];
      if (which) args.push('--which', which);
      const asked = agentIn(folder, ...args);
      assert(asked.ok && asked.result.points.length === 1, `query snap ${id}: ${JSON.stringify(asked)}`);
      return asked.result.points[0].at;
    };
    // Waits for the journal to hold one more entry than `before`, which
    // must be a change set that landed, and for the page
    // to draw the sheet with what it added, if it added an object.
    const landed = async (page, before, what, sheet = 'DP-LD-101') => {
      const until = Date.now() + 30000;
      let entries = journal();
      while (!grownAndDone(entries, before)) {
        assert(Date.now() < until, `${what}: no change set landed: ${await bar(page)}`);
        await sleep(100);
        entries = journal();
      }
      const entry = entries.at(-1);
      assert(entry.kind === 'change_set' && entry.landed_on?.includes(sheet), `${what} landed: ${JSON.stringify(entry)}`);
      const added = (entry.objects ?? []).filter((o) => o.how === 'added' && o.id.startsWith(`${sheet}/`)).map((o) => o.id);
      for (const id of added) {
        await page.waitForFunction((wanted) => window.agentdrafterViewer.summary.objects.some((o) => o.id === wanted), id, { timeout: 15000 });
      }
      return entry;
    };
    // Shows `at` at the middle of the drawing, three times the fitted scale, for a snap to be caught alone.
    const zoomTo = async (page, at) => {
      await page.click('[aria-label="Fit the sheet"]');
      await sleep(600);
      await page.evaluate(([x, y]) => {
        const p = window.agentdrafterViewer;
        const r = document.querySelector('.stage').getBoundingClientRect();
        const scale = p.view().scale * 3;
        p.show({ left: x - r.width / 2 / scale, top: y + r.height / 3 / scale, scale });
      }, at);
      await frames(page);
    };
    const fitAll = async (page) => {
      await page.click('[aria-label="Fit the sheet"]');
      await sleep(600);
    };
    const typeIn = async (page, ...lines) => {
      for (const line of lines) {
        await page.keyboard.type(line);
        await page.keyboard.press('Enter');
        await frames(page);
      }
    };
    try {
      // A laptop: the mouse, and typing.
      {
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        // The current layer: new objects go on it.
        await page.waitForFunction(() => document.querySelectorAll('select[data-layer] option').length > 2);
        await page.selectOption('select[data-layer]', 'wire');
        // Line, by its alias and typed coordinates; Ctrl+Z takes back a placed point (#245).
        let before = journal().length;
        let built = dwg('DP-LD-101');
        await typeIn(page, 'L', '1,1', '@2,0', '@0,5');
        assert((await bar(page)).includes('LINEon wire Click the next point'), `Line prompts on the current layer: ${await bar(page)}`);
        const lineNote = await page.textContent('.cmdnote');
        assert(lineNote.includes('Ctrl+Z: one back'), `Line note names Ctrl+Z: ${lineNote}`);
        await page.keyboard.press('Control+z');
        await frames(page);
        await typeIn(page, '@1<90', '');
        let entry = await landed(page, before, 'Line, typed');
        const segments = entry.change_set.operations.map((o) => `${o.op} ${o.sheet} ${o.layer} ${JSON.stringify(o.from)} ${JSON.stringify(o.to)}`);
        assert(
          JSON.stringify(segments) === JSON.stringify(['add_line DP-LD-101 wire [1,1,0] [3,1,0]', 'add_line DP-LD-101 wire [3,1,0] [3,2,0]']),
          `Line sends one add_line a segment on the current layer, where typed (Ctrl+Z took @0,5 back): ${JSON.stringify(segments)}`,
        );
        assert(dwg('DP-LD-101') !== built, 'the DWG is built again');
        // Outside a tool, Ctrl+Z does nothing: Undo last stays a button (#245).
        const afterLine = journal().length;
        await page.keyboard.press('Control+z');
        await frames(page);
        assert(journal().length === afterLine, 'Ctrl+Z outside a tool does not run Undo last');
        seen.laptop.line_typed = entry.entry;
        // Line by mouse: from FT-101's endpoint, a snap, then level with it, by Ortho.
        const end = snapOf('DP-LD-101/device', 'endpoint', 'objects[3].from');
        await zoomTo(page, end);
        before = journal().length;
        await page.click('.rb[data-tool="line"]');
        const [ex, ey] = await onPage(page, end);
        await page.mouse.move(ex + 4, ey + 3);
        await frames(page);
        await page.mouse.click(ex + 4, ey + 3);
        await page.keyboard.press('F8');
        await page.mouse.move(ex + 150, ey + 9);
        await frames(page);
        const dyn = await page.textContent('[data-dyn]');
        assert(/in/.test(dyn) && /°/.test(dyn), `the length and angle show at the cursor: ${dyn}`);
        await page.screenshot({ path: join(out, 'tools-laptop-line.png') });
        await page.mouse.click(ex + 150, ey + 9);
        await page.mouse.click(ex + 150, ey + 9, { button: 'right' });
        entry = await landed(page, before, 'Line, by mouse');
        const [op] = entry.change_set.operations;
        assert(
          JSON.stringify(op.from) === JSON.stringify({ snap: { id: 'DP-LD-101/device', snap: 'endpoint', which: 'objects[3].from' } }),
          `the first point goes as a snapped point: ${JSON.stringify(op.from)}`,
        );
        assert(Array.isArray(op.to) && op.to[1] === end[1], `Ortho holds the line level: ${JSON.stringify(op.to)} from ${end}`);
        await page.keyboard.press('F8');
        seen.laptop.line_mouse = entry.entry;
        // Fit again, for the clicks to come, in the empty part of the
        // control room under the cabinet.
        await fitAll(page);
        // Circle, by its alias, a typed centre (taken back once with Ctrl+Z) and a typed radius.
        before = journal().length;
        await typeIn(page, 'c', '10,10');
        await page.keyboard.press('Control+z');
        await frames(page);
        assert((await bar(page)).includes('Click the centre'), `Ctrl+Z in Circle takes the centre back: ${await bar(page)}`);
        await typeIn(page, '13,3.6', '0.25');
        entry = await landed(page, before, 'Circle');
        assert(
          entry.change_set.operations[0].op === 'add_circle' && entry.change_set.operations[0].radius === 0.25 && JSON.stringify(entry.change_set.operations[0].center) === '[13,3.6,0]',
          `Circle sends add_circle: ${JSON.stringify(entry.change_set.operations)}`,
        );
        const circleId = entry.objects.find((o) => o.how === 'added').id;
        seen.laptop.circle = entry.entry;
        // Rectangle: a corner clicked, the other typed as its size.
        before = journal().length;
        await page.click('.rb[data-tool="rectangle"]');
        // Snapping off, so the click is the point itself.
        await page.keyboard.press('F3');
        const corner = await onPage(page, [14, 3]);
        await page.mouse.click(corner[0], corner[1]);
        await typeIn(page, '@1,0.5');
        await page.keyboard.press('F3');
        entry = await landed(page, before, 'Rectangle');
        const rect = entry.change_set.operations[0];
        assert(rect.op === 'add_polyline' && rect.closed === true && rect.points.length === 4 && rect.layer === 'wire', `Rectangle sends a closed add_polyline: ${JSON.stringify(rect)}`);
        seen.laptop.rectangle = entry.entry;
        // Text, typed on the drawing: two lines.
        before = journal().length;
        await typeIn(page, 'DT');
        await page.keyboard.press('F3');
        const place = await onPage(page, [10.2, 4.3]);
        await page.mouse.click(place[0], place[1]);
        await page.keyboard.press('F3');
        await page.keyboard.type('PUMP P-1');
        await frames(page);
        const shown = await page.textContent('[data-typed]');
        assert(shown.includes('PUMP P-1'), `the text shows on the drawing as it is typed: ${shown}`);
        await page.screenshot({ path: join(out, 'tools-laptop-text.png') });
        await page.keyboard.press('Enter');
        await typeIn(page, 'SUCTION', '');
        entry = await landed(page, before, 'Text');
        assert(
          entry.change_set.operations.map((o) => `${o.op} ${o.contents}`).join() === 'add_text PUMP P-1,add_text SUCTION',
          `Text sends one add_text a line: ${JSON.stringify(entry.change_set.operations)}`,
        );
        seen.laptop.text = entry.entry;
        // Move: the row's device selected first, then Move, from its endpoint.
        before = journal().length;
        built = dwg('DP-LD-101');
        await page.keyboard.press('F3');
        const device = await objectOf(page, 'DP-LD-101/device');
        // Near its top, clear of the line drawn from its endpoint, which runs across its middle.
        await page.mouse.click(...(await onPage(page, [(device.bounds[0] + device.bounds[2]) / 2, device.bounds[3] - 0.02])));
        await page.keyboard.press('F3');
        assert((await bar(page)).includes('FT-101 selected'), `the device is selected: ${await bar(page)}`);
        await page.waitForSelector('.qp');
        await page.click('.opt[data-opt="move"]');
        assert((await bar(page)).includes('MOVE'), `Move takes what is selected: ${await bar(page)}`);
        await zoomTo(page, end);
        const [bx, by] = await onPage(page, end);
        await page.mouse.move(bx + 3, by + 2);
        await frames(page);
        await page.mouse.click(bx + 3, by + 2);
        await page.mouse.move(bx + 120, by + 40);
        await frames(page);
        await page.screenshot({ path: join(out, 'tools-laptop-move.png') });
        await typeIn(page, '@0.25,0');
        entry = await landed(page, before, 'Move');
        const moved = entry.change_set.operations[0];
        assert(
          moved.op === 'move' && moved.target.id === 'DP-LD-101/device' && moved.from?.snap?.snap === 'endpoint' && Array.isArray(moved.to),
          `Move sends move from the snapped base point: ${JSON.stringify(moved)}`,
        );
        await fitAll(page);
        assert(JSON.stringify(entry.landed_on) === '["DP-LD-101"]', 'the row\'s own object reaches its sheet alone');
        assert(dwg('DP-LD-101') !== built, 'the DWG is built again');
        seen.laptop.move = entry.entry;
        // Erase: the circle drawn, selected, then Delete.
        before = journal().length;
        const circle = await objectOf(page, circleId);
        assert(circle.place >= 0, 'the circle drawn is on the sheet');
        await page.keyboard.press('F3');
        await page.mouse.click(...(await onPage(page, [circle.bounds[2], (circle.bounds[1] + circle.bounds[3]) / 2])));
        await page.keyboard.press('F3');
        await page.waitForSelector('.qp');
        await page.keyboard.press('Delete');
        entry = await landed(page, before, 'Erase');
        assert(JSON.stringify(entry.change_set.operations) === JSON.stringify([{ op: 'delete', target: { id: circleId } }]), `Erase sends delete: ${JSON.stringify(entry.change_set.operations)}`);
        await page.waitForFunction((gone) => !window.agentdrafterViewer.summary.objects.some((o) => o.id === gone), circleId);
        seen.laptop.erase = entry.entry;
        // A template's object: the junction box, by the tool first, then the pick.
        const corner0 = snapOf('DP-LD-101/junction-box', 'endpoint', 'points[0]');
        const tryMove = async () => {
          await fitAll(page);
          // Its left side, half way up, between the wires that cross it.
          const edgeAt = await onPage(page, [6.38, 6.5]);
          await typeIn(page, 'M');
          await page.mouse.click(edgeAt[0], edgeAt[1]);
          await frames(page);
          const warned = await page.textContent('[data-warning]');
          assert(warned.includes('This is part of the template.') && warned.includes('Moving it changes 7 sheets.'), `the command bar warns: ${warned}`);
          await typeIn(page, '');
          await zoomTo(page, corner0);
          const c = await onPage(page, corner0);
          await page.mouse.move(c[0] + 2, c[1] + 2);
          await frames(page);
          await page.mouse.click(c[0] + 2, c[1] + 2);
          await typeIn(page, '@0.1,0');
          // Drawn before it lands, the band above the command bar.
          await page.waitForSelector('[data-preview] [data-pv-summary]');
          const asked = await page.textContent('[data-preview]');
          assert(/Move \S+/.test(asked) && asked.includes('on 7 sheets') && asked.includes('Change all 7 sheets'), `the band: ${asked}`);
          const tabs = await tabMarks(page);
          for (const s of ['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302']) {
            // What the check drew on DP-LD-101 before may make it fail a check there: it would still land.
            assert(['✓', '!'].includes(tabs[s]), `${s}'s tab says it would land: ${JSON.stringify(tabs)}`);
          }
          assert(tabs['DP-LD-103'] === 'off', `the sheet it does not reach is greyed: ${JSON.stringify(tabs)}`);
        };
        before = journal().length;
        const templateBefore = template();
        await tryMove();
        await page.screenshot({ path: join(out, 'tools-laptop-template.png') });
        await page.click('[data-pv="cancel"]');
        await sleep(1500);
        assert(journal().length === before && template() === templateBefore, 'Cancel lands nothing');
        const sheetsBuilt = ['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302'].map((s) => [s, dwg(s)]);
        await tryMove();
        await page.click('[data-pv="go"]');
        entry = await landed(page, before, 'Move of a template object');
        assert(entry.landed_on.length === 7 && !entry.landed_on.includes('DP-LD-103'), `it lands on every sheet built from the template: ${entry.landed_on}`);
        assert(sheetsBuilt.every(([s, was]) => dwg(s) !== was), 'every sheet named has its DWG built again');
        assert(template() !== templateBefore, 'the template changes');
        assert(
          JSON.stringify(entry.change_set.operations[0].from) === JSON.stringify({ snap: { id: 'DP-LD-101/junction-box', snap: 'endpoint', which: 'points[0]' } }),
          `a snap on a template's object stays a snapped point: ${JSON.stringify(entry.change_set.operations[0])}`,
        );
        seen.laptop.template_move = { entry: entry.entry, sheets: entry.landed_on.length };
        // Undo last takes the drafter's edit back, on every sheet: once
        // `change` has replied, it names it, and is on.
        await page.waitForFunction(
          (n) => {
            const b = document.querySelector('.titlebar button[data-act="undo"]');
            return b && !b.disabled && b.textContent.includes(`(#${n})`);
          },
          entry.entry,
          { timeout: 30000 },
        );
        await page.click('.titlebar button[data-act="undo"]');
        // The journal's entry is written ahead of the files and marked done after: wait for both.
        const until = Date.now() + 20000;
        let undone = journal().at(-1);
        while (!(undone.kind === 'undo' && finished(undone) && template() === templateBefore)) {
          assert(Date.now() < until, `Undo last undoes the drafter's edit and puts the template back: ${JSON.stringify(undone)}`);
          await sleep(100);
          undone = journal().at(-1);
        }
        assert(JSON.stringify(undone.undid) === JSON.stringify([entry.entry]), `the undo takes the drafter's edit: ${JSON.stringify(undone)}`);
        seen.laptop.undo = journal().at(-1).entry;
        await page.screenshot({ path: join(out, 'tools-laptop-after.png') });
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      // A phone: taps, the options, ⌨ Type and the text box.
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        // Snapping off, so each tap is the point itself.
        await page.tap('.toolsbtn');
        await page.tap('.ribbon [data-toggle="osnap"]');
        // Line: Tools, Line, two taps and Done.
        let before = journal().length;
        await page.tap('.rb[data-tool="line"]', { force: true });
        assert((await bar(page)).includes('Tap the first point'), `the phone's words: ${await bar(page)}`);
        await page.touchscreen.tap(...(await onPage(page, [11.5, 3.0])));
        await sleep(400);
        await page.touchscreen.tap(...(await onPage(page, [13.5, 3.0])));
        await sleep(400);
        await page.screenshot({ path: join(out, 'tools-phone-line.png') });
        await page.tap('.opt[data-opt="tool:done"]');
        let entry = await landed(page, before, 'Line, by touch');
        assert(entry.change_set.operations.length === 1 && entry.change_set.operations[0].op === 'add_line', `one line: ${JSON.stringify(entry.change_set.operations)}`);
        const lineId = entry.objects.find((o) => o.how === 'added').id;
        seen.phone.line = entry.entry;
        // Circle by ⌨ Type: its alias, a tap for the centre and a radius typed.
        before = journal().length;
        await page.tap('[data-type]');
        await page.keyboard.type('C');
        await page.keyboard.press('Enter');
        await sleep(300);
        await page.touchscreen.tap(...(await onPage(page, [12.5, 4.0])));
        await sleep(300);
        // The field stays open; typing goes back into it.
        await page.keyboard.type('0.2');
        await page.keyboard.press('Enter');
        entry = await landed(page, before, 'Circle, by touch and typing');
        assert(entry.change_set.operations[0].radius === 0.2, `the radius typed: ${JSON.stringify(entry.change_set.operations)}`);
        seen.phone.circle = entry.entry;
        // Text in the small box.
        before = journal().length;
        await page.tap('.toolsbtn');
        await page.tap('.rb[data-tool="text"]', { force: true });
        await page.touchscreen.tap(...(await onPage(page, [12.0, 3.5])));
        await page.waitForSelector('[data-tbox-text]');
        await page.fill('[data-tbox-text]', 'VENT');
        await page.screenshot({ path: join(out, 'tools-phone-text.png') });
        await page.tap('[data-tbox-add]');
        entry = await landed(page, before, 'Text, in the box');
        assert(entry.change_set.operations[0].contents === 'VENT', `Text from the box: ${JSON.stringify(entry.change_set.operations)}`);
        seen.phone.text = entry.entry;
        // Rectangle from Tools: two corners tapped.
        before = journal().length;
        await page.tap('.toolsbtn');
        await page.tap('.rb[data-tool="rectangle"]', { force: true });
        await page.touchscreen.tap(...(await onPage(page, [14, 3.2])));
        await sleep(400);
        await page.touchscreen.tap(...(await onPage(page, [15.2, 4.0])));
        entry = await landed(page, before, 'Rectangle, by touch');
        const rectangle = entry.change_set.operations[0];
        assert(rectangle.op === 'add_polyline' && rectangle.closed === true && rectangle.points.length === 4, `Rectangle by touch: ${JSON.stringify(rectangle)}`);
        const rectangleId = entry.objects.find((o) => o.how === 'added').id;
        seen.phone.rectangle = entry.entry;
        // Move from Tools: the rectangle tapped, Done, a base point and where it goes.
        before = journal().length;
        await page.tap('.toolsbtn');
        await page.tap('.rb[data-tool="move"]', { force: true });
        await page.touchscreen.tap(...(await onPage(page, [15.2, 3.6])));
        await sleep(300);
        assert((await bar(page)).includes('1 object picked'), `the rectangle is picked: ${await bar(page)}`);
        await page.tap('.opt[data-opt="tool:done"]');
        await page.touchscreen.tap(...(await onPage(page, [15.2, 3.2])));
        await sleep(400);
        await page.touchscreen.tap(...(await onPage(page, [15.2, 4.4])));
        entry = await landed(page, before, 'Move, by touch');
        const moved = entry.change_set.operations[0];
        assert(moved.op === 'move' && moved.target.id === rectangleId && Array.isArray(moved.from) && Array.isArray(moved.to), `Move by touch: ${JSON.stringify(moved)}`);
        assert(moved.to[1] - moved.from[1] > 1, `it moves up by the taps: ${JSON.stringify(moved)}`);
        seen.phone.move = entry.entry;
        // Erase from Tools: the line drawn, tapped, then Erase 1.
        before = journal().length;
        await page.tap('.toolsbtn');
        await page.tap('.rb[data-tool="erase"]', { force: true });
        await page.touchscreen.tap(...(await onPage(page, [12.5, 3.0])));
        await sleep(300);
        await page.screenshot({ path: join(out, 'tools-phone-erase.png') });
        assert((await bar(page)).includes('1 object picked'), `the line is picked: ${await bar(page)}`);
        await page.tap('.opt[data-opt="tool:done"]');
        entry = await landed(page, before, 'Erase, by touch');
        assert(JSON.stringify(entry.change_set.operations) === JSON.stringify([{ op: 'delete', target: { id: lineId } }]), `Erase by touch: ${JSON.stringify(entry.change_set.operations)}`);
        seen.phone.erase = entry.entry;
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      return { ...seen, screenshots: ['tools-laptop-line.png', 'tools-laptop-text.png', 'tools-laptop-move.png', 'tools-laptop-template.png', 'tools-laptop-after.png', 'tools-phone-line.png', 'tools-phone-text.png'] };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * The Copy tool (#66), against a copy of the demo plant, at laptop and
   * phone sizes: FT-101 selected, then Copy from its options; a base point,
   * then copies placed by a typed displacement, a length along the cursor
   * and a click, dashed and tagged while they wait; the last taken back by
   * Ctrl+Z, U and Undo; Enter landing them all as one change set, one
   * `copy` with `offsets`, one journal entry; Esc landing the copy placed,
   * and with none placed ending the tool; and the junction box, a template's
   * object, warned of while Copy runs and asked about once, when the copies
   * land, in the preview's band, Change all landing on every sheet.
   */
  async copy() {
    const folder = freshPlant('copy');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    const seen = { laptop: {}, phone: {} };
    const shots = [];
    const journal = () => agentIn(folder, 'journal', 'demo-plant').result.entries;
    const template = () => readFileSync(join(folder, 'demo-plant', 'templates', 'loop-2-wire.yaml'), 'utf8');
    const bar = (page) => page.evaluate(() => document.querySelector('.cmdbar')?.textContent ?? '');
    const frames = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    const onPage = (page, at) =>
      page.evaluate(([x, y]) => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
      }, at);
    const tags = (page) => page.evaluate(() => [...document.querySelectorAll('[data-copytag]')].map((t) => t.textContent));
    const shot = async (page, name) => {
      await frames(page);
      await page.screenshot({ path: join(out, name) });
      shots.push(name);
    };
    // Waits for the journal to hold one more entry than `before`, a change
    // set that landed on `sheet`, and (when `waitDrawn` is true) for the page
    // to draw what it added.
    const landed = async (page, before, what, sheet = 'DP-LD-101', waitDrawn = true) => {
      const until = Date.now() + 30000;
      let entries = journal();
      while (!grownAndDone(entries, before)) {
        assert(Date.now() < until, `${what}: no change set landed: ${await bar(page)}`);
        await sleep(100);
        entries = journal();
      }
      assert(entries.length === before + 1, `${what}: one journal entry, not ${entries.length - before}`);
      const entry = entries.at(-1);
      assert(entry.kind === 'change_set' && entry.landed_on?.includes(sheet), `${what} landed: ${JSON.stringify(entry)}`);
      if (waitDrawn) {
        const added = (entry.objects ?? []).filter((o) => o.how === 'added' && o.id.startsWith(`${sheet}/`)).map((o) => o.id);
        for (const id of added) {
          await page.waitForFunction((wanted) => window.agentdrafterViewer.summary.objects.some((o) => o.id === wanted), id, { timeout: 15000 });
        }
      }
      return entry;
    };
    const typeIn = async (page, ...lines) => {
      for (const line of lines) {
        await page.keyboard.type(line);
        await page.keyboard.press('Enter');
        await frames(page);
      }
    };
    const fitAll = async (page) => {
      await page.click('[aria-label="Fit the sheet"]');
      await sleep(600);
    };
    // The journal's line for entry `n`, in the side panel.
    const journalLine = async (page, n, how) => {
      await sideTab(page, 'journal', how);
      await page.waitForSelector(`[data-entry="${n}"] .je-t`, { timeout: 15000 });
      return page.textContent(`[data-entry="${n}"] .je-t`);
    };
    // How many pixels in the drawing-unit box `[left, bottom, right, top]`
    // are the proposal's purple, as the renderer draws the copies placed,
    // the tags over them hidden: a copy placed is drawn, not only tagged.
    const purpleIn = async (page, [l, b, r, t]) => {
      const [x0, y0] = await onPage(page, [l, t]);
      const [x1, y1] = await onPage(page, [r, b]);
      await page.evaluate(() => (document.querySelector('[data-copytags]').style.visibility = 'hidden'));
      await frames(page);
      const png = decodePng(await page.screenshot({ clip: { x: Math.floor(x0) - 2, y: Math.floor(y0) - 2, width: Math.ceil(x1 - x0) + 4, height: Math.ceil(y1 - y0) + 4 } }));
      await page.evaluate(() => (document.querySelector('[data-copytags]').style.visibility = ''));
      let n = 0;
      for (let i = 0; i < png.rgba.length; i += 4) {
        const [red, green, blue] = [png.rgba[i], png.rgba[i + 1], png.rgba[i + 2]];
        if (Math.abs(red - 0xc9) + Math.abs(green - 0xa8) + Math.abs(blue - 0xff) < 60) n += 1;
      }
      return n;
    };
    // Waits until the canvas shows the set as it is, live: no band, no
    // preview's drawing (on which nothing is picked) and no entry; then
    // shows the whole sheet, and gives a point on the page that a click
    // picks the object `id` at, as the renderer picks (16 CSS pixels).
    const pointOn = async (page, id) => {
      await page.waitForFunction(
        () => {
          const p = window.agentdrafterViewer;
          return !document.querySelector('[data-preview]') && p.summary && p.summary.preview == null && p.summary.entry === null;
        },
        null,
        { timeout: 30000 },
      );
      await fitAll(page);
      const { place, box } = await page.evaluate((wanted) => {
        const p = window.agentdrafterViewer;
        const at = p.summary.objects.findIndex((o) => o.id === wanted);
        return { place: at, box: at >= 0 ? p.bounds([at]) : [] };
      }, id);
      assert(place >= 0 && box.length === 4, `${id} is on the sheet`);
      const [x0, y0, x1, y1] = box;
      for (let j = 1; j < 8; j++) {
        for (let i = 1; i < 12; i++) {
          const at = [x0 + ((x1 - x0) * i) / 12, y0 + ((y1 - y0) * j) / 8];
          if ((await page.evaluate(([x, y]) => window.agentdrafterViewer.pickAt(x, y, 16), at)) === place) return { place, at: await onPage(page, at) };
        }
      }
      throw new Error(`no point on the page picks ${id}`);
    };
    const copyOp = (entry) => {
      const ops = entry.change_set.operations;
      assert(ops.length === 1 && ops[0].op === 'copy', `one copy operation: ${JSON.stringify(ops)}`);
      return ops[0];
    };
    try {
      // A laptop: the mouse, and typing.
      {
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        // The ribbon: Copy in Modify, between Move and Array, CO and CP in its tip.
        const modify = await page.evaluate(() => [...document.querySelectorAll('.ribbon .rb[data-tool]')].map((x) => x.dataset.tool));
        assert(JSON.stringify(modify.slice(-4)) === '["move","copy","array","erase"]', `Copy sits between Move and Array: ${modify}`);
        await page.hover('.rb[data-tool="copy"]');
        const tip = await page.textContent('.tip');
        assert(tip.includes('CO') && tip.includes('CP') && tip.includes('keeps placing copies'), `Copy's tip: ${tip}`);
        await page.mouse.move(700, 500);
        // FT-101 selected first, then Copy from its options.
        await fitAll(page);
        await page.keyboard.press('F3');
        const device = await objectOf(page, 'DP-LD-101/device');
        await page.mouse.click(...(await onPage(page, [(device.bounds[0] + device.bounds[2]) / 2, device.bounds[3] - 0.02])));
        assert((await bar(page)).includes('FT-101 selected'), `the device is selected: ${await bar(page)}`);
        assert(/Move.*Copy.*Erase.*Clear/.test(await bar(page)), `a selected object's options hold Copy: ${await bar(page)}`);
        await page.click('.opt[data-opt="copy-objects"]');
        assert((await bar(page)).includes('COPY') && (await bar(page)).includes('the base point'), `Copy takes what is selected: ${await bar(page)}`);
        let before = journal().length;
        // The base point typed; a displacement typed; a length towards the
        // cursor, below the base; a click; then Ctrl+Z takes it back.
        const above = [device.bounds[0], device.bounds[1] + 1.2, device.bounds[2], device.bounds[3] + 1.2];
        const below = [device.bounds[0], device.bounds[1] - 1.2, device.bounds[2], device.bounds[3] - 1.2];
        await typeIn(page, '2,6.5');
        const bare = [await purpleIn(page, above), await purpleIn(page, below)];
        assert(bare[0] < 5 && bare[1] < 5, `nothing purple where the copies will go: ${bare}`);
        await typeIn(page, '@0,1.2');
        assert((await bar(page)).includes('1 copy placed'), `a copy is placed: ${await bar(page)}`);
        // Held upright by Ortho (F8), as the cursor is only nearly below.
        await page.keyboard.press('F8');
        await page.mouse.move(...(await onPage(page, [2.05, 3])));
        await frames(page);
        await typeIn(page, '1.2');
        await page.keyboard.press('F8');
        await page.mouse.click(...(await onPage(page, [4.5, 6.5])));
        await frames(page);
        assert((await bar(page)).includes('3 copies placed'), `three copies placed: ${await bar(page)}`);
        await page.keyboard.press('Control+z');
        await frames(page);
        assert((await bar(page)).includes('2 copies placed'), `Ctrl+Z takes the last back: ${await bar(page)}`);
        // U, then the Undo option, each take one back; placed again by typing.
        await typeIn(page, '@3,0', 'U');
        assert((await bar(page)).includes('2 copies placed'), `U takes the last back: ${await bar(page)}`);
        await typeIn(page, '@3,0');
        // Undo reached by Tab, then clicked: it gives the focus up, so the
        // Enter below lands the copies rather than pressing Undo again.
        await page.focus('.opt[data-opt="tool:undo"]');
        await page.click('.opt[data-opt="tool:undo"]');
        assert(!(await page.evaluate(() => document.activeElement?.closest?.('.opt'))), 'Undo clicked keeps no focus');
        assert((await bar(page)).includes('2 copies placed'), `Undo takes the last back: ${await bar(page)}`);
        assert(journal().length === before, 'nothing lands while the copies are placed');
        // The copy that follows the cursor, and the two placed, tagged.
        await page.mouse.move(...(await onPage(page, [3.2, 4.4])));
        await frames(page);
        assert(JSON.stringify(await tags(page)) === JSON.stringify(['copy 1 · waits for Enter', 'copy 2 · waits for Enter']), `the copies placed are tagged: ${await tags(page)}`);
        const note = await page.textContent('.cmdnote');
        assert(note.includes('Enter or Esc: the copies land') && note.includes('Ctrl+Z'), `the note: ${note}`);
        // Each copy placed is drawn, dashed in the proposal's purple, where it goes.
        const drawn = [await purpleIn(page, above), await purpleIn(page, below)];
        assert(drawn[0] > 40 && drawn[1] > 40, `the copies placed are drawn in purple: ${drawn}`);
        await shot(page, 'copy-laptop-1-mid-copy.png');
        // Enter lands them all: one change set, one copy with offsets.
        await page.keyboard.press('Enter');
        let entry = await landed(page, before, 'Copy, Enter');
        let op = copyOp(entry);
        assert(op.target?.id === 'DP-LD-101/device' && JSON.stringify(op.offsets) === '[[0,1.2,0],[0,-1.2,0]]', `one copy with offsets: ${JSON.stringify(op)}`);
        const added = entry.objects.filter((o) => o.how === 'added').map((o) => o.id);
        assert(added.length === 2, `two copies added: ${JSON.stringify(entry.objects)}`);
        assert(JSON.stringify(entry.landed_on) === '["DP-LD-101"]', `the row's own object's copies land on its sheet alone: ${entry.landed_on}`);
        assert((await tags(page)).length === 0, 'the tags go when the copies land');
        seen.laptop.enter = entry.entry;
        await page.mouse.move(700, 600);
        await shot(page, 'copy-laptop-2-after-enter.png');
        const line = await journalLine(page, entry.entry);
        assert(line.includes('copy device, 2 copies'), `one journal line: ${line}`);
        await shot(page, 'copy-laptop-3-journal.png');
        await sideTab(page, 'chat');
        // Esc lands the copy placed: by CP, the device picked, one copy.
        before = journal().length;
        await page.keyboard.press('Escape');
        await typeIn(page, 'CP');
        await page.mouse.click(...(await onPage(page, [(device.bounds[0] + device.bounds[2]) / 2, device.bounds[3] - 0.02])));
        await typeIn(page, '', '2,6.5', '@1.5,-2.5');
        await page.keyboard.press('Escape');
        entry = await landed(page, before, 'Copy, Esc');
        op = copyOp(entry);
        assert(JSON.stringify(op.from) === '[2,6.5,0]' && JSON.stringify(op.to) === '[3.5,4,0]', `one copy goes from and to: ${JSON.stringify(op)}`);
        seen.laptop.escape = entry.entry;
        // With none placed, Esc just ends the tool.
        before = journal().length;
        await typeIn(page, 'CO');
        await page.mouse.click(...(await onPage(page, [(device.bounds[0] + device.bounds[2]) / 2, device.bounds[3] - 0.02])));
        await typeIn(page, '', '2,6.5');
        await page.keyboard.press('Escape');
        await sleep(1000);
        assert(journal().length === before, 'Esc with no copy placed lands nothing');
        assert(!(await bar(page)).includes('COPY'), `Esc ends the tool: ${await bar(page)}`);
        const history = () => page.evaluate(() => document.querySelector('.cmd .hist')?.textContent ?? '');
        // A copy placed, then a journal entry opened (#246): the copy lands first, then the entry shows.
        before = journal().length;
        let target = await pointOn(page, 'DP-LD-101/device');
        await typeIn(page, 'CO');
        await page.mouse.click(...target.at);
        assert((await bar(page)).includes('1 object picked'), `Copy picks the device: ${await bar(page)}`);
        await typeIn(page, '', '2,6.5', '@0,-3');
        assert((await bar(page)).includes('1 copy placed'), `a copy is placed: ${await bar(page)}`);
        await sideTab(page, 'journal');
        await page.click(`[data-entry="${seen.laptop.enter}"] .je-head`);
        entry = await landed(page, before, 'Copy, journal entry opened', 'DP-LD-101', false);
        op = copyOp(entry);
        assert(JSON.stringify(op.from) === '[2,6.5,0]' && JSON.stringify(op.to) === '[2,3.5,0]', `journal entry open lands the placed copy: ${JSON.stringify(op)}`);
        await page.waitForFunction(() => document.querySelector('.cmdbar')?.textContent.includes('Showing entry'), null, { timeout: 15000 });
        assert(!(await bar(page)).includes('COPY'), `the copy landed and the tool ended: ${await bar(page)}`);
        await page.click('.opt[data-opt="live"]');
        await page.waitForFunction(() => !document.querySelector('.cmdbar')?.textContent.includes('Showing entry'), null, { timeout: 15000 });
        for (const id of (entry.objects ?? []).filter((o) => o.how === 'added' && o.id.startsWith('DP-LD-101/')).map((o) => o.id)) {
          await page.waitForFunction((wanted) => window.agentdrafterViewer.summary.objects.some((o) => o.id === wanted), id, { timeout: 15000 });
        }
        await sideTab(page, 'chat');
        // A copy placed, then another sheet clicked (#246): the copy lands on DP-LD-101, then DP-LD-102 opens.
        before = journal().length;
        target = await pointOn(page, 'DP-LD-101/device');
        await typeIn(page, 'CO');
        await page.mouse.click(...target.at);
        assert((await bar(page)).includes('1 object picked'), `Copy picks the device before sheet switch: ${await bar(page)}`);
        await typeIn(page, '', '2,6.5', '@0,-2.5');
        assert((await bar(page)).includes('1 copy placed'), `a copy is placed before sheet switch: ${await bar(page)}`);
        await page.click('.tab:has-text("DP-LD-102")');
        entry = await landed(page, before, 'Copy, sheet switched', 'DP-LD-101', false);
        op = copyOp(entry);
        assert(JSON.stringify(op.from) === '[2,6.5,0]' && JSON.stringify(op.to) === '[2,4,0]', `sheet switch lands the placed copy: ${JSON.stringify(op)}`);
        await page.waitForFunction(() => window.agentdrafterViewer.summary?.sheet === 'DP-LD-102', null, { timeout: 15000 });
        assert(!(await bar(page)).includes('COPY'), `sheet switch ends the tool: ${await bar(page)}`);
        await page.click('.tab:has-text("DP-LD-101")');
        await page.waitForFunction(() => window.agentdrafterViewer.summary?.sheet === 'DP-LD-101', null, { timeout: 15000 });
        for (const id of (entry.objects ?? []).filter((o) => o.how === 'added' && o.id.startsWith('DP-LD-101/')).map((o) => o.id)) {
          await page.waitForFunction((wanted) => window.agentdrafterViewer.summary.objects.some((o) => o.id === wanted), id, { timeout: 15000 });
        }
        // A template object's copy placed, then another sheet clicked (#246):
        // stays on DP-LD-101 and shows the Change all band first.
        before = journal().length;
        target = await pointOn(page, 'DP-LD-101/junction-box');
        await typeIn(page, 'CO');
        await page.mouse.click(...target.at);
        assert((await bar(page)).includes('1 object picked'), `Copy picks the junction box before sheet switch: ${await bar(page)}`);
        await typeIn(page, '', '6.38,6.5', '@0,-2.5');
        assert((await bar(page)).includes('1 copy placed'), `a template copy is placed before sheet switch: ${await bar(page)}`);
        await page.click('.tab:has-text("DP-LD-102")');
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        const askedOnSwitch = await page.textContent('[data-preview]');
        assert(askedOnSwitch.includes('Copy') && askedOnSwitch.includes('Change all 7 sheets'), `sheet switch asks Change all first: ${askedOnSwitch}`);
        assert(await page.evaluate(() => window.agentdrafterViewer.summary?.sheet) === 'DP-LD-101', 'stays on DP-LD-101 while Change all asks');
        assert(!(await bar(page)).includes('COPY'), `the tool ended into preview: ${await bar(page)}`);
        await page.click('[data-pv="cancel"]');
        await page.waitForFunction(() => !document.querySelector('[data-preview]'), null, { timeout: 30000 });
        assert(journal().length === before, 'Cancel writes nothing after sheet-switch preview');
        // A proposal waiting, then a template's copy placed and the proposal
        // opened (#246): stays and shows the copy's Change all band first.
        const proposalFile = join(folder, 'proposal.json');
        writeFileSync(proposalFile, JSON.stringify(move('DP-LD-101/device', 0.1)));
        const proposed = agentIn(folder, 'propose', 'demo-plant', proposalFile);
        assert(proposed.ok, `a proposal waits: ${JSON.stringify(proposed)}`);
        const number = proposed.result.proposal;
        await page.waitForSelector(`[data-pcard="${number}"]`, { timeout: 30000 });
        // The whole sheet again.
        before = journal().length;
        target = await pointOn(page, 'DP-LD-101/junction-box');
        await typeIn(page, 'CO');
        await page.mouse.click(...target.at);
        assert((await bar(page)).includes('1 object picked'), `Copy picks the junction box: ${await bar(page)}`);
        await typeIn(page, '', '6.38,6.5', '@0,-2.2');
        assert((await bar(page)).includes('1 copy placed'), `a template's copy is placed: ${await bar(page)}`);
        await page.click(`[data-pcard="${number}"] [data-pc="show"]`);
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        const askedOnProposal = await page.textContent('[data-preview]');
        assert(askedOnProposal.includes('Copy') && askedOnProposal.includes('Change all 7 sheets') && !/Discard/.test(askedOnProposal), `the copy's Change all band is shown first: ${askedOnProposal}`);
        assert(!(await bar(page)).includes('COPY'), `the tool ended into preview: ${await bar(page)}`);
        await page.click('[data-pv="cancel"]');
        await page.waitForFunction(() => !document.querySelector('[data-preview]'), null, { timeout: 30000 });
        assert(journal().length === before, 'cancelling the template copy writes nothing');
        // Now open and discard the waiting proposal.
        await page.click(`[data-pcard="${number}"] [data-pc="show"]`);
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        const opened = await page.textContent('[data-preview]');
        assert(!opened.includes('Copy') && /Discard/.test(opened), `the proposal is shown once Copy is closed: ${opened}`);
        await page.click('[data-pv="discard"]');
        await page.waitForFunction(() => !document.querySelector('[data-preview]'), null, { timeout: 30000 });
        assert(journal().slice(before).every((e) => e.kind === 'discard'), `only the discard is written: ${JSON.stringify(journal().slice(before))}`);
        // A quick properties field edited, then Clear: the edit lands, as leaving the field lands it.
        // Nothing selected, the proposal's card gone and the set drawn live first.
        await page.keyboard.press('Escape');
        await page.waitForFunction((n) => !document.querySelector(`[data-pcard="${n}"]`), number, { timeout: 30000 });
        before = journal().length;
        target = await pointOn(page, 'DP-LD-101/device');
        await page.mouse.click(...target.at);
        await page.waitForFunction((p) => window.agentdrafterViewer.marked?.picked === p, target.place, { timeout: 15000 });
        await page.waitForSelector('.qp');
        assert((await bar(page)).includes('FT-101 selected'), `the device alone is selected: ${await bar(page)}`);
        const funcKey = await page.evaluate(() => {
          const p = window.agentdrafterViewer;
          const place = p.summary.objects.findIndex((o) => o.id === 'DP-LD-101/device');
          return `attribute:${p.summary.objects[place].values.attributes.findIndex((a) => a.tag === 'FUNC')}`;
        });
        await page.click(`.qp [data-prop="${funcKey}"]`);
        await page.fill(`.qp [data-prop="${funcKey}"]`, 'FQ');
        await page.click('.opt[data-opt="clear"]');
        entry = await landed(page, before, 'A field edited, then Clear');
        assert(entry.change_set.operations[0].op === 'set_attribute' && entry.change_set.operations[0].value === 'FQ', `the edit lands: ${JSON.stringify(entry.change_set.operations)}`);
        assert(!(await page.isVisible('.qp')), 'Clear clears the selection');
        // OFFSET is no drafting-app tool yet: Enter sends it to the assistant, as any other word.
        await page.keyboard.type('OFFSET');
        const hint = await page.textContent('.cmdbar');
        assert(hint.includes('ask the assistant'), `OFFSET is for the assistant: ${hint}`);
        await page.keyboard.press('Enter');
        await page.waitForFunction(() => document.querySelector('.cmd .hist')?.textContent.includes('To the assistant: “OFFSET”'), null, { timeout: 15000 });
        assert(await page.isVisible('.stab[data-side="chat"].on'), 'the chat is shown');
        assert(!(await bar(page)).includes('OFFSET'), `no tool starts: ${await bar(page)}`);
        // Not sent, with no agent here, the words go back in the bar: cleared.
        await sleep(1000);
        await page.fill('.cmdin', '');
        await page.keyboard.press('Escape');
        // The junction box, a template's object: warned while Copy runs,
        // asked about once, when the copies land.
        await page.keyboard.press('F3');
        await fitAll(page);
        await page.keyboard.press('F3');
        before = journal().length;
        const templateBefore = template();
        const edgeAt = await onPage(page, [6.38, 6.5]);
        await typeIn(page, 'CO');
        await page.mouse.click(edgeAt[0], edgeAt[1]);
        await typeIn(page, '', '6.38,6.5', '@0,-2.2', '@0,1.9');
        const warned = await page.textContent('[data-warning]');
        assert(
          warned.includes('This is part of the template.') && warned.includes('This copies it on every sheet built from this template: 7 sheets.') && warned.includes('You will be asked before it lands.'),
          `the command bar warns: ${warned}`,
        );
        await page.mouse.move(...(await onPage(page, [9.5, 4.0])));
        await shot(page, 'copy-laptop-4-template-warning.png');
        assert(journal().length === before, 'nothing lands, nor is asked, while copies are placed');
        // Circle picked now: the copies are asked about and Circle does not
        // start, so two clicks on the drawing draw nothing before the answer.
        await page.click('.rb[data-tool="circle"]');
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        assert(!(await bar(page)).includes('CIRCLE'), `Circle does not start: ${await bar(page)}`);
        assert((await history()).includes('CIRCLE not started'), `it says so: ${await history()}`);
        await page.mouse.click(...(await onPage(page, [13, 3.6])));
        await page.mouse.click(...(await onPage(page, [13.3, 3.6])));
        await sleep(1500);
        assert(journal().length === before, `nothing lands before Change all or Cancel: ${JSON.stringify(journal().at(-1))}`);
        await page.click('[data-pv="cancel"]');
        await sleep(1000);
        assert(journal().length === before && template() === templateBefore, 'Cancel lands nothing');
        // Placed again, and Enter.
        await page.keyboard.press('F3');
        await fitAll(page);
        await page.keyboard.press('F3');
        await typeIn(page, 'CO');
        await page.mouse.click(...(await onPage(page, [6.38, 6.5])));
        await typeIn(page, '', '6.38,6.5', '@0,-2.2', '@0,1.9');
        await page.keyboard.press('Enter');
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        const asked = await page.textContent('[data-preview]');
        assert(asked.includes('Copy') && asked.includes('2 copies') && asked.includes('Change all 7 sheets'), `the band: ${asked}`);
        await shot(page, 'copy-laptop-5-change-all.png');
        await page.click('[data-pv="go"]');
        entry = await landed(page, before, 'Copy of a template object');
        op = copyOp(entry);
        assert(op.offsets?.length === 2, `the template's copies go as offsets: ${JSON.stringify(op)}`);
        assert(entry.landed_on.length === 7 && !entry.landed_on.includes('DP-LD-103'), `they land on every sheet built from the template: ${entry.landed_on}`);
        assert(template() !== templateBefore, 'the template changes');
        seen.laptop.template = { entry: entry.entry, sheets: entry.landed_on.length };
        const tline = await journalLine(page, entry.entry);
        assert(tline.includes('copy junction-box, 2 copies'), `its journal line: ${tline}`);
        await shot(page, 'copy-laptop-6-template-journal.png');
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      // A phone: Tools, taps, the options, and the band in the command bar's place.
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        // Snapping off, so each tap is the point itself.
        await page.tap('.toolsbtn');
        await page.tap('.ribbon [data-toggle="osnap"]');
        let before = journal().length;
        await page.tap('.rb[data-tool="copy"]', { force: true });
        assert((await bar(page)).includes('Tap the objects to copy'), `the phone's words: ${await bar(page)}`);
        const device = await objectOf(page, 'DP-LD-101/device');
        await page.touchscreen.tap(...(await onPage(page, [(device.bounds[0] + device.bounds[2]) / 2, device.bounds[3] - 0.02])));
        await sleep(300);
        assert((await bar(page)).includes('1 object picked'), `the device is picked: ${await bar(page)}`);
        await page.tap('.opt[data-opt="tool:done"]');
        await page.touchscreen.tap(...(await onPage(page, [2, 6.5])));
        await sleep(400);
        await page.touchscreen.tap(...(await onPage(page, [4, 4.2])));
        await sleep(400);
        await page.touchscreen.tap(...(await onPage(page, [4, 9.2])));
        await sleep(400);
        await page.touchscreen.tap(...(await onPage(page, [5, 9.2])));
        await sleep(400);
        assert((await bar(page)).includes('3 copies placed'), `three placed: ${await bar(page)}`);
        await page.tap('.opt[data-opt="tool:undo"]');
        await sleep(200);
        assert((await bar(page)).includes('2 copies placed'), `Undo takes one back: ${await bar(page)}`);
        assert(JSON.stringify(await tags(page)) === JSON.stringify(['copy 1', 'copy 2']), `a phone's tags: ${await tags(page)}`);
        await shot(page, 'copy-phone-1-mid-copy.png');
        await page.tap('.opt[data-opt="tool:done"]');
        let entry = await landed(page, before, 'Copy, by touch');
        const op = copyOp(entry);
        assert(op.offsets?.length === 2 && op.target?.id === 'DP-LD-101/device', `one copy with two offsets: ${JSON.stringify(op)}`);
        seen.phone.done = entry.entry;
        await shot(page, 'copy-phone-2-after-done.png');
        // The journal, the sheet dragged up as a finger raises it.
        const cdp = await context.newCDPSession(page);
        for (let attempt = 1; ; attempt += 1) {
          assert(await sheetAtRest(page, false), 'the sheet comes to rest at its peek');
          const [gx, gy] = await page.evaluate(() => {
            const r = document.querySelector('.va-grab').getBoundingClientRect();
            return [r.left + r.width / 2, r.top + r.height / 2];
          });
          await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: gx, y: gy }] });
          for (let i = 1; i <= 10; i += 1) {
            await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: gx, y: gy - i * 30 }] });
            await sleep(16);
          }
          await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
          if (await sheetAtRest(page, true, attempt >= 2 ? 30000 : 5000)) break;
          assert(attempt < 2, 'dragged up twice, the sheet is raised');
        }
        const line = await journalLine(page, entry.entry, 'tap');
        assert(line.includes('copy device, 2 copies'), `one journal line: ${line}`);
        await sleep(500);
        await shot(page, 'copy-phone-3-journal.png');
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        await page.tap('.toolsbtn');
        await page.tap('.ribbon [data-toggle="osnap"]');
        // The junction box: the warning, then the band, then Change all.
        const before = journal().length;
        await page.tap('.rb[data-tool="copy"]', { force: true });
        await page.touchscreen.tap(...(await onPage(page, [6.38, 6.5])));
        await sleep(300);
        assert((await bar(page)).includes('1 object picked'), `the junction box is picked: ${await bar(page)}`);
        await page.tap('.opt[data-opt="tool:done"]');
        await page.touchscreen.tap(...(await onPage(page, [6.38, 6.5])));
        await sleep(400);
        await page.touchscreen.tap(...(await onPage(page, [6.38, 3.6])));
        await sleep(400);
        const warned = await page.textContent('[data-warning]');
        assert(warned.includes('This copies it on every sheet built from this template: 7 sheets.'), `the warning on a phone: ${warned}`);
        await shot(page, 'copy-phone-4-template-warning.png');
        await page.tap('.opt[data-opt="tool:done"]');
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        const asked = await page.textContent('[data-preview]');
        assert(asked.includes('Change all 7 sheets'), `the band on a phone: ${asked}`);
        await shot(page, 'copy-phone-5-change-all.png');
        await page.tap('[data-pv="go"]');
        const entry = await landed(page, before, 'Copy of a template object, by touch');
        assert(entry.landed_on.length === 7, `on every sheet built from the template: ${entry.landed_on}`);
        seen.phone.template = { entry: entry.entry, sheets: entry.landed_on.length };
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      return { ...seen, screenshots: shots };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * The Array tool (#66), against a copy of the demo plant, with a circle
   * and a short tick on it added in DP-LD-101's row for a polar array. At
   * laptop size: Array between Copy and Erase, AR in its tip; FT-101
   * selected, Array from its options, its panel beside it; rows, columns and
   * spacings typed in the panel, the copies drawn dashed in the proposal's
   * purple and tagged; the total toggle; Esc sending nothing; AR typed with
   * rows,columns, TO and @dx,dy in the command bar, a button pressed and
   * Enter landing one change set, one `array`, one journal entry; leaving
   * for an entry dropping it; ARRAYPOLAR on the tick, its centre clicked on
   * the circle's Centre snap, eight round a whole turn, drawn, landing with
   * a snapped centre; and terminal-2, a template's, warned of, its band on
   * Enter, no tool starting while it asks, Change all landing on the seven
   * sheets. At phone size, by touch: Array from Tools, the panel docked
   * above the command bar with the device in sight above it, Cancel sending
   * nothing, the total toggle, Array landing; a polar array's centre typed;
   * and terminal-2's band and Change all.
   */
  async array() {
    const folder = freshPlant('array');
    // A circle and a short tick on its top, in DP-LD-101's row, as the
    // prototype's entry 4 drew them, to array round the circle's centre.
    const seeded = sendChange(folder, {
      operations: [
        { op: 'add_circle', sheet: 'DP-LD-101', layer: 'notes', center: [3, 8.8, 0], radius: 0.3 },
        { op: 'add_line', sheet: 'DP-LD-101', layer: 'notes', from: [3, 9.14, 0], to: [3, 9.32, 0] },
      ],
    });
    assert(seeded.ok, `the circle and the tick are added: ${JSON.stringify(seeded.error ?? seeded)}`);
    const [circleId, tickId] = seeded.result.operations.map((o) => o.ids_written?.[0] ?? /`(DP-LD-101\/[^`]+)`/.exec(o.message)?.[1]);
    assert(circleId && tickId, `the circle's and the tick's IDs: ${JSON.stringify(seeded.result.operations)}`);
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    const seen = { laptop: {}, phone: {} };
    const shots = [];
    const journal = () => agentIn(folder, 'journal', 'demo-plant').result.entries;
    const template = () => readFileSync(join(folder, 'demo-plant', 'templates', 'loop-2-wire.yaml'), 'utf8');
    const bar = (page) => page.evaluate(() => document.querySelector('.cmdbar')?.textContent ?? '');
    const panelText = (page) => page.evaluate(() => document.querySelector('[data-arraypanel]')?.textContent ?? '');
    const count = (page) => page.evaluate(() => document.querySelector('[data-acount]')?.textContent ?? '');
    const tag = (page) => page.evaluate(() => {
      const t = document.querySelector('[data-arraytag]');
      return t && !t.hidden ? t.textContent : null;
    });
    const fieldValue = (page, key) => page.inputValue(`[data-arraypanel] input[data-k="${key}"]`);
    const frames = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    const onPage = (page, at) =>
      page.evaluate(([x, y]) => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
      }, at);
    // A shot of the tool being set up waits for the toast of what landed
    // before to go, as the prototype's shots show it; one of a landing keeps it.
    const shot = async (page, name, toast = false) => {
      if (!toast) await page.waitForFunction(() => !document.querySelector('.va-toast'), null, { timeout: 20000 }).catch(() => {});
      await frames(page);
      await page.screenshot({ path: join(out, name) });
      shots.push(name);
    };
    const landed = async (page, before, what, sheet = 'DP-LD-101') => {
      const until = Date.now() + 30000;
      let entries = journal();
      while (entries.length <= before) {
        assert(Date.now() < until, `${what}: no change set landed: ${await bar(page)}`);
        await sleep(100);
        entries = journal();
      }
      assert(entries.length === before + 1, `${what}: one journal entry, not ${entries.length - before}`);
      const entry = entries.at(-1);
      assert(entry.kind === 'change_set' && entry.landed_on?.includes(sheet), `${what} landed: ${JSON.stringify(entry)}`);
      const added = (entry.objects ?? []).filter((o) => o.how === 'added' && o.id.startsWith(`${sheet}/`)).map((o) => o.id);
      for (const id of added) {
        await page.waitForFunction((wanted) => window.agentdrafterViewer.summary.objects.some((o) => o.id === wanted), id, { timeout: 15000 });
      }
      return entry;
    };
    const typeIn = async (page, ...lines) => {
      for (const line of lines) {
        await page.keyboard.type(line);
        await page.keyboard.press('Enter');
        await frames(page);
      }
    };
    const fitAll = async (page) => {
      await page.click('[aria-label="Fit the sheet"]');
      await sleep(600);
    };
    const journalLine = async (page, n, how) => {
      await sideTab(page, 'journal', how);
      await page.waitForSelector(`[data-entry="${n}"] .je-t`, { timeout: 15000 });
      return page.textContent(`[data-entry="${n}"] .je-t`);
    };
    // How many pixels in the drawing-unit box `[left, bottom, right, top]`
    // are the proposal's purple, the tag and a laptop's panel hidden: the
    // copies are drawn, not only tagged.
    const purpleIn = async (page, [l, bottom, r, t]) => {
      await still(page);
      const [x0, y0] = await onPage(page, [l, t]);
      const [x1, y1] = await onPage(page, [r, bottom]);
      // Each put back as it was: the laptop's panel is placed, and shown, at each draw.
      const was = await page.evaluate(() =>
        [...document.querySelectorAll('.ctags, .arr:not(.docked)')].map((el) => {
          const v = el.style.visibility;
          el.style.visibility = 'hidden';
          return v;
        }),
      );
      await frames(page);
      const png = decodePng(await page.screenshot({ clip: { x: Math.floor(x0) - 2, y: Math.floor(y0) - 2, width: Math.ceil(x1 - x0) + 4, height: Math.ceil(y1 - y0) + 4 } }));
      await page.evaluate((back) => {
        [...document.querySelectorAll('.ctags, .arr:not(.docked)')].forEach((el, i) => (el.style.visibility = back[i] ?? ''));
      }, was);
      await frames(page);
      // The proposal's purple, #c9a8ff, or a thin dash of it blended into the
      // dark drawing: blue strongest, then red, green weakest. Grey and white
      // ink, the blue halo and red wire numbers are none of these.
      let n = 0;
      for (let i = 0; i < png.rgba.length; i += 4) {
        const [red, green, blue] = [png.rgba[i], png.rgba[i + 1], png.rgba[i + 2]];
        if (blue > 110 && blue - green > 40 && red - green > 12 && blue > red) n += 1;
      }
      return n;
    };
    const shifted = (box, dx, dy) => [box[0] + dx, box[1] + dy, box[2] + dx, box[3] + dy];
    // The live sheet, whole, and a point on the page that picks `id`.
    const pointOn = async (page, id) => {
      await page.waitForFunction(
        () => {
          const p = window.agentdrafterViewer;
          return !document.querySelector('[data-preview]') && p.summary && p.summary.preview == null && p.summary.entry === null;
        },
        null,
        { timeout: 30000 },
      );
      await fitAll(page);
      await still(page);
      const { place, box } = await page.evaluate((wanted) => {
        const p = window.agentdrafterViewer;
        const at = p.summary.objects.findIndex((o) => o.id === wanted);
        return { place: at, box: at >= 0 ? p.bounds([at]) : [] };
      }, id);
      assert(place >= 0 && box.length === 4, `${id} is on the sheet`);
      const [x0, y0, x1, y1] = box;
      const mid = [(x0 + x1) / 2, (y0 + y1) / 2];
      if ((await page.evaluate(([x, y]) => window.agentdrafterViewer.pickAt(x, y, 16), mid)) === place) {
        return { place, box, at: await onPage(page, mid) };
      }
      for (let j = 1; j < 8; j++) {
        for (let i = 1; i < 12; i++) {
          const at = [x0 + ((x1 - x0) * i) / 12, y0 + ((y1 - y0) * j) / 8];
          if ((await page.evaluate(([x, y]) => window.agentdrafterViewer.pickAt(x, y, 16), at)) === place) return { place, box, at: await onPage(page, at) };
        }
      }
      throw new Error(`no point on the page picks ${id}`);
    };
    // A click that selects the object `target` names, snapping off so the
    // click picks rather than reads a snap, waiting until the page marks it.
    // F3 asks for the sheet again, without its snaps and then with them:
    // each is let settle, so the click lands on the sheet drawn.
    // Until the drawing itself is under `at`: a toast of what landed sits
    // over the drawing's top left for a few seconds.
    const clearAt = (page, at) =>
      page.waitForFunction(([x, y]) => document.elementFromPoint(x, y)?.classList.contains('stage'), at, { timeout: 20000 });
    const selectAt = async (page, target) => {
      await clearAt(page, target.at);
      await page.keyboard.press('F3');
      await sleep(800);
      await page.mouse.click(...target.at);
      await page
        .waitForFunction((p) => window.agentdrafterViewer.marked?.picked === p, target.place, { timeout: 15000 })
        .catch(async () => {
          throw new Error(`the click at ${target.at} selected nothing: ${await bar(page)}`);
        });
      await page.keyboard.press('F3');
      await sleep(800);
    };
    // Until the view stops moving, as the glide to the array ends: four
    // frames on end drawn at the same view.
    const still = async (page) => {
      const until = Date.now() + 15000;
      let last = '';
      for (let same = 0; same < 4; ) {
        assert(Date.now() < until, 'the view comes to rest');
        await frames(page);
        const now = JSON.stringify(await page.evaluate(() => window.agentdrafterViewer.view()));
        same = now === last ? same + 1 : 0;
        last = now;
      }
    };
    // The cursor on the drawing point `at`, until it catches the snap
    // `kind` there: the sheet with its snaps, asked for again as F3 turns
    // them back on, may still be on its way. Gives where it is on the page.
    const onSnap = async (page, at, kind) => {
      const until = Date.now() + 20000;
      for (let i = 0; ; i++) {
        await still(page);
        const [x, y] = await onPage(page, at);
        await clearAt(page, [x, y]);
        await page.mouse.move(x + 3 + (i % 2), y + 3);
        await page.mouse.move(x, y);
        await frames(page);
        const h = await page.evaluate(() => window.agentdrafterViewer.hovered());
        if (h?.snap === kind) return [x, y];
        assert(Date.now() < until, `the cursor catches the ${kind} at ${at}: ${JSON.stringify(h)}`);
        await sleep(250);
      }
    };
    const arrayOp = (entry) => {
      const ops = entry.change_set.operations;
      assert(ops.length === 1 && ops[0].op === 'array', `one array operation: ${JSON.stringify(ops)}`);
      return ops[0];
    };
    const panelBox = (page) =>
      page.evaluate(() => {
        const r = document.querySelector('[data-arraypanel]').getBoundingClientRect();
        return [r.left, r.top, r.right, r.bottom];
      });
    const screenBox = async (page, box) => {
      const [l, t] = await onPage(page, [box[0], box[3]]);
      const [r, bottom] = await onPage(page, [box[2], box[1]]);
      return [l, t, r, bottom];
    };
    try {
      // A laptop: the mouse, and typing.
      {
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        // The ribbon: Array in Modify, between Copy and Erase, AR in its tip.
        const modify = await page.evaluate(() => [...document.querySelectorAll('.ribbon .rb[data-tool]')].map((x) => x.dataset.tool));
        assert(JSON.stringify(modify.slice(-4)) === '["move","copy","array","erase"]', `Array sits between Copy and Erase: ${modify}`);
        await page.hover('.rb[data-tool="array"]');
        const tip = await page.textContent('.tip');
        assert(tip.includes('AR') && tip.includes('Esc sends nothing'), `Array's tip: ${tip}`);
        await page.mouse.move(700, 500);
        // FT-101 selected first, then Array from its options: the panel.
        let before = journal().length;
        let target = await pointOn(page, 'DP-LD-101/device');
        const device = target.box;
        await selectAt(page, target);
        assert((await bar(page)).includes('FT-101 selected'), `the device is selected: ${await bar(page)}`);
        assert(/Move.*Copy.*Array.*Erase.*Clear/.test(await bar(page)), `a selected object's options hold Array: ${await bar(page)}`);
        await page.click('.opt[data-opt="array-objects"]');
        await page.waitForSelector('[data-arraypanel]');
        await page.waitForFunction(() => getComputedStyle(document.querySelector('[data-arraypanel]')).visibility === 'visible');
        await sleep(600);
        assert((await bar(page)).includes('ARRAYRECT'), `Array runs, rectangular: ${await bar(page)}`);
        assert((await panelText(page)).includes('of FT-101'), `the panel names it: ${await panelText(page)}`);
        // A field left blank holds it back, saying why: Array off, Enter landing nothing.
        await page.fill('[data-arraypanel] input[data-k="rows"]', '');
        assert((await count(page)) === 'Rows is not a number: type one.', `a blank field is said: ${await count(page)}`);
        assert(await page.isDisabled('[data-arraypanel] [data-act="array"]'), 'Array is off');
        await page.focus('[data-arraypanel] input[data-k="rows"]');
        await page.keyboard.press('Enter');
        await sleep(1000);
        assert(journal().length === before, 'a blank field lands nothing');
        // Rows, columns and spacing typed in the panel: the copies follow.
        await page.fill('[data-arraypanel] input[data-k="rows"]', '3');
        await page.fill('[data-arraypanel] input[data-k="columns"]', '1');
        await page.fill('[data-arraypanel] input[data-k="row_spacing"]', '-1.2');
        // The view comes to a small array as the panel opens: let it settle.
        await sleep(600);
        assert((await count(page)) === '3 × 1: 2 copies', `the panel counts them: ${await count(page)}`);
        assert((await tag(page)) === '3 × 1 · waits for Enter', `the copies are tagged: ${await tag(page)}`);
        const panel = await panelBox(page);
        const deviceOnPage = await screenBox(page, device);
        assert(panel[0] >= deviceOnPage[2] || panel[2] <= deviceOnPage[0], `the panel sits beside the device: ${JSON.stringify({ panel, deviceOnPage })}`);
        const drawn = [await purpleIn(page, shifted(device, 0, -1.2)), await purpleIn(page, shifted(device, 0, -2.4)), await purpleIn(page, shifted(device, 0, 1.2))];
        assert(drawn[0] > 40 && drawn[1] > 40 && drawn[2] < 5, `the copies are drawn in purple where they go, and nowhere else: ${drawn}`);
        seen.laptop.purple = drawn;
        // The total, divided evenly: the field's words, and the spacing it gives.
        await page.click('[data-arraypanel] [data-hold="total"]');
        assert((await fieldValue(page, 'row_total')) === '-2.4', `the total follows the spacing: ${await fieldValue(page, 'row_total')}`);
        assert((await panelText(page)).includes('Rows, first to last, in'), `the field is the total's: ${await panelText(page)}`);
        await page.fill('[data-arraypanel] input[data-k="row_total"]', '-2');
        await frames(page);
        assert((await bar(page)).includes('rows 1.000 in apart down'), `2 over 2 gaps is 1 apart: ${await bar(page)}`);
        await shot(page, 'array-laptop-1-total.png');
        // Esc sends nothing.
        await page.keyboard.press('Escape');
        await page.keyboard.press('Escape');
        await sleep(1000);
        assert(journal().length === before, 'Esc sends nothing');
        assert(!(await page.$('[data-arraypanel]')), 'the panel goes');
        assert((await page.textContent('.cmd .hist')).includes('ARRAY: cancelled, nothing sent'), `the history says so: ${await page.textContent('.cmd .hist')}`);
        // AR typed with the device selected, then values in the command bar:
        // rows,columns, TO and the totals; Between pressed, then Enter lands it.
        target = await pointOn(page, 'DP-LD-101/device');
        await selectAt(page, target);
        await typeIn(page, 'AR');
        await page.waitForSelector('[data-arraypanel]');
        await typeIn(page, '2,2', 'TO', '@1.5,-1.2');
        assert((await count(page)) === '2 × 2: 3 copies', `typed rows and columns: ${await count(page)}`);
        assert((await fieldValue(page, 'column_total')) === '1.5' && (await fieldValue(page, 'row_total')) === '-1.2', 'typed totals');
        // Between, reached by Tab and then clicked, gives the focus up, so
        // Enter lands the array rather than pressing Between again.
        await page.focus('[data-arraypanel] [data-hold="between"]');
        await page.click('[data-arraypanel] [data-hold="between"]');
        assert(!(await page.evaluate(() => document.activeElement?.closest?.('[data-arraypanel] button'))), 'a button pressed keeps no focus');
        assert(journal().length === before, 'nothing lands while it is set up');
        await page.keyboard.press('Enter');
        let entry = await landed(page, before, 'Array, Enter');
        let op = arrayOp(entry);
        assert(
          op.target?.id === 'DP-LD-101/device' && op.kind === 'rectangular' && op.rows === 2 && op.columns === 2 && op.row_spacing === -1.2 && op.column_spacing === 1.5,
          `one rectangular array: ${JSON.stringify(op)}`,
        );
        assert(entry.objects.filter((o) => o.how === 'added').length === 3, `three copies added: ${JSON.stringify(entry.objects)}`);
        assert(JSON.stringify(entry.landed_on) === '["DP-LD-101"]', `the row's own object's copies land on its sheet alone: ${entry.landed_on}`);
        assert((await tag(page)) === null, 'the tag goes when it lands');
        seen.laptop.rectangular = entry.entry;
        let line = await journalLine(page, entry.entry);
        assert(line.includes('array device, 2 × 2'), `one journal line: ${line}`);
        await sideTab(page, 'chat');
        // Set up, then a journal entry opened: dropped, nothing sent.
        before = journal().length;
        target = await pointOn(page, 'DP-LD-101/device');
        await selectAt(page, target);
        await page.click('.opt[data-opt="array-objects"]');
        await page.waitForSelector('[data-arraypanel]');
        await sideTab(page, 'journal');
        await page.click(`[data-entry="${seen.laptop.rectangular}"] .je-head`);
        await page.waitForFunction(() => document.querySelector('.cmdbar')?.textContent.includes('Showing entry'), null, { timeout: 15000 });
        await sleep(1500);
        assert(journal().length === before, `a journal entry opened sends nothing: ${JSON.stringify(journal().at(-1))}`);
        assert(!(await page.$('[data-arraypanel]')), 'the array is dropped');
        await page.click('.opt[data-opt="live"]');
        await page.waitForFunction(() => !document.querySelector('.cmdbar')?.textContent.includes('Showing entry'), null, { timeout: 15000 });
        await sideTab(page, 'chat');
        // Set up, then another sheet asked for, not yet drawn: Array and
        // Enter straight after land nothing, on DP-LD-101 or anywhere.
        before = journal().length;
        target = await pointOn(page, 'DP-LD-101/device');
        await selectAt(page, target);
        await page.click('.opt[data-opt="array-objects"]');
        await page.waitForSelector('[data-arraypanel]');
        // A double click on a text while it is set up opens no text to edit.
        await fitAll(page);
        const service = await objectOf(page, 'DP-LD-101/service');
        const serviceAt = await onPage(page, [(service.bounds[0] + service.bounds[2]) / 2, (service.bounds[1] + service.bounds[3]) / 2]);
        await clearAt(page, serviceAt);
        await page.mouse.dblclick(...serviceAt);
        await sleep(600);
        assert(!(await bar(page)).includes('TEXTEDIT') && (await bar(page)).includes('ARRAYRECT'), `a double click opens no text while Array is set up: ${await bar(page)}`);
        assert(await page.$('[data-arraypanel]'), 'the panel stays');
        // TO typed for it, not entered: it goes with the array, so Enter on
        // the next sheet sends nothing to the assistant (#66, review 2).
        await page.keyboard.type('TO');
        assert((await page.inputValue('.cmdin')) === 'TO', 'TO is typed in the command bar');
        await page.evaluate(() => {
          [...document.querySelectorAll('.sheettabs .tab')].find((t) => t.querySelector('b')?.textContent === 'DP-LD-102').click();
          document.querySelector('[data-arraypanel] [data-act="array"]')?.click();
        });
        assert((await page.inputValue('.cmdin')) === '', `the draft goes with the array, at once: ${await page.inputValue('.cmdin')}`);
        await page.keyboard.press('Enter');
        await page.waitForFunction(() => window.agentdrafterViewer.summary?.sheet === 'DP-LD-102', null, { timeout: 30000 });
        await sleep(1000);
        assert(journal().length === before, `a sheet asked for drops the array, nothing sent: ${JSON.stringify(journal().at(-1))}`);
        assert(!(await page.$('[data-arraypanel]')), 'the panel goes');
        assert(!(await page.textContent('.cmd .hist')).includes('To the assistant'), `nothing goes to the assistant: ${await page.textContent('.cmd .hist')}`);
        assert((await page.inputValue('.cmdin')) === '', 'the command bar is empty on DP-LD-102');
        await page.evaluate(() => [...document.querySelectorAll('.sheettabs .tab')].find((t) => t.querySelector('b')?.textContent === 'DP-LD-101').click());
        await page.waitForFunction(() => window.agentdrafterViewer.summary?.sheet === 'DP-LD-101', null, { timeout: 30000 });
        // Polar: ARRAYPOLAR on the tick, its centre on the circle's Centre snap.
        before = journal().length;
        target = await pointOn(page, tickId);
        await selectAt(page, target);
        await typeIn(page, 'ARRAYPOLAR');
        await page.waitForSelector('[data-arraypanel]', { state: 'attached', timeout: 10000 }).catch(async () => {
          await page.screenshot({ path: join(out, 'array-laptop-polar-failed.png') });
          throw new Error(`ARRAYPOLAR opened no panel: ${await bar(page)} | ${await page.textContent('.cmd .hist')}`);
        });
        assert((await bar(page)).includes('ARRAYPOLAR') && (await bar(page)).includes('Click the centre the items go round'), `it asks for the centre: ${await bar(page)}`);
        // The view comes to the tick as the panel opens: once it is still,
        // the cursor on the circle's centre until it catches the Centre snap.
        const center = await onSnap(page, [3, 8.8], 'center');
        await page.mouse.click(center[0], center[1]);
        await page
          .waitForFunction(() => document.querySelector('[data-acenter]')?.textContent.startsWith('Centre of'), null, { timeout: 15000 })
          .catch(async () => {
            await page.screenshot({ path: join(out, 'array-laptop-centre-failed.png') });
            throw new Error(`the centre clicked is not the circle's Centre snap: ${await page.textContent('[data-acenter]')} | ${await bar(page)}`);
          });
        const centerWords = await page.textContent('[data-acenter]');
        assert(centerWords.includes('3.000, 8.800'), `the centre is the circle's: ${centerWords}`);
        await page.fill('[data-arraypanel] input[data-k="items"]', '8');
        await page.mouse.move(700, 300);
        await sleep(600);
        assert((await fieldValue(page, 'angle_between')) === '45', `eight round a whole turn are 45 apart: ${await fieldValue(page, 'angle_between')}`);
        assert((await count(page)) === '8 round: 7 copies', `the panel counts them: ${await count(page)}`);
        // The copy half a turn round is drawn below the circle, upside down.
        const tick = await objectOf(page, tickId);
        const below = [tick.bounds[0] - 0.02, 2 * 8.8 - tick.bounds[3] - 0.02, tick.bounds[2] + 0.02, 2 * 8.8 - tick.bounds[1] + 0.02];
        await shot(page, 'array-laptop-2-polar.png');
        const polarDrawn = await purpleIn(page, below);
        assert(polarDrawn > 5, `the copy half a turn round is drawn: ${polarDrawn} in ${JSON.stringify(below)}, the view ${JSON.stringify(await page.evaluate(() => window.agentdrafterViewer.view()))}`);
        seen.laptop.polarPurple = polarDrawn;
        await page.click('[data-arraypanel] [data-act="array"]');
        entry = await landed(page, before, 'Array, polar');
        op = arrayOp(entry);
        assert(
          op.kind === 'polar' && op.count === 8 && op.angle === undefined && JSON.stringify(op.center) === JSON.stringify({ snap: { id: circleId, snap: 'center' } }),
          `one polar array round a snapped centre: ${JSON.stringify(op)}`,
        );
        assert(entry.objects.filter((o) => o.how === 'added').length === 7, `seven copies added: ${JSON.stringify(entry.objects)}`);
        seen.laptop.polar = entry.entry;
        line = await journalLine(page, entry.entry);
        assert(line.includes(`array ${tickId.split('/')[1]}, 8 round`), `its journal line: ${line}`);
        await shot(page, 'array-laptop-3-polar-journal.png', true);
        await sideTab(page, 'chat');
        // terminal-2, a template's: picked from the ribbon, warned of, asked about in the band.
        before = journal().length;
        const templateBefore = template();
        target = await pointOn(page, 'DP-LD-101/terminal-2');
        await page.click('.rb[data-tool="array"]');
        assert((await bar(page)).includes('Click the objects to array'), `Array picks first: ${await bar(page)}`);
        await clearAt(page, target.at);
        await page.keyboard.press('F3');
        await sleep(800);
        await page.mouse.click(...target.at);
        await page.keyboard.press('F3');
        assert((await bar(page)).includes('1 object picked'), `terminal-2 is picked: ${await bar(page)}`);
        await page.keyboard.press('Enter');
        await page.waitForSelector('[data-arraypanel]');
        const warned = await page.textContent('[data-warning]');
        assert(
          warned.includes('This is part of the template.') && warned.includes('This arrays it on every sheet built from this template: 7 sheets.') && warned.includes('You will be asked before it lands.'),
          `the command bar warns: ${warned}`,
        );
        await page.fill('[data-arraypanel] input[data-k="rows"]', '3');
        await page.fill('[data-arraypanel] input[data-k="columns"]', '2');
        await page.fill('[data-arraypanel] input[data-k="row_spacing"]', '-0.5');
        await page.fill('[data-arraypanel] input[data-k="column_spacing"]', '1');
        await page.mouse.move(700, 120);
        await sleep(600);
        assert((await count(page)) === '3 × 2: 5 copies', `the panel counts them: ${await count(page)}`);
        assert((await tag(page)) === '3 × 2 · waits for Enter', `the tag: ${await tag(page)}`);
        const terminalDrawn = await purpleIn(page, shifted(target.box, 1, -0.5));
        assert(terminalDrawn > 20, `terminal-2's copies are drawn: ${terminalDrawn}`);
        await shot(page, 'array-laptop-4-template-panel.png');
        // Enter in a field lands it: the band asks first.
        await page.focus('[data-arraypanel] input[data-k="column_spacing"]');
        await page.keyboard.press('Enter');
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        const asked = await page.textContent('[data-preview]');
        assert(asked.includes('Array terminal-2, 3 × 2') && asked.includes('Change all 7 sheets'), `the band: ${asked}`);
        // No tool starts while it asks: Circle picked, then two clicks on
        // the drawing, draw nothing and write nothing.
        const askedTemplate = template();
        await page.click('.rb[data-tool="circle"]');
        assert(!(await bar(page)).includes('CIRCLE'), `Circle does not start: ${await bar(page)}`);
        await page.mouse.click(...(await onPage(page, [13, 3.6])));
        await page.mouse.click(...(await onPage(page, [13.3, 3.6])));
        await sleep(1500);
        assert(!(await bar(page)).includes('CIRCLE'), `Circle has not started: ${await bar(page)}`);
        assert(journal().length === before && template() === askedTemplate, 'nothing lands, nothing is written, before Change all');
        await shot(page, 'array-laptop-5-change-all.png');
        await page.click('[data-pv="go"]');
        entry = await landed(page, before, 'Array of a template object');
        op = arrayOp(entry);
        assert(op.target?.id === 'DP-LD-101/terminal-2' && op.rows === 3 && op.columns === 2 && op.row_spacing === -0.5 && op.column_spacing === 1, `one array: ${JSON.stringify(op)}`);
        assert(entry.landed_on.length === 7 && !entry.landed_on.includes('DP-LD-103'), `it lands on every sheet built from the template: ${entry.landed_on}`);
        assert(template() !== templateBefore, 'the template changes');
        seen.laptop.template = { entry: entry.entry, sheets: entry.landed_on.length };
        await page.mouse.move(700, 600);
        await shot(page, 'array-laptop-6-landed.png', true);
        line = await journalLine(page, entry.entry);
        assert(line.includes('array terminal-2, 3 × 2'), `its journal line: ${line}`);
        await shot(page, 'array-laptop-7-journal.png', true);
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      // A phone: Tools, taps, the panel docked above the command bar.
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        // Snapping off, so each tap is the point itself.
        await page.tap('.toolsbtn');
        await page.tap('.ribbon [data-toggle="osnap"]');
        let before = journal().length;
        await page.tap('.rb[data-tool="array"]', { force: true });
        await page.waitForFunction(
          () => !document.querySelector('.ribbonwrap.open') && (document.querySelector('.cmdbar')?.textContent ?? '').includes('Tap the objects to array'),
          null,
          { timeout: 15000 },
        );
        assert((await bar(page)).includes('Tap the objects to array'), `the phone's words: ${await bar(page)}`);
        let target = await pointOn(page, 'DP-LD-101/device');
        await clearAt(page, target.at);
        await page.touchscreen.tap(...target.at);
        await page
          .waitForFunction(() => (document.querySelector('.cmdbar')?.textContent ?? '').includes('1 object picked'), null, { timeout: 10000 })
          .catch(async () => {
            throw new Error(`the device is picked: ${await bar(page)}`);
          });
        await page.tap('.opt[data-opt="tool:done"]');
        await page.waitForSelector('[data-arraypanel].arr.docked');
        await sleep(600);
        // Docked above the command bar, the device in sight above it.
        await still(page);
        const docked = await panelBox(page);
        const barTop = await page.evaluate(() => document.querySelector('.cmdbar').getBoundingClientRect().top);
        const deviceOnPage = await screenBox(page, target.box);
        assert(docked[3] <= barTop + 1, `the panel sits above the command bar: ${JSON.stringify({ docked, barTop })}`);
        assert(deviceOnPage[3] <= docked[1] && deviceOnPage[1] >= 0, `the device is in sight above the panel: ${JSON.stringify({ deviceOnPage, docked })}`);
        // Cancel sends nothing.
        await page.tap('[data-arraypanel] [data-act="cancel"]');
        await sleep(1000);
        assert(journal().length === before && !(await page.$('[data-arraypanel]')), 'Cancel sends nothing');
        // Again, from the selection: 2 by 2, the total toggled, then Array.
        target = await pointOn(page, 'DP-LD-101/device');
        await clearAt(page, target.at);
        await page.touchscreen.tap(...target.at);
        await sleep(300);
        await page.tap('.opt[data-opt="array-objects"]');
        await page.waitForSelector('[data-arraypanel].arr.docked');
        await page.fill('[data-arraypanel] input[data-k="rows"]', '2');
        await page.fill('[data-arraypanel] input[data-k="columns"]', '2');
        await page.fill('[data-arraypanel] input[data-k="row_spacing"]', '-1.2');
        await page.fill('[data-arraypanel] input[data-k="column_spacing"]', '1.5');
        await page.tap('[data-arraypanel] [data-hold="total"]');
        assert((await panelText(page)).includes('Columns, first to last'), `the total's fields: ${await panelText(page)}`);
        assert((await fieldValue(page, 'column_total')) === '1.5', 'the total of two columns is their spacing');
        await page.evaluate(() => document.activeElement?.blur?.());
        await sleep(600);
        assert((await tag(page)) === '2 × 2', `a phone's tag: ${await tag(page)}`);
        await shot(page, 'array-phone-1-panel.png');
        await page.tap('[data-arraypanel] [data-act="array"]');
        let entry = await landed(page, before, 'Array, by touch');
        let op = arrayOp(entry);
        assert(op.rows === 2 && op.columns === 2 && op.row_spacing === -1.2 && op.column_spacing === 1.5, `one array: ${JSON.stringify(op)}`);
        seen.phone.rectangular = entry.entry;
        await shot(page, 'array-phone-2-landed.png', true);
        // Polar, its centre typed with ⌨ Type. Start Array and let the Tools
        // drawer close and view settle before computing tap coordinates (#313).
        before = journal().length;
        await page.tap('.toolsbtn');
        await page.tap('.rb[data-tool="array"]', { force: true });
        await page.waitForFunction(
          () => !document.querySelector('.ribbonwrap.open') && (document.querySelector('.cmdbar')?.textContent ?? '').includes('Tap the objects to array'),
          null,
          { timeout: 15000 },
        );
        await frames(page);
        target = await pointOn(page, tickId);
        await clearAt(page, target.at);
        await page.touchscreen.tap(...target.at);
        await page
          .waitForFunction(() => (document.querySelector('.cmdbar')?.textContent ?? '').includes('1 object picked'), null, { timeout: 10000 })
          .catch(async () => {
            throw new Error(`the tick is picked: ${await bar(page)}`);
          });
        await page.tap('.opt[data-opt="tool:done"]');
        await page.waitForSelector('[data-arraypanel].arr.docked');
        await page.tap('[data-arraypanel] [data-kind="polar"]');
        assert((await bar(page)).includes('Tap the centre the items go round'), `it asks for the centre: ${await bar(page)}`);
        await page.tap('[data-type]');
        await page.keyboard.type('3,8.8');
        await page.keyboard.press('Enter');
        await page.waitForFunction(() => document.querySelector('[data-acenter]')?.textContent.includes('3.000, 8.800'), null, { timeout: 15000 });
        await page.fill('[data-arraypanel] input[data-k="items"]', '8');
        await page.evaluate(() => document.activeElement?.blur?.());
        await sleep(600);
        assert((await count(page)) === '8 round: 7 copies', `eight round: ${await count(page)}`);
        await shot(page, 'array-phone-3-polar.png');
        await page.tap('[data-arraypanel] [data-act="array"]');
        entry = await landed(page, before, 'Array, polar, by touch');
        op = arrayOp(entry);
        assert(op.kind === 'polar' && op.count === 8 && JSON.stringify(op.center) === '[3,8.8,0]', `a polar array round the centre typed: ${JSON.stringify(op)}`);
        seen.phone.polar = entry.entry;
        // terminal-2: the warning, the band in the command bar's place, Change all.
        before = journal().length;
        await page.tap('.toolsbtn');
        await page.tap('.rb[data-tool="array"]', { force: true });
        await page.waitForFunction(
          () => !document.querySelector('.ribbonwrap.open') && (document.querySelector('.cmdbar')?.textContent ?? '').includes('Tap the objects to array'),
          null,
          { timeout: 15000 },
        );
        await frames(page);
        target = await pointOn(page, 'DP-LD-101/terminal-2');
        await clearAt(page, target.at);
        await page.touchscreen.tap(...target.at);
        await page
          .waitForFunction(() => (document.querySelector('.cmdbar')?.textContent ?? '').includes('1 object picked'), null, { timeout: 10000 })
          .catch(async () => {
            throw new Error(`terminal-2 is picked: ${await bar(page)}`);
          });
        await page.tap('.opt[data-opt="tool:done"]');
        await page.waitForSelector('[data-arraypanel].arr.docked');
        const warned = await page.textContent('[data-warning]');
        assert(warned.includes('Part of the template:') && warned.includes('the copies land on all 7 sheets'), `the warning on a phone: ${warned}`);
        await page.fill('[data-arraypanel] input[data-k="columns"]', '2');
        await page.evaluate(() => document.activeElement?.blur?.());
        await sleep(600);
        await shot(page, 'array-phone-4-template-panel.png');
        await page.tap('[data-arraypanel] [data-act="array"]');
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        const asked = await page.textContent('[data-preview]');
        assert(asked.includes('Change all 7 sheets'), `the band on a phone: ${asked}`);
        await shot(page, 'array-phone-5-change-all.png');
        await page.tap('[data-pv="go"]');
        entry = await landed(page, before, 'Array of a template object, by touch');
        assert(entry.landed_on.length === 7, `on every sheet built from the template: ${entry.landed_on}`);
        seen.phone.template = { entry: entry.entry, sheets: entry.landed_on.length };
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      return { ...seen, screenshots: shots };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * A change set drawn before it lands (#154), against a copy of the demo
   * plant: a template's Move, the service text into the zone line, drawn
   * as it would leave each sheet it reaches, stepped through by ◀ ▶, the
   * arrow keys and the tabs, each tab marked, the checks it would fail
   * clouded and named; nothing written while it is shown; Esc and Cancel
   * landing nothing; Change all landing on all seven sheets. A sheet whose
   * DWG cannot be written refuses it and holds back the rest, nothing can
   * land, and Closed it: check again asks again once it can be. At phone
   * size, 375 by 812, the band takes the command bar's place, and it is
   * stepped, cancelled and changed by touch.
   */
  async preview() {
    const folder = freshPlant('preview');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    const seen = { laptop: {}, phone: {} };
    const set = join(folder, 'demo-plant');
    const SEVEN = ['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302'];
    const journal = () => agentIn(folder, 'journal', 'demo-plant').result?.entries ?? [];
    const band = (page) => page.textContent('[data-preview]');
    const bar = (page) => page.evaluate(() => document.querySelector('.cmdbar').textContent);
    const frames = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    // The sheet shown, drawn as the change set would leave it.
    const shows = (page, sheet) =>
      page.waitForFunction((s) => window.agentdrafterViewer.summary?.sheet === s && !!window.agentdrafterViewer.summary.preview, sheet, { timeout: 30000 });
    const onPage = (page, at) =>
      page.evaluate(([x, y]) => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
      }, at);
    // Shows `at` in the middle of the drawing, `times` the fitted scale.
    const zoomTo = async (page, at, times) => {
      await page.click('[aria-label="Fit the sheet"]');
      await sleep(600);
      await page.evaluate(([x, y, k]) => {
        const p = window.agentdrafterViewer;
        const r = document.querySelector('.stage').getBoundingClientRect();
        const scale = p.view().scale * k;
        p.show({ left: x - r.width / 2 / scale, top: y + r.height / 3 / scale, scale });
      }, [...at, times]);
      await frames(page);
    };
    const typeIn = async (page, ...lines) => {
      for (const line of lines) {
        await page.keyboard.type(line);
        await page.keyboard.press('Enter');
        await frames(page);
      }
    };
    const nothingWritten = (before, what) => {
      const now = filesIn(set);
      assert(JSON.stringify(now) === JSON.stringify(before), `${what}: nothing is written, the journal among it`);
    };
    // The service text, a template's object, moved 2.2 in right, into the
    // zone line on DP-LD-202, DP-LD-301 and DP-LD-302: by the Move tool,
    // the text picked by a click on its letters, the points typed.
    const moveService = async (page) => {
      await zoomTo(page, [1.4, 4.65], 4);
      await typeIn(page, 'M');
      const [x, y] = await onPage(page, [1.2, 4.65]);
      await page.mouse.click(x, y);
      await frames(page);
      assert((await bar(page)).includes('1 object picked'), `the service text is picked: ${await bar(page)}`);
      await typeIn(page, '', '0,0', '@2.2,0');
      await shows(page, 'DP-LD-101');
    };
    const landedOnAll = async (before, what) => {
      const until = Date.now() + 30000;
      while (!grownAndDone(journal(), before)) {
        assert(Date.now() < until, `${what}: nothing landed`);
        await sleep(100);
      }
      const entry = journal().at(-1);
      assert(entry.kind === 'change_set' && JSON.stringify(entry.landed_on) === JSON.stringify(SEVEN), `${what} lands on all seven: ${JSON.stringify(entry.landed_on)}`);
      return entry;
    };
    try {
      {
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        const files = filesIn(set);
        const entries = journal().length;
        // Every loop sheet is ANSI B: one fitted scale.
        const fitted = await page.evaluate(() => window.agentdrafterViewer.view().scale);
        // The view comes to what the preview marks, closer than the fit, and rests.
        const cameToIt = async () => {
          let last = -1;
          for (let i = 0; i < 40; i += 1) {
            const now = await page.evaluate(() => window.agentdrafterViewer.view().scale);
            if (now === last) break;
            last = now;
            await sleep(150);
          }
          assert(last > fitted * 1.5, `the view comes to the change: ${last} against the fit's ${fitted}`);
        };

        // Drawn on the sheet it was made on: the band names it, every tab it reaches says what would become of it.
        await moveService(page);
        await page.waitForSelector('[data-pv-summary]');
        let said = await band(page);
        assert(said.includes('Preview') && said.includes('Move service') && said.includes('on 7 sheets'), `the band names the edit: ${said}`);
        assert(said.includes('DP-LD-101') && said.includes('1 of 7') && said.includes('Would land.'), `the sheet shown would take it: ${said}`);
        assert(
          said.includes('All 7 would take it, but DP-LD-202, DP-LD-301 and DP-LD-302 would then fail a check.') && said.includes('Change all 7 sheets'),
          `the summary: ${said}`,
        );
        const tabs = await tabMarks(page);
        assert(
          JSON.stringify(tabs) ===
            JSON.stringify({ 'DP-LD-101': '✓', 'DP-LD-102': '✓', 'DP-LD-103': 'off', 'DP-LD-104': '✓', 'DP-LD-201': '✓', 'DP-LD-202': '!', 'DP-LD-301': '!', 'DP-LD-302': '!' }),
          `each tab's mark: ${JSON.stringify(tabs)}`,
        );
        assert((await bar(page)).startsWith('>_PREVIEWNothing has landed yet.'), `the command bar: ${await bar(page)}`);
        assert((await page.textContent('[data-pvhead]')).includes('DP-LD-101 as it would be'), 'the drawing says what it shows');
        assert((await page.evaluate(() => window.agentdrafterViewer.summary.preview.change.dry_run)) === true, 'the core drew it as a dry run');
        nothingWritten(files, 'Shown');
        assert(journal().length === entries, 'nothing goes in the journal');
        await cameToIt();
        await page.screenshot({ path: join(out, 'preview-laptop-101.png') });

        // Stepped: the arrow key straight after Move's last point was typed,
        // nothing clicked first, as the command bar's field lets go; ▶; a
        // tab; a sheet it does not reach stays shut.
        await page.keyboard.press('ArrowRight');
        await shows(page, 'DP-LD-102');
        assert((await band(page)).includes('2 of 7'), `→ steps on: ${await band(page)}`);
        await page.click('[data-pv="next"]');
        await shows(page, 'DP-LD-104');
        await page.keyboard.press('ArrowLeft');
        await shows(page, 'DP-LD-102');
        await page.click('.sheettabs .tab:has-text("DP-LD-103")', { force: true });
        await sleep(300);
        assert((await page.evaluate(() => window.agentdrafterViewer.summary.sheet)) === 'DP-LD-102', 'a sheet it does not reach is not shown');
        await page.click('.sheettabs .tab:has-text("DP-LD-301")');
        await shows(page, 'DP-LD-301');
        await page.waitForFunction(() => document.querySelector('[data-pvtag="check"]')?.textContent === 'Would fail: text_on_line', null, { timeout: 15000 });
        said = await band(page);
        assert(said.includes('Would land, but fails a check.') && said.includes('(text_on_line)'), `DP-LD-301 would fail a check: ${said}`);
        assert((await page.textContent('[data-pvtag="check"]')) === 'Would fail: text_on_line', 'the cloud is named');
        // The view came to what it changes.
        const [l, bottom, r, top] = await page.evaluate(() => window.agentdrafterViewer.previewExtent());
        const [x0, y0] = await onPage(page, [l, top]);
        const [x1, y1] = await onPage(page, [r, bottom]);
        const stage = await page.evaluate(() => document.querySelector('.stage').getBoundingClientRect().toJSON());
        await cameToIt();
        assert(x0 >= stage.left && x1 <= stage.right && y0 >= stage.top && y1 <= stage.bottom, 'what it marks is in sight');
        await page.screenshot({ path: join(out, 'preview-laptop-301.png') });
        seen.laptop.stepped = ['DP-LD-102', 'DP-LD-104', 'DP-LD-102', 'DP-LD-301'];

        // Esc cancels: nothing lands, nothing is written, the sheet it was made on comes back.
        await page.keyboard.press('Escape');
        await page.waitForFunction(() => !document.querySelector('[data-preview]'));
        await page.waitForFunction(() => window.agentdrafterViewer.summary?.sheet === 'DP-LD-101' && !window.agentdrafterViewer.summary.preview);
        await sleep(800);
        nothingWritten(files, 'Esc');
        assert(journal().length === entries, 'Esc lands nothing');

        // Changed: it lands on every sheet it reaches.
        await moveService(page);
        await page.click('[data-pv="go"]');
        const entry = await landedOnAll(entries, 'Change all');
        assert(entry.change_set.operations[0].op === 'move' && entry.change_set.operations[0].target.id === 'DP-LD-101/service', `the move: ${JSON.stringify(entry.change_set)}`);
        await page.waitForFunction(() => !document.querySelector('[data-preview]'));
        seen.laptop.changed = { entry: entry.entry, sheets: entry.landed_on.length };
        // Put back, from the command line.
        assert(agentIn(folder, 'undo', 'demo-plant', String(entry.entry)).ok, 'undone');
        await page.waitForFunction((n) => document.querySelector('.titlebar button[data-act="undo"]')?.textContent.includes(`(#${n})`) === false, entry.entry);

        // A sheet whose DWG cannot be written refuses it, and holds back the rest.
        const locked = join(set, 'dwg', 'DP-LD-202.dwg');
        chmodSync(locked, 0o444);
        try {
          const before = filesIn(set);
          const n = journal().length;
          await moveService(page);
          await page.waitForSelector('[data-pv="recheck"]');
          said = await band(page);
          assert(said.includes('Nothing would change. DP-LD-202 would refuse it, so all 7 sheets are held back.'), `the summary: ${said}`);
          assert(said.includes('Held back.'), `DP-LD-101 is held back: ${said}`);
          assert(await page.evaluate(() => document.querySelector('[data-pv="go"]').disabled), 'Change all is off');
          assert((await page.textContent('[data-pv="go"]')) === 'Nothing can land', 'it says nothing can land');
          const marks = await tabMarks(page);
          assert(marks['DP-LD-202'] === '✕' && marks['DP-LD-101'] === '–', `the tabs: ${JSON.stringify(marks)}`);
          await page.click('.sheettabs .tab:has-text("DP-LD-202")');
          await shows(page, 'DP-LD-202');
          await page.waitForFunction(() => document.querySelector('[data-pvtag="sheet"]')?.textContent === 'Refused on DP-LD-202', null, { timeout: 15000 });
          assert((await band(page)).includes('Refused.') && (await band(page)).includes('it may be open in AutoCAD'), `why: ${await band(page)}`);
          await page.screenshot({ path: join(out, 'preview-laptop-refused.png') });
          // Closed it: check again, once it can be written.
          chmodSync(locked, 0o644);
          await page.click('[data-pv="recheck"]');
          await page.waitForFunction(() => {
            const go = document.querySelector('[data-pv="go"]');
            return go && !go.disabled && go.textContent === 'Change all 7 sheets';
          });
          await page.click('[data-pv="cancel"]');
          await page.waitForFunction(() => !document.querySelector('[data-preview]'));
          await sleep(500);
          nothingWritten(before, 'A refused preview, then Cancel');
          assert(journal().length === n, 'nothing lands');
          seen.laptop.refused = 'DP-LD-202';
        } finally {
          chmodSync(locked, 0o644);
        }

        // A change landing while it is shown makes it old: the band says it
        // is drawn again, and Change all waits for that.
        {
          const asked = [];
          page.on('request', (r) => {
            if (r.url().includes('/display-list')) asked.push(JSON.parse(r.postData() ?? '{}'));
          });
          await moveService(page);
          await page.waitForSelector('[data-pv-summary]');
          await page.waitForFunction(() => !document.querySelector('[data-pv="go"]').disabled);
          await page.evaluate(() => {
            window.pvSeen = [];
            const watch = () =>
              window.pvSeen.push({ refreshing: !!document.querySelector('[data-pv-refreshing]'), off: !!document.querySelector('[data-pv="go"]')?.disabled });
            new MutationObserver(watch).observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
          });
          assert(sendChange(folder, move('DP-LD-104/io-channel', 0.001)).ok, 'a change lands from the command line');
          await page.waitForFunction(() => window.pvSeen.some((s) => s.refreshing), null, { timeout: 15000 });
          await page.waitForFunction(() => !document.querySelector('[data-pv-refreshing]') && !document.querySelector('[data-pv="go"]').disabled, null, { timeout: 30000 });
          assert(await page.evaluate(() => window.pvSeen.every((s) => !s.refreshing || s.off)), 'Change all is off while it is drawn again');
          seen.laptop.refreshed = true;

          // A journal entry clicked while it is shown: it ends, landing
          // nothing, and the entry is drawn as it was, never with the change set.
          const n = journal().length;
          const last = journal().at(-1).entry;
          await journalTab(page);
          await page.click(`li[data-entry="${last}"] .je-head`);
          await page.waitForFunction(() => !document.querySelector('[data-preview]'));
          await page.waitForFunction((e) => window.agentdrafterViewer.summary?.entry === e && !window.agentdrafterViewer.summary.preview, last, { timeout: 15000 });
          await sleep(500);
          assert(!asked.some((input) => input.entry !== undefined && input.change_set !== undefined), 'no drawing is asked for at an entry with a change set');
          const toast = await page.evaluate(() => document.querySelector('.va-toast')?.textContent ?? '');
          assert(!toast.includes('not landed'), `nothing is refused: ${toast}`);
          assert(journal().length === n, 'nothing lands');
          seen.laptop.entry = last;
        }
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }

      // A phone, 375 by 812: by touch, the band in the command bar's place.
      {
        const context = await b.newContext({ ...PHONE, viewport: { width: 375, height: 812 } });
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        const files = filesIn(set);
        const entries = journal().length;
        const moveBox = async () => {
          // The junction box, a template's object: Tools, Move, its edge tapped, Done, two taps.
          await zoomTo(page, [6.4, 6.5], 3);
          await page.tap('.toolsbtn');
          await page.tap('.rb[data-tool="move"]', { force: true });
          await page.touchscreen.tap(...(await onPage(page, [6.4, 6.6])));
          await sleep(300);
          assert((await bar(page)).includes('1 object picked'), `the junction box is picked: ${await bar(page)}`);
          await page.tap('.opt[data-opt="tool:done"]');
          await page.touchscreen.tap(...(await onPage(page, [6.4, 6.2])));
          await sleep(400);
          await page.touchscreen.tap(...(await onPage(page, [7.4, 5.2])));
          await shows(page, 'DP-LD-101');
          await page.waitForSelector('[data-pv-summary]');
        };
        await moveBox();
        assert(await page.evaluate(() => document.querySelector('.cmdbar').hidden), 'the band takes the command bar\'s place');
        const said = await band(page);
        assert(said.includes('Move junction-box') && said.includes('1 of 7') && said.includes('Change all 7 sheets'), `the band: ${said}`);
        const box = await page.evaluate(() => document.querySelector('[data-preview]').getBoundingClientRect().toJSON());
        assert(box.left >= 0 && box.right <= 375 && box.bottom <= 812, `the band fits the phone: ${JSON.stringify(box)}`);
        assert(box.height < 812 / 2, `the band leaves the drawing most of the screen: ${box.height}`);
        const tag = await page.evaluate(() => document.querySelector('[data-pvtag]')?.getBoundingClientRect().toJSON());
        assert(!tag || tag.right <= 375, `a tag is kept on the screen: ${JSON.stringify(tag)}`);
        await sleep(1200);
        await page.screenshot({ path: join(out, 'preview-phone-101.png') });
        await page.tap('[data-pv="next"]');
        await shows(page, 'DP-LD-102');
        await page.tap('.sheettabs .tab:has-text("DP-LD-302")');
        await shows(page, 'DP-LD-302');
        assert((await band(page)).includes('7 of 7'), `a tab shows its sheet: ${await band(page)}`);
        await sleep(1200);
        await page.screenshot({ path: join(out, 'preview-phone-302.png') });
        await page.tap('[data-pv="cancel"]');
        await page.waitForFunction(() => !document.querySelector('[data-preview]'));
        await page.waitForFunction(() => window.agentdrafterViewer.summary?.sheet === 'DP-LD-101' && !window.agentdrafterViewer.summary.preview);
        await sleep(800);
        nothingWritten(files, 'Cancel, by touch');
        assert(!(await page.evaluate(() => document.querySelector('.cmdbar').hidden)), 'the command bar is back');
        await moveBox();
        await page.tap('[data-pv="go"]');
        const entry = await landedOnAll(entries, 'Change all, by touch');
        seen.phone.changed = { entry: entry.entry, sheets: entry.landed_on.length };
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      // Circular viewport clipping (#188): a filled solid crossing a circular
      // viewport's edge clips both its amber changed highlight and its faint
      // old-place outline to the circular boundary, with nothing outside it.
      {
        const circ = join(folder, 'circular-vp');
        mkdirSync(join(circ, 'templates'), { recursive: true });
        writeFileSync(join(circ, 'set.yaml'), 'units: mm\noutput: dwg\nrows: rows.yaml\n');
        writeFileSync(
          join(circ, 'templates', 't.yaml'),
          [
            'units: mm',
            'paper: ISO A4 landscape',
            'layout: {name: Plan}',
            'objects:',
            '  - solid: {id: pad, corners: [[115, 50, 0], [160, 50, 0], [160, 160, 0], [115, 160, 0]]}',
            '  - viewport: {id: round, space: paper, clip: {center: [150, 105, 0], radius: 45}, view_center: [150, 105, 0], scale: 1}',
            '',
          ].join('\n'),
        );
        writeFileSync(join(circ, 'rows.yaml'), 'rows:\n  - sheet: S-1\n    template: templates/t.yaml\n');
        const built = agentIn(folder, 'build', 'circular-vp');
        assert(built.ok, `built circular-vp: ${JSON.stringify(built)}`);
        const proposed = await (
          await fetch(`${server.base}/commands/propose`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              set: 'circular-vp',
              change_set: { operations: [{ op: 'move', target: { id: 'S-1/pad' }, by: [15, 0, 0] }] },
              by: 'Assistant',
            }),
          })
        ).json();
        assert(proposed.ok, `proposed on circular-vp: ${JSON.stringify(proposed)}`);
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=circular-vp&sheet=S-1`, 'S-1');
        await page.waitForSelector('[data-pcard="1"] [data-pc="show"]', { timeout: 30000 });
        await page.click('[data-pcard="1"] [data-pc="show"]');
        await shows(page, 'S-1');
        await page.waitForSelector('[data-proposal="1"] [data-pv-summary]', { timeout: 30000 });
        await page.click('[aria-label="Fit the sheet"]');
        await sleep(600);
        await frames(page);
        const sample = async ([x0, y0, x1, y1], test) => {
          const [l, t] = await onPage(page, [x0, y1]);
          const [r, bottom] = await onPage(page, [x1, y0]);
          const png = decodePng(
            await page.screenshot({
              clip: {
                x: Math.floor(l),
                y: Math.floor(t),
                width: Math.max(1, Math.ceil(r - l)),
                height: Math.max(1, Math.ceil(bottom - t)),
              },
            }),
          );
          let n = 0;
          for (let i = 0; i < png.rgba.length; i += 4) {
            if (test(png.rgba[i], png.rgba[i + 1], png.rgba[i + 2])) n += 1;
          }
          return n;
        };
        const isAmber = (r, g, b) => r > 180 && g > 100 && r - b > 100;
        const isOldPlace = (r, g, b) => r > 55 && g > 62 && b > 70 && Math.abs(r - b) < 35;
        // Circle center (150, 105) radius 45 (box [105..195, 60..150]).
        // Moved pad right edge at x=175: inside circle at y=105 (d=25 < 45),
        // outside circle at y=146 (d=sqrt(25^2+41^2)=48.0 > 45, inside box).
        const amberInside = await sample([172, 100, 178, 110], isAmber);
        const amberOutside = await sample([173, 144, 176.5, 148], isAmber);
        // Old pad left edge at x=115: inside circle at y=105 (d=35 < 45),
        // outside circle at y=146 (d=sqrt(35^2+41^2)=53.9 > 45, inside box).
        const oldInside = await sample([112, 100, 118, 110], isOldPlace);
        const oldOutside = await sample([113, 144, 117, 148], (r, g, b) => isAmber(r, g, b) || isOldPlace(r, g, b));
        assert(amberInside > 10, `amber highlight draws inside circular viewport: ${amberInside}`);
        assert(amberOutside === 0, `amber highlight clipped outside circular viewport: ${amberOutside}`);
        assert(oldInside > 5, `old-place outline draws inside circular viewport: ${oldInside}`);
        assert(oldOutside === 0, `old-place outline clipped outside circular viewport: ${oldOutside}`);
        seen.circular = { amberInside, amberOutside, oldInside, oldOutside };
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      return {
        ...seen,
        screenshots: ['preview-laptop-101.png', 'preview-laptop-301.png', 'preview-laptop-refused.png', 'preview-phone-101.png', 'preview-phone-302.png'],
      };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * Editing in quick properties (#153), by mouse and keyboard at laptop
   * size and by touch at phone size, against a copy of the demo plant: a
   * layer, a text's contents in the panel and on the drawing, an attribute,
   * a radius and a diameter, and the Layer box moving the selected object.
   * Each edit is one change set through `change`; Esc lands nothing; a
   * value the row fills says so first and lands on its sheet alone; an edit
   * to the template asks first, Cancel landing nothing and Change all
   * landing on every sheet; Undo last takes an edit back.
   */
  async props() {
    const folder = freshPlant('props');
    const set = join(folder, 'demo-plant');
    // The made-up panel set beside it, whose template joins row values
    // into other text (#102).
    cpSync(join(root, 'examples', 'panel-set'), join(folder, 'panel-set'), { recursive: true, filter: (path) => !path.includes(join('panel-set', 'dwg')) });
    const panel = agentIn(folder, 'build', 'panel-set');
    assert(panel.ok, `the panel set builds: ${JSON.stringify(panel.error)}`);
    // A circle of the sheet's own, for its radius and diameter.
    const added = sendChange(folder, {
      operations: [
        { op: 'add_circle', sheet: 'DP-LD-101', center: [13, 3.6, 0], radius: 0.25, layer: 'notes', id: 'washer' },
        // A second, far from it, clear of its card, which shows the radius the first comes to.
        { op: 'add_circle', sheet: 'DP-LD-101', center: [4.5, 3.4, 0], radius: 0.3, layer: 'notes', id: 'washer2' },
      ],
    });
    assert(added.ok, `the circle is added: ${JSON.stringify(added)}`);
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    const seen = { laptop: {}, phone: {} };
    // The same JSON, whatever the order of its keys.
    const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v);
    const same = (a, b) => JSON.stringify(canon(a)) === JSON.stringify(canon(b));
    const SEVEN = ['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302'];
    const journal = () => agentIn(folder, 'journal', 'demo-plant').result.entries;
    const rows = () => readFileSync(join(set, 'rows.yaml'), 'utf8');
    const template = () => readFileSync(join(set, 'templates', 'loop-2-wire.yaml'), 'utf8');
    const bar = (page) => page.evaluate(() => document.querySelector('.cmdbar').textContent);
    const frames = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    // Waits for the journal to hold one more entry than `before`, a change
    // set that landed, and gives it with its one operation.
    const landed = async (page, before, what) => {
      const until = Date.now() + 30000;
      let entries = journal();
      while (!grownAndDone(entries, before)) {
        assert(Date.now() < until, `${what}: no change set landed: ${await bar(page)}`);
        await sleep(100);
        entries = journal();
      }
      assert(entries.length === before + 1, `${what}: one change set: ${JSON.stringify(entries.slice(before))}`);
      const entry = entries.at(-1);
      assert(entry.kind === 'change_set' && entry.landed_on?.length, `${what} landed: ${JSON.stringify(entry)}`);
      assert(entry.change_set.operations.length === 1, `${what} is one operation: ${JSON.stringify(entry.change_set.operations)}`);
      return { entry, op: entry.change_set.operations[0] };
    };
    const nothingLands = async (before, what) => {
      await sleep(1200);
      assert(journal().length === before, `${what} lands nothing`);
    };
    // The place of the first object of the sheet shown that `test` takes, as the page holds it.
    const placeOf = (page, test) => page.evaluate(`window.agentdrafterViewer.summary.objects.findIndex(${test})`);
    // Where the drawing point `at` is on the page.
    const onPage = (page, at) =>
      page.evaluate(([x, y]) => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
      }, at);
    // Shows the object at `place` large, and gives a point on the page that
    // picks it, as a click there does. The fit's glide is let finish first:
    // its next frame would put back the view it was heading for (#180).
    const showAndFind = async (page, place, zoom = 4) => {
      await page.click('[aria-label="Fit the sheet"]').catch(() => page.tap('[aria-label="Fit the sheet"]'));
      await viewAtRest(page);
      const at = await page.evaluate(
        ([p, zoom]) => {
          const probe = window.agentdrafterViewer;
          const [x0, y0, x1, y1] = probe.bounds([p]);
          const r = document.querySelector('.stage').getBoundingClientRect();
          const scale = probe.view().scale * zoom;
          probe.show({ left: (x0 + x1) / 2 - r.width / 2 / scale, top: (y0 + y1) / 2 + r.height / 4 / scale, scale });
          return [x0, y0, x1, y1];
        },
        [place, zoom],
      );
      await viewAtRest(page);
      return findOn(page, place, at);
    };
    // A point on the page that picks the object at `place`, in its box, at the view shown.
    const findOn = async (page, place, box) => {
      const [x0, y0, x1, y1] = box ?? (await page.evaluate((p) => window.agentdrafterViewer.bounds([p]), place));
      for (let j = 1; j < 8; j++) {
        for (let i = 1; i < 12; i++) {
          const x = x0 + ((x1 - x0) * i) / 12;
          const y = y0 + ((y1 - y0) * j) / 8;
          const picked = await page.evaluate(([x, y]) => window.agentdrafterViewer.pickAt(x, y, 16), [x, y]);
          if (picked === place) return onPage(page, [x, y]);
        }
      }
      throw new Error(`no point on the page picks object ${place}`);
    };
    // Why a click or tap at the page point `at` would not pick the object
    // at `place` now, or null when it would: the view still moving, the
    // renderer picking another object there, or something over the canvas.
    const wontPick = async (page, at, place) => {
      const rest = await viewAtRest(page).then(
        () => null,
        (e) => e.message,
      );
      return (
        rest ??
        page.evaluate(
          ([[x, y], place]) => {
            const probe = window.agentdrafterViewer;
            const v = probe.view();
            const r = document.querySelector('.stage').getBoundingClientRect();
            const picks = probe.pickAt(v.left + (x - r.left) / v.scale, v.top - (y - r.top) / v.scale, 16);
            if (picks !== place) return `the renderer picks ${picks} there, not ${place}`;
            const over = document.elementFromPoint(x, y);
            return over?.closest('.stage') ? null : `${over?.tagName}.${over?.className} is over the canvas there`;
          },
          [at, place],
        )
      );
    };
    // Picks the object at `place` with a click or a tap, once the view is
    // at rest, at a point the renderer picks it at, and waits until it is
    // picked (#180). One that still picks nothing is tried once more from a
    // point found again, and the run says so.
    const select = async (page, place, touch) => {
      for (let attempt = 1; ; attempt += 1) {
        const at = await showAndFind(page, place);
        let missed = await wontPick(page, at, place);
        if (!missed) {
          if (touch) await page.touchscreen.tap(...at);
          else await page.mouse.click(...at);
          const picked = await page
            .waitForFunction((p) => window.agentdrafterViewer.marked?.picked === p, place, { timeout: 8000 })
            .then(
              () => true,
              () => false,
            );
          // Picked late is picked: a second tap on a text picked opens it.
          const now = await page.evaluate(() => window.agentdrafterViewer.marked?.picked);
          if (picked || now === place) {
            await page.waitForSelector('.qp [data-prop="layer"]');
            return at;
          }
          missed = `the ${touch ? 'tap' : 'click'} at ${at.map(Math.round)} picked ${now}`;
        }
        assert(attempt < 2, `object ${place} is not picked, twice: ${missed}`);
        console.log(`     props: picking object ${place} again: ${missed}`);
      }
    };
    const chips = (page) => page.evaluate(() => [...document.querySelectorAll('.qp [data-chip]')].map((c) => c.textContent));
    const valueOf = (page, key) => page.inputValue(`.qp [data-prop="${key}"]`);
    // An attribute's field, by its tag.
    const attributeKey = (page, place, tag) =>
      page.evaluate(([p, t]) => `attribute:${window.agentdrafterViewer.summary.objects[p].values.attributes.findIndex((a) => a.tag === t)}`, [place, tag]);
    const waitValue = (page, key, value) =>
      page.waitForFunction(
        ([k, v]) => document.querySelector(`.qp [data-prop="${k}"]`)?.value === v,
        [key, value],
        { timeout: 15000 },
      );
    try {
      // A laptop: the mouse and the keyboard.
      {
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        // Snapping off, so a click picks.
        await page.keyboard.press('F3');
        await page.waitForFunction(() => window.agentdrafterViewer.summary.objects.some((o) => o.values));

        // A wire number, which the row's WIRE_1 fills in four places.
        const wire = await placeOf(page, "(o) => o.values?.contents?.row_value === 'WIRE_1'");
        await select(page, wire, false);
        const wireChips = await chips(page);
        assert(wireChips.includes('4 places') && wireChips.includes('7 sheets'), `the chips say where its values are written: ${wireChips}`);
        await page.click('.qp [data-prop="contents"]');
        const note = await page.textContent('.qp [data-note="contents"]');
        assert(note.includes('row value WIRE_1') && note.includes('4 places') && note.includes('Only this sheet changes'), `the row value says so before it is edited: ${note}`);
        await page.screenshot({ path: join(out, 'props-laptop-row-value.png') });
        // Esc puts it back and lands nothing.
        let before = journal().length;
        const rowsBefore = rows();
        await page.fill('.qp [data-prop="contents"]', '999');
        await page.keyboard.press('Escape');
        assert((await valueOf(page, 'contents')) === '101+', 'Esc puts the value back');
        assert(await page.isVisible('.qp'), 'Esc in a field keeps the selection');
        await nothingLands(before, 'Esc');
        assert(rows() === rowsBefore, 'Esc writes nothing');
        // Enter lands it: the row's value, on this sheet alone.
        await page.click('.qp [data-prop="contents"]');
        await page.fill('.qp [data-prop="contents"]', '101A+');
        await page.keyboard.press('Enter');
        let { entry, op } = await landed(page, before, 'Contents, in the panel');
        assert(op.op === 'change' && JSON.stringify(op.set) === '{"contents":"101A+"}', `change sets the contents: ${JSON.stringify(op)}`);
        assert(JSON.stringify(entry.landed_on) === '["DP-LD-101"]', `a row value lands on its sheet alone: ${entry.landed_on}`);
        assert(rows().includes('WIRE_1: 101A+') && template() === readFileSync(join(root, 'examples', 'demo-plant', 'templates', 'loop-2-wire.yaml'), 'utf8'), 'the row value is set, the template untouched');
        await waitValue(page, 'contents', '101A+');
        seen.laptop.contents = entry.entry;

        // The row's own device: its layer, then an attribute left by a click away.
        const device = await placeOf(page, "(o) => o.id === 'DP-LD-101/device'");
        await select(page, device, false);
        before = journal().length;
        await page.selectOption('.qp [data-prop="layer"]', 'wire');
        ({ entry, op } = await landed(page, before, 'Layer, in the panel'));
        assert(op.op === 'change' && op.target.id === 'DP-LD-101/device' && op.set.layer === 'wire', `change sets the layer: ${JSON.stringify(op)}`);
        assert(JSON.stringify(entry.landed_on) === '["DP-LD-101"]', 'the row\'s own object reaches its sheet alone');
        seen.laptop.layer = entry.entry;
        await waitValue(page, 'layer', 'wire');
        const loop = await attributeKey(page, device, 'LOOP');
        before = journal().length;
        await page.click(`.qp [data-prop="${loop}"]`);
        await page.fill(`.qp [data-prop="${loop}"]`, '101A');
        // Leaving the field lands it.
        await page.click('.qp header b');
        ({ entry, op } = await landed(page, before, 'An attribute, on leaving the field'));
        assert(
          same(op, { op: 'set_attribute', target: { id: 'DP-LD-101/device' }, tag: 'LOOP', value: '101A' }),
          `set_attribute sets it: ${JSON.stringify(op)}`,
        );
        seen.laptop.attribute = entry.entry;

        // The circle: its radius, then its diameter, which sends its radius.
        const washer = await placeOf(page, "(o) => o.id === 'DP-LD-101/washer'");
        await select(page, washer, false);
        assert((await valueOf(page, 'radius')) === '0.2500' && (await valueOf(page, 'diameter')) === '0.5000', 'the radius and the diameter show');
        before = journal().length;
        await page.fill('.qp [data-prop="radius"]', '0.4');
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'Radius'));
        assert(same(op, { op: 'change', target: { id: 'DP-LD-101/washer' }, set: { radius: 0.4 } }), `the radius: ${JSON.stringify(op)}`);
        seen.laptop.radius = entry.entry;
        await waitValue(page, 'diameter', '0.8000');
        before = journal().length;
        await page.fill('.qp [data-prop="diameter"]', '0.6');
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'Diameter'));
        assert(JSON.stringify(op.set) === '{"radius":0.3}', `a diameter typed sends its radius: ${JSON.stringify(op)}`);
        seen.laptop.diameter = entry.entry;
        await waitValue(page, 'radius', '0.3000');
        // A wrong radius says why and lands nothing.
        before = journal().length;
        await page.fill('.qp [data-prop="radius"]', '-2');
        await page.keyboard.press('Enter');
        await page.waitForSelector('.qp [data-prop-error="radius"]');
        assert((await page.textContent('.qp [data-prop-error="radius"]')).includes('more than 0'), 'a radius of 0 or less is refused');
        await page.keyboard.press('Escape');
        await nothingLands(before, 'A wrong radius');

        // What is typed on one circle does not follow the selection to
        // another whose radius shows the same: leaving the first lands it,
        // and the second shows its own.
        // The sheet's drawing is held back meanwhile, so the second is
        // picked before the first's new radius is drawn.
        let releaseFirst;
        const firstHeld = new Promise((done) => {
          releaseFirst = done;
        });
        await context.route('**/commands/display-list', async (route) => {
          await firstHeld;
          await route.continue().catch(() => {});
        });
        before = journal().length;
        await page.click('.qp [data-prop="radius"]');
        await page.fill('.qp [data-prop="radius"]', '0.45');
        const washer2 = await placeOf(page, "(o) => o.id === 'DP-LD-101/washer2'");
        await select(page, washer2, false);
        ({ entry, op } = await landed(page, before, 'A radius left by picking another circle'));
        assert(op.target.id === 'DP-LD-101/washer' && op.set.radius === 0.45, `the first circle's radius lands: ${JSON.stringify(op)}`);
        assert((await valueOf(page, 'radius')) === '0.3000', `the second circle shows its own radius: ${await valueOf(page, 'radius')}`);
        before = journal().length;
        await page.click('.qp [data-prop="radius"]');
        await page.click('.qp header b');
        await nothingLands(before, 'Focusing and leaving the second circle');
        releaseFirst();
        await sleep(300);
        await context.unroute('**/commands/display-list');
        seen.laptop.draft_stays = entry.entry;

        // A field landed with Enter, focused again and left before the
        // sheet is drawn again, while it still shows what was sent, sends
        // nothing more: the sheet's drawing is held back meanwhile.
        const cabinet = await placeOf(page, "(o) => o.id === 'DP-LD-101/cabinet-tag'");
        await select(page, cabinet, false);
        let releaseSheet;
        const sheetHeld = new Promise((done) => {
          releaseSheet = done;
        });
        await context.route('**/commands/display-list', async (route) => {
          await sheetHeld;
          await route.continue().catch(() => {});
        });
        before = journal().length;
        await page.click('.qp [data-prop="contents"]');
        await page.fill('.qp [data-prop="contents"]', 'CONTROL PANEL CP-100A');
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'Contents, by Enter'));
        await page.click('.qp [data-prop="contents"]');
        assert((await valueOf(page, 'contents')) === 'CONTROL PANEL CP-100A', 'the field shows what was sent');
        await page.keyboard.press('Tab');
        await nothingLands(before + 1, 'Leaving it again before the sheet is drawn again');
        releaseSheet();
        await sleep(300);
        await context.unroute('**/commands/display-list');
        await waitValue(page, 'contents', 'CONTROL PANEL CP-100A');
        seen.laptop.left_again = entry.entry;

        // A change that fails while the field has the focus again: the
        // field shows its value again, and the same value typed again
        // lands. The change is held, then fails, made up.
        await select(page, await placeOf(page, "(o) => o.id === 'DP-LD-101/washer2'"), false);
        let failChange;
        const changeHeld = new Promise((done) => {
          failChange = done;
        });
        await context.route('**/commands/change', async (route) => {
          await changeHeld;
          await route
            .fulfill({
              status: 422,
              contentType: 'application/json',
              body: JSON.stringify({ ok: false, error: { code: 'refused', message: 'a made-up failure of the change' } }),
            })
            .catch(() => {});
        });
        before = journal().length;
        await page.click('.qp [data-prop="radius"]');
        await page.fill('.qp [data-prop="radius"]', '0.55');
        await page.keyboard.press('Enter');
        await page.click('.qp [data-prop="radius"]');
        assert((await valueOf(page, 'radius')) === '0.55', 'the field shows what was sent');
        failChange();
        await waitValue(page, 'radius', '0.3000');
        await context.unroute('**/commands/change');
        assert(await page.evaluate(() => document.activeElement?.getAttribute('data-prop') === 'radius'), 'the field has the focus still');
        assert(journal().length === before, 'the failed change landed nothing');
        await page.fill('.qp [data-prop="radius"]', '0.55');
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'The same radius typed again after a failed change'));
        assert(same(op, { op: 'change', target: { id: 'DP-LD-101/washer2' }, set: { radius: 0.55 } }), `the radius typed again lands: ${JSON.stringify(op)}`);
        await waitValue(page, 'radius', '0.5500');
        seen.laptop.retry_after_failure = entry.entry;

        // A radius pasted with a line break is only checked: it lands on
        // Enter, not as it is pasted.
        before = journal().length;
        await page.click('.qp [data-prop="radius"]');
        await page.evaluate(() => {
          const el = document.querySelector('.qp [data-prop="radius"]');
          el.select();
          const data = new DataTransfer();
          data.setData('text/plain', '0.6\n');
          el.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));
        });
        await nothingLands(before, 'A radius pasted, before Enter');
        assert(!(await page.$('.qp [data-prop-error="radius"]')), 'a number pasted with a line break is not refused');
        assert((await valueOf(page, 'radius')) === '0.6', `the field shows what was pasted: ${await valueOf(page, 'radius')}`);
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'A pasted radius, on Enter'));
        assert(same(op, { op: 'change', target: { id: 'DP-LD-101/washer2' }, set: { radius: 0.6 } }), `the pasted radius lands on Enter: ${JSON.stringify(op)}`);
        await waitValue(page, 'radius', '0.6000');
        seen.laptop.paste = entry.entry;

        // A text on the drawing: a double click on the service text.
        const service = await placeOf(page, "(o) => o.id === 'DP-LD-101/service'");
        const [sx, sy] = await showAndFind(page, service);
        await page.mouse.dblclick(sx, sy);
        await page.waitForSelector('[data-text-edit]');
        await page.waitForFunction(() => getComputedStyle(document.querySelector('.tedit')).opacity === '1');
        // It takes the keys as it opens, the text selected, as AutoCAD's does.
        assert(await page.evaluate(() => document.activeElement?.matches('[data-text-edit]')), 'the text on the drawing takes the keys');
        assert((await bar(page)).includes('TEXTEDIT'), `the command bar says the text is edited on the drawing: ${await bar(page)}`);
        const onDrawing = await page.textContent('.tedit');
        assert(onDrawing.includes('row value SERVICE'), `the box says where it is written: ${onDrawing}`);
        await page.screenshot({ path: join(out, 'props-laptop-text-on-drawing.png') });
        before = journal().length;
        await page.keyboard.press('Escape');
        await page.waitForFunction(() => !document.querySelector('[data-text-edit]'));
        await nothingLands(before, 'Esc on the drawing');
        await page.mouse.dblclick(sx, sy);
        await page.waitForSelector('[data-text-edit]');
        // Typed with the keys, as a drafter does: End, then the words.
        await page.keyboard.press('End');
        await page.keyboard.type(' METER');
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'Contents, on the drawing'));
        assert(op.op === 'change' && op.target.id === 'DP-LD-101/service' && op.set.contents === 'FT-101 RAW WATER INLET FLOW METER', `the text on the drawing: ${JSON.stringify(op)}`);
        assert(rows().includes('SERVICE: FT-101 RAW WATER INLET FLOW METER'), 'its row value is set');
        seen.laptop.text_on_drawing = entry.entry;

        // An attribute the template leaves unset: Cancel, then Change all.
        const title = await placeOf(page, "(o) => o.id === 'DP-LD-101/title-block'");
        await select(page, title, false);
        const checked = await attributeKey(page, title, 'CHECKED');
        const templateBefore = template();
        const ask = async () => {
          await page.click(`.qp [data-prop="${checked}"]`);
          const said = await page.textContent(`.qp [data-note="${checked}"]`);
          assert(said.includes('leaves CHECKED unset') && said.includes('all 7 sheets'), `it says so before it is edited: ${said}`);
          await page.fill(`.qp [data-prop="${checked}"]`, 'ZM');
          await page.keyboard.press('Enter');
          await page.waitForSelector('[data-preview] [data-pv-summary]');
          const asked = await page.textContent('[data-preview]');
          assert(asked.includes('CHECKED') && asked.includes('Change all 7 sheets'), `the band: ${asked}`);
          const tabs = await tabMarks(page);
          assert(SEVEN.every((s) => tabs[s] === '✓') && tabs['DP-LD-103'] === 'off', `the tabs: ${JSON.stringify(tabs)}`);
        };
        before = journal().length;
        await ask();
        await page.screenshot({ path: join(out, 'props-laptop-template.png') });
        await page.click('[data-pv="cancel"]');
        await nothingLands(before, 'Cancel');
        assert(template() === templateBefore, 'Cancel writes nothing');
        await waitValue(page, checked, '');
        await ask();
        await page.click('[data-pv="go"]');
        ({ entry, op } = await landed(page, before, 'An unset attribute, Change all'));
        assert(same(op, { op: 'set_attribute', target: { id: 'DP-LD-101/title-block' }, tag: 'CHECKED', value: 'ZM' }), `set_attribute: ${JSON.stringify(op)}`);
        assert(JSON.stringify(entry.landed_on) === JSON.stringify(SEVEN), `it lands on every sheet built from the template: ${entry.landed_on}`);
        assert(template().includes('CHECKED: ZM'), 'it is set on the template');
        seen.laptop.template_attribute = { entry: entry.entry, sheets: entry.landed_on.length };

        // The Layer box moves the selected object: a template's text.
        const room = await placeOf(page, "(o) => o.values?.contents?.value === 'CONTROL ROOM'");
        await select(page, room, false);
        const boxSays = await page.textContent('.layerbox small');
        assert(boxSays.startsWith('Pick a layer to move'), `the Layer box says it moves the object: ${boxSays}`);
        before = journal().length;
        const beforeMove = template();
        await page.selectOption('select[data-layer]', 'wire');
        await page.waitForSelector('[data-preview] [data-pv-summary]');
        const asked = await page.textContent('[data-preview]');
        assert(asked.includes('moved to layer wire') && asked.includes('Change all 7 sheets'), `the band: ${asked}`);
        await page.click('[data-pv="go"]');
        ({ entry, op } = await landed(page, before, 'The Layer box'));
        assert(op.op === 'change' && op.set.layer === 'wire' && entry.landed_on.length === 7, `the Layer box moves it, on every sheet: ${JSON.stringify(op)} ${entry.landed_on}`);
        seen.laptop.layer_box = { entry: entry.entry, sheets: entry.landed_on.length };
        // Undo last takes it back.
        await page.waitForFunction(
          (n) => {
            const b = document.querySelector('.titlebar button[data-act="undo"]');
            return b && !b.disabled && b.textContent.includes(`(#${n})`);
          },
          entry.entry,
          { timeout: 30000 },
        );
        await page.click('.titlebar button[data-act="undo"]');
        const until = Date.now() + 20000;
        let undone = journal().at(-1);
        while (!(undone.kind === 'undo' && finished(undone) && template() === beforeMove)) {
          assert(Date.now() < until, `Undo last takes the edit back: ${JSON.stringify(undone)}`);
          await sleep(100);
          undone = journal().at(-1);
        }
        assert(JSON.stringify(undone.undid) === JSON.stringify([entry.entry]), `the undo takes the edit: ${JSON.stringify(undone)}`);
        seen.laptop.undo = undone.entry;

        // An entry asked for whose drawing has not come yet: the live
        // drawing is still on the canvas, and quick properties and the
        // Layer box edit nothing while it is.
        let releaseEntry;
        const entryHeld = new Promise((done) => {
          releaseEntry = done;
        });
        await context.route('**/commands/display-list', async (route) => {
          if (route.request().postDataJSON()?.entry !== undefined) await entryHeld;
          await route.continue().catch(() => {});
        });
        await journalTab(page);
        await page.click('button[aria-label="Older"]');
        await page.waitForFunction(() => document.querySelector('.cmdbar').textContent.includes('as it was'));
        const deviceNow = await placeOf(page, "(o) => o.id === 'DP-LD-101/device'");
        await page.mouse.click(...(await showAndFind(page, deviceNow, 3)));
        await page.waitForSelector('.qp');
        assert((await page.locator('.qp [data-prop]').count()) === 0, 'quick properties are read-only');
        assert((await page.locator('select[data-layer]').count()) === 0, 'the Layer box moves nothing');
        releaseEntry();
        await sleep(300);
        await context.unroute('**/commands/display-list');
        await page.click('button[data-act="live"]');
        seen.laptop.read_only_while_coming = true;
        // The made-up failure of a change is the one 422.
        assert(problems.filter((p) => !p.includes('422')).length === 0, problems.join('; '));
        await context.close();
      }

      // A phone: taps, its keyboard and the text box.
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        await page.tap('.toolsbtn');
        await page.tap('.ribbon [data-toggle="osnap"]');
        await page.tap('.toolsbtn');
        await page.waitForFunction(() => window.agentdrafterViewer.summary.objects.some((o) => o.values));

        // Two quick taps on two texts within 30 pixels are two picks: the
        // second selects its own and opens no text box.
        {
          const service = await placeOf(page, "(o) => o.id === 'DP-LD-101/service'");
          const signalFirst = await placeOf(page, "(o) => o.id === 'DP-LD-101/signal'");
          const a = await showAndFind(page, service, 2);
          const c = await findOn(page, signalFirst);
          assert(Math.hypot(a[0] - c[0], a[1] - c[1]) < 30, `the two taps are within 30 pixels: ${a} ${c}`);
          // When each finger lifted, as the gestures time it.
          await page.evaluate(() => {
            window.liftedAt = [];
            window.addEventListener('pointerup', (e) => window.liftedAt.push(e.timeStamp), true);
          });
          await page.touchscreen.tap(...a);
          await page.touchscreen.tap(...c);
          const lifted = await page.evaluate(() => window.liftedAt);
          assert(lifted.length === 2 && lifted[1] - lifted[0] < 320, `the two taps are inside the double tap's 320 ms: ${lifted}`);
          await sleep(600);
          assert(!(await page.$('[data-text-box]')), 'two taps on two objects open no text box');
          const picked = await page.evaluate(() => window.agentdrafterViewer.marked?.picked);
          assert(picked === signalFirst, `the second tap selects the second text: ${picked}, not ${signalFirst}`);
          await page.tap('.qp [data-close]');
          await page.waitForFunction(() => !document.querySelector('.qp'));
          seen.phone.two_taps = picked;
        }

        // A text: tapped, then tapped again, edited in the box.
        const signal = await placeOf(page, "(o) => o.id === 'DP-LD-101/signal'");
        await select(page, signal, true);
        // The card may bring it up above itself: found again where it is now.
        await sleep(800);
        await page.touchscreen.tap(...(await findOn(page, signal)));
        await page.waitForSelector('[data-text-box]');
        assert((await page.textContent('[data-text-box]')).includes('row value SIGNAL'), 'the box says where it is written');
        await page.fill('[data-text-box] [data-tbox-text]', '0-600 GPM = 4-20 mA');
        await page.screenshot({ path: join(out, 'props-phone-text.png') });
        let before = journal().length;
        await page.tap('[data-tbox-done]');
        let { entry, op } = await landed(page, before, 'Contents, by touch');
        assert(op.op === 'change' && op.target.id === 'DP-LD-101/signal' && op.set.contents === '0-600 GPM = 4-20 mA', `the text by touch: ${JSON.stringify(op)}`);
        seen.phone.text = entry.entry;

        // The device's attribute, in the card, with the phone's keyboard.
        const device = await placeOf(page, "(o) => o.id === 'DP-LD-101/device'");
        await select(page, device, true);
        const func = await attributeKey(page, device, 'FUNC');
        before = journal().length;
        await page.tap(`.qp [data-prop="${func}"]`);
        await page.fill(`.qp [data-prop="${func}"]`, 'FIT');
        await page.screenshot({ path: join(out, 'props-phone-attribute.png') });
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'An attribute, by touch'));
        assert(op.op === 'set_attribute' && op.tag === 'FUNC' && op.value === 'FIT', `the attribute by touch: ${JSON.stringify(op)}`);
        seen.phone.attribute = entry.entry;
        // Its layer, in the card.
        before = journal().length;
        await page.selectOption('.qp [data-prop="layer"]', 'notes');
        ({ entry, op } = await landed(page, before, 'Layer, by touch'));
        assert(op.set.layer === 'notes', `the layer by touch: ${JSON.stringify(op)}`);
        seen.phone.layer = entry.entry;
        // The Layer box, in Tools, moves it again.
        // The sheet drawn again with it, not only the field, which holds what was sent until then.
        await page.waitForFunction(() => window.agentdrafterViewer.summary.objects.find((o) => o.id === 'DP-LD-101/device')?.layer === 'notes', null, { timeout: 15000 });
        before = journal().length;
        await page.tap('.toolsbtn');
        await page.selectOption('select[data-layer]', 'wire');
        ({ entry, op } = await landed(page, before, 'The Layer box, by touch'));
        assert(op.target.id === 'DP-LD-101/device' && op.set.layer === 'wire', `the Layer box by touch: ${JSON.stringify(op)}`);
        seen.phone.layer_box = entry.entry;
        if (await page.isVisible('.ribbon [data-toggle="osnap"]')) await page.tap('.toolsbtn');

        // The circle's radius and diameter. The sheet is drawn again with
        // the device on `wire`, the revision the journal now ends at, before
        // the washer is found: a sheet drawn again after the Fit would fit
        // it again, and move the washer from under the tap (#180).
        await page.waitForFunction(() => window.agentdrafterViewer.summary.objects.find((o) => o.id === 'DP-LD-101/device')?.layer === 'wire', null, { timeout: 15000 });
        const washer = await placeOf(page, "(o) => o.id === 'DP-LD-101/washer'");
        await select(page, washer, true);
        before = journal().length;
        await page.tap('.qp [data-prop="radius"]');
        await page.fill('.qp [data-prop="radius"]', '0.35');
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'Radius, by touch'));
        assert(JSON.stringify(op.set) === '{"radius":0.35}', `the radius by touch: ${JSON.stringify(op)}`);
        seen.phone.radius = entry.entry;
        await waitValue(page, 'diameter', '0.7000');
        before = journal().length;
        await page.tap('.qp [data-prop="diameter"]');
        await page.fill('.qp [data-prop="diameter"]', '1');
        await page.keyboard.press('Enter');
        ({ entry, op } = await landed(page, before, 'Diameter, by touch'));
        assert(JSON.stringify(op.set) === '{"radius":0.5}', `a diameter typed by touch sends its radius: ${JSON.stringify(op)}`);
        seen.phone.diameter = entry.entry;

        // A template's own text, in the card: Cancel, then Change all.
        const box = await placeOf(page, "(o) => o.values?.contents?.value === 'JUNCTION BOX'");
        await select(page, box, true);
        const templateBefore = template();
        const ask = async () => {
          await page.tap('.qp [data-prop="contents"]');
          await page.fill('.qp [data-prop="contents"]', 'JUNCTION BOXES');
          await page.keyboard.press('Enter');
          await page.waitForSelector('[data-preview] [data-pv-summary]');
          const asked = await page.textContent('[data-preview]');
          assert(asked.includes('JUNCTION BOXES') && asked.includes('Change all 7 sheets'), `the band: ${asked}`);
          // On a phone the band takes the command bar's place.
          assert(await page.evaluate(() => document.querySelector('.cmdbar').hidden), 'the band replaces the command bar');
        };
        before = journal().length;
        await ask();
        await page.screenshot({ path: join(out, 'props-phone-template.png') });
        await page.tap('[data-pv="cancel"]');
        await nothingLands(before, 'Cancel, by touch');
        assert(template() === templateBefore, 'Cancel writes nothing');
        await waitValue(page, 'contents', 'JUNCTION BOX');
        await ask();
        await page.tap('[data-pv="go"]');
        ({ entry, op } = await landed(page, before, 'A template text, Change all, by touch'));
        assert(op.set.contents === 'JUNCTION BOXES' && JSON.stringify(entry.landed_on) === JSON.stringify(SEVEN), `Change all by touch: ${JSON.stringify(op)} ${entry.landed_on}`);
        seen.phone.template_text = { entry: entry.entry, sheets: entry.landed_on.length };
        // Undo last, in the peek, takes it back.
        await page.waitForFunction(
          (n) => [...document.querySelectorAll('button[data-act="undo"]')].some((b) => !b.disabled && b.textContent.includes(`(#${n})`)),
          entry.entry,
          { timeout: 30000 },
        );
        await page.tap('button[data-act="undo"]:visible');
        const until = Date.now() + 20000;
        let undone = journal().at(-1);
        while (!(undone.kind === 'undo' && finished(undone) && template() === templateBefore)) {
          assert(Date.now() < until, `Undo last takes the edit back, by touch: ${JSON.stringify(undone)}`);
          await sleep(100);
          undone = journal().at(-1);
        }
        seen.phone.undo = undone.entry;
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }

      // A value joined from a list of parts (#102), on the made-up panel
      // set beside the plant: it edits as a row value does, its note names
      // its row values, and the whole text goes in the change the page
      // sends, which the core matches to the parts.
      {
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=panel-set&sheet=PNL-001`, 'PNL-001');
        await live(page);
        await page.keyboard.press('F3');
        const panelRows = () => readFileSync(join(folder, 'panel-set', 'rows.yaml'), 'utf8');
        const panelJournal = () => agentIn(folder, 'journal', 'panel-set').result.entries;
        const heading = await placeOf(page, "(o) => o.id === 'PNL-001/heading'");
        await select(page, heading, false);
        const said = await chips(page);
        assert(said.includes('2 row values'), `the chip says its row values: ${said}`);
        await page.click('.qp [data-prop="contents"]');
        const note = await page.textContent('.qp [data-note="contents"]');
        assert(note.includes('row values PANEL and SLOT') && note.includes('Only this sheet changes'), `the note names its row values: ${note}`);
        const before = panelJournal().length;
        await page.fill('.qp [data-prop="contents"]', 'PANEL P9 SLOT 5');
        await page.keyboard.press('Enter');
        assert((await page.locator('[data-preview]').count()) === 0, 'no preview: it lands at once');
        const until = Date.now() + 30000;
        while (!grownAndDone(panelJournal(), before)) {
          assert(Date.now() < until, `the joined text lands: ${await bar(page)}`);
          await sleep(100);
        }
        const entry = panelJournal().at(-1);
        const op = entry.change_set.operations[0];
        assert(same(op, { op: 'change', target: { id: 'PNL-001/heading' }, set: { contents: 'PANEL P9 SLOT 5' } }), `the whole text is sent: ${JSON.stringify(op)}`);
        assert(JSON.stringify(entry.landed_on) === '["PNL-001"]', `it lands on its sheet alone: ${entry.landed_on}`);
        assert(panelRows().includes('PANEL: P9') && panelRows().includes('SLOT: 5'), 'each row value is set');
        await waitValue(page, 'contents', 'PANEL P9 SLOT 5');
        await page.screenshot({ path: join(out, 'props-laptop-parts.png') });
        seen.laptop.parts = entry.entry;
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      return {
        ...seen,
        screenshots: [
          'props-laptop-row-value.png',
          'props-laptop-text-on-drawing.png',
          'props-laptop-template.png',
          'props-phone-text.png',
          'props-phone-attribute.png',
          'props-phone-template.png',
          'props-laptop-parts.png',
        ],
      };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /**
   * Object snaps: the cursor catches the demo plant's own, as `query snap`
   * gives them, with no round trip; a click reads the point out and Copy
   * gives the snapped point a change set takes; the menu and F3 change the
   * running snaps, kept across a reload; a phone's held finger brings up
   * the magnifier and lifting takes the snap.
   */
  async snaps() {
    const folder = freshPlant('snaps');
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    const seen = {};
    // FT-101's endpoint `objects[3].from`, as `query snap` gives it.
    const asked = agentIn(folder, 'query', 'snap', 'demo-plant', '--id', 'DP-LD-101/device', '--snap', 'endpoint', '--which', 'objects[3].from');
    assert(asked.ok && asked.result.points.length === 1, `query snap: ${JSON.stringify(asked)}`);
    const end = asked.result.points[0].at;
    // The two snaps of the running kinds nearest each other, from the display list's own, for Tab.
    const listed = agentIn(folder, 'display-list', 'demo-plant', '--sheet', 'DP-LD-101', '--snaps').result.snaps;
    const runs = (k) => ['endpoint', 'midpoint', 'center', 'intersection'].includes(k);
    let pair = null;
    for (let i = 0; i < listed.length; i++) {
      for (let j = i + 1; j < listed.length; j++) {
        const [a, c] = [listed[i], listed[j]];
        const d = Math.hypot(a.at[0] - c.at[0], a.at[1] - c.at[1]);
        if (d > 1e-6 && runs(a.snap) && runs(c.snap) && (!pair || d < pair[2])) pair = [a, c, d];
      }
    }
    // Shows `at`, drawing units, at the middle of the drawing, `zoom` times the fitted scale.
    const centre = (page, at, zoom) =>
      page.evaluate(
        ([x, y, zoom]) => {
          const p = window.agentdrafterViewer;
          const r = document.querySelector('.stage').getBoundingClientRect();
          const scale = p.view().scale * zoom;
          p.show({ left: x - r.width / 2 / scale, top: y + r.height / 3 / scale, scale });
        },
        [at[0], at[1], zoom],
      );
    const onPage = (page, at) =>
      page.evaluate(([x, y]) => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
      }, at);
    const frames = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    const hovered = (page) => page.evaluate(() => window.agentdrafterViewer.hovered());
    try {
      // A laptop, with a mouse.
      {
        const context = await b.newContext({ ...LAPTOP, permissions: ['clipboard-read', 'clipboard-write'] });
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        const count = await page.evaluate(() => window.agentdrafterViewer.summary.snaps);
        assert(count === listed.length && count > 100, `the sheet came with its snaps: ${count}`);
        await centre(page, end, 6);
        await frames(page);
        const [x, y] = await onPage(page, end);
        await page.mouse.move(x + 5, y + 4);
        await frames(page);
        const h = await hovered(page);
        assert(
          h && h.snap === 'endpoint' && h.which === 'objects[3].from' && JSON.stringify(h.at) === JSON.stringify(end),
          `the cursor catches FT-101's endpoint where query snap puts it: ${JSON.stringify({ h, end })}`,
        );
        const tag = await page.textContent('.snaptag');
        const coords = await page.textContent('.coords');
        assert(tag === 'Endpoint · FT-101', `the tag names it: ${tag}`);
        assert(coords === `${end[0].toFixed(4)}, ${end[1].toFixed(4)} inEndpoint`, `the status bar shows the snapped point exactly: ${coords}`);
        // The marker is drawn: AutoSnap yellow round the point, on the screen.
        const shot = decodePng(await page.screenshot({ clip: { x: x - 12, y: y - 12, width: 24, height: 24 } }));
        let yellow = 0;
        for (let i = 0; i < shot.rgba.length; i += 4) {
          const [r, g, bl] = [shot.rgba[i], shot.rgba[i + 1], shot.rgba[i + 2]];
          if (Math.abs(r - 0xf2) < 24 && Math.abs(g - 0xe2) < 24 && Math.abs(bl - 0x4b) < 40) yellow++;
        }
        assert(yellow > 20, `the marker is drawn at the point: ${yellow} yellow pixels`);
        await page.screenshot({ path: join(out, 'snaps-laptop-endpoint.png') });
        // A click reads the point out and selects its object; Copy gives the snapped point.
        await page.mouse.click(x + 5, y + 4);
        await page.waitForSelector('.qp');
        const bar = await page.textContent('.cmdbar');
        assert(bar.includes(`Endpoint of FT-101: ${end[0].toFixed(4)}, ${end[1].toFixed(4)} in [ Copy as snapped point Clear ]`), `the read-out: ${bar}`);
        await page.click('.opt[data-opt="copy"]');
        const copied = JSON.parse(await page.evaluate(() => navigator.clipboard.readText()));
        assert(
          JSON.stringify(copied) === JSON.stringify({ snap: { id: 'DP-LD-101/device', snap: 'endpoint', which: 'objects[3].from' } }),
          `Copy gives the snapped point: ${JSON.stringify(copied)}`,
        );
        // A change set takes it as it is.
        writeFileSync(join(folder, 'from-snap.json'), JSON.stringify({ operations: [{ op: 'add_line', sheet: 'DP-LD-101', layer: '0', from: copied, to: [5, 5, 0] }] }));
        const dry = agentIn(folder, 'change', 'demo-plant', 'from-snap.json', '--dry-run');
        const snapped = dry.result?.operations?.[0]?.snapped?.[0];
        assert(dry.ok && JSON.stringify(snapped?.at) === JSON.stringify(end), `a change set takes the copied point: ${JSON.stringify(dry.error ?? snapped)}`);
        await page.screenshot({ path: join(out, 'snaps-laptop-taken.png') });
        // Another sheet asked for: the read-out, of an object not on it, goes.
        await page.click('.tab:has-text("DP-LD-102")');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-102');
        assert(!(await page.$('.opt[data-opt="copy"]')), 'Copy goes with the sheet its object is on');
        await page.click('.tab:has-text("DP-LD-101")');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-101');
        // The sheet's tab keeps the keys until the drawing takes them back.
        await page.evaluate(() => document.activeElement?.blur());
        await centre(page, end, 6);
        await frames(page);
        // Tab steps to the next snap in reach.
        assert(pair, 'two snaps lie near each other');
        {
          const between = [(pair[0].at[0] + pair[1].at[0]) / 2, (pair[0].at[1] + pair[1].at[1]) / 2];
          // Shown so the two are 10 CSS pixels apart: both in reach of the point between them.
          await page.evaluate(
            ([x, y, scale]) => {
              const r = document.querySelector('.stage').getBoundingClientRect();
              window.agentdrafterViewer.show({ left: x - r.width / 2 / scale, top: y + r.height / 3 / scale, scale });
            },
            [between[0], between[1], 10 / pair[2]],
          );
          await frames(page);
          const [tx, ty] = await onPage(page, between);
          await page.mouse.move(tx, ty);
          await frames(page);
          const first = await hovered(page);
          await page.keyboard.press('Tab');
          await frames(page);
          const second = await hovered(page);
          assert(first?.count >= 2 && second?.place === 1 && second.count === first.count, `Tab steps to the next: ${JSON.stringify({ first, second })}`);
          seen.tab = { count: first.count, first: first.snap, second: second.snap };
          // From a focused button, Tab moves the focus on, as on any page.
          await page.focus('.status [data-toggle="grid"]');
          await page.keyboard.press('Tab');
          await frames(page);
          const kept = await hovered(page);
          const moved = await page.evaluate(() => document.activeElement?.getAttribute('data-toggle'));
          assert(kept?.place === 1 && moved !== 'grid', `Tab from a button leaves the snaps alone: ${JSON.stringify({ kept, moved })}`);
          await page.evaluate(() => document.activeElement?.blur());
        }
        // A click made before the next frame takes the snap under it, not
        // the one the cursor caught before: the move and the click in one
        // task, so no frame comes between.
        {
          const other = listed
            .filter((s) => s.snap === 'endpoint' && s.object !== h.object)
            .map((s) => [s, Math.hypot(s.at[0] - end[0], s.at[1] - end[1])])
            .filter(([, d]) => d > 0.05)
            .sort((a, c) => a[1] - c[1])[0]?.[0];
          assert(other, 'another endpoint lies near');
          await centre(page, end, 6);
          await page.mouse.move(x + 5, y + 4);
          await frames(page);
          assert((await hovered(page))?.which === 'objects[3].from', 'the cursor holds FT-101');
          const [ox, oy] = await onPage(page, other.at);
          await page.evaluate(([ox, oy]) => {
            const stage = document.querySelector('.stage');
            const at = { clientX: ox, clientY: oy, pointerId: 1, pointerType: 'mouse', button: 0, bubbles: true };
            stage.dispatchEvent(new PointerEvent('pointermove', at));
            stage.dispatchEvent(new PointerEvent('pointerdown', at));
            stage.dispatchEvent(new PointerEvent('pointerup', at));
          }, [ox, oy]);
          await frames(page);
          const fast = await page.textContent('.cmdbar');
          const said = `${other.at[0].toFixed(4)}, ${other.at[1].toFixed(4)} in`;
          assert(fast.includes(said) && !fast.includes('Endpoint of FT-101'), `a fast click takes the snap under it: ${fast.slice(0, 120)} (wanted ${said})`);
          await page.click('.opt[data-opt="clear"]');
        }
        // The tag beside a snap at the drawing's foot flips above it, in sight.
        {
          await page.evaluate(
            ([ex, ey]) => {
              const p = window.agentdrafterViewer;
              const r = document.querySelector('.stage').getBoundingClientRect();
              const scale = p.view().scale;
              p.show({ left: ex - r.width / 2 / scale, top: ey + (r.height - 10) / scale, scale });
            },
            [end[0], end[1]],
          );
          await frames(page);
          const [bx, by] = await onPage(page, end);
          await page.mouse.move(bx + 2, by - 1);
          await frames(page);
          const place = await page.evaluate(() => {
            const t = document.querySelector('.snaptag').getBoundingClientRect();
            const s = document.querySelector('.stage').getBoundingClientRect();
            return { shown: getComputedStyle(document.querySelector('.snaptag')).display, top: t.top, bottom: t.bottom, stage: s.bottom };
          });
          assert(place.shown === 'block' && place.bottom <= place.stage && place.bottom < by, `the tag sits above the snap, on the drawing: ${JSON.stringify({ place, by })}`);
          await centre(page, end, 6);
          await frames(page);
        }
        // The menu: Endpoint unticked, and the cursor no longer catches it; kept across a reload.
        await page.mouse.move(x + 5, y + 4);
        await page.click('.osmenubtn');
        await page.waitForSelector('.osmenu');
        await page.screenshot({ path: join(out, 'snaps-laptop-menu.png') });
        await page.click('.osrow[data-snap="endpoint"]');
        assert((await page.getAttribute('.osrow[data-snap="endpoint"]', 'aria-checked')) === 'false', 'Endpoint is unticked');
        await page.keyboard.press('Escape');
        await page.mouse.move(x + 6, y + 4);
        await frames(page);
        const without = await hovered(page);
        assert(!without || without.snap !== 'endpoint', `with Endpoint off, it is not caught: ${JSON.stringify(without)}`);
        await page.reload();
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.snaps > 0);
        await page.click('.osmenubtn');
        const kept = await page.evaluate(() => [...document.querySelectorAll('.osrow[aria-checked="true"]')].map((r) => r.dataset.snap));
        assert(JSON.stringify(kept) === JSON.stringify(['midpoint', 'center', 'intersection']), `the running snaps are kept: ${JSON.stringify(kept)}`);
        await page.click('.osmenu [data-defaults]');
        await page.keyboard.press('Escape');
        // F3 turns snapping off, and on.
        await centre(page, end, 6);
        await frames(page);
        const [fx, fy] = await onPage(page, end);
        await page.mouse.move(fx + 5, fy + 4);
        await frames(page);
        assert((await hovered(page))?.snap === 'endpoint', 'Defaults puts Endpoint back');
        await page.keyboard.press('F3');
        await page.mouse.move(fx + 5, fy + 5);
        await frames(page);
        assert((await hovered(page)) === null && (await page.getAttribute('.status [data-toggle="osnap"]', 'aria-pressed')) === 'false', 'F3 turns snapping off');
        await page.keyboard.press('F3');
        // With Object snap off the sheet comes without its snaps, and
        // turning it on asks for them.
        await page.evaluate(() => {
          const kept = JSON.parse(localStorage.getItem('agentdrafter-object-snaps') ?? '{}');
          localStorage.setItem('agentdrafter-object-snaps', JSON.stringify({ ...kept, on: false }));
        });
        const asked = [];
        const ask = (request) => {
          if (request.url().includes('/display-list')) asked.push(JSON.parse(request.postData() ?? '{}'));
        };
        page.on('request', ask);
        await page.reload();
        await page.waitForFunction(() => window.agentdrafterViewer?.summary);
        assert((await page.evaluate(() => window.agentdrafterViewer.summary.snaps)) === 0, 'with Object snap off, no snaps come');
        const before = asked.length;
        await page.keyboard.press('F3');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.snaps > 0, null, { timeout: 15000 });
        page.off('request', ask);
        assert(
          before > 0 && asked.slice(0, before).every((input) => !input.snaps) && asked.slice(before).some((input) => input.snaps === true),
          `the snaps are asked for only once Object snap is on: ${JSON.stringify(asked)}`,
        );
        // The search for the snaps in reach, per pointer move, over the paper.
        const each = await page.evaluate(() => {
          const p = window.agentdrafterViewer;
          const [l, bt, r, t] = p.summary.paper;
          const at = Array.from({ length: 5000 }, (_, i) => [l + (((i * 7919) % 1000) / 1000) * (r - l), bt + (((i * 104729) % 1000) / 1000) * (t - bt)]);
          return p.timeSnapSearch(at, 12);
        });
        assert(each < 0.5, `the search takes ${each} ms a move`);
        // An intersection taken: deleting the other object it names takes
        // the read-out and its Copy away, though its own object is still
        // there. One clear of every endpoint, so it is the snap caught.
        {
          const objects = agentIn(folder, 'display-list', 'demo-plant', '--sheet', 'DP-LD-101').result.objects;
          const device = objects.findIndex((o) => o.id === 'DP-LD-101/device');
          const crossing = listed.find(
            (s) =>
              s.snap === 'intersection' &&
              objects[s.object]?.id &&
              objects[s.with]?.id &&
              s.object !== device &&
              s.with !== device &&
              listed.every((t) => t.snap === 'intersection' || Math.hypot(t.at[0] - s.at[0], t.at[1] - s.at[1]) > 0.1),
          );
          assert(crossing, 'an intersection clear of every other snap');
          await centre(page, crossing.at, 6);
          await frames(page);
          const [cx, cy] = await onPage(page, crossing.at);
          await page.mouse.move(cx + 2, cy + 1);
          await frames(page);
          const caught = await hovered(page);
          assert(caught?.snap === 'intersection', `the intersection is caught: ${JSON.stringify(caught)}`);
          await page.mouse.click(cx + 2, cy + 1);
          await page.waitForSelector('.opt[data-opt="copy"]');
          const other = objects[caught.with].id;
          assert(sendChange(folder, { operations: [{ op: 'delete', target: { id: other } }] }).ok, `${other} is deleted`);
          await page.waitForFunction((gone) => !window.agentdrafterViewer.summary.objects.some((o) => o.id === gone), other, { timeout: 15000 });
          await frames(page);
          assert(!(await page.$('.opt[data-opt="copy"]')), `Copy goes with ${other}, the intersection's other object`);
        }
        seen.laptop = { snaps: count, endpoint: end, copied, search_ms_per_move: each };
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      // A phone: a finger held brings up the magnifier; lifting takes the snap.
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await centre(page, end, 4);
        await frames(page);
        const [x, y] = await onPage(page, end);
        const cdp = await context.newCDPSession(page);
        const touch = (type, points) =>
          cdp.send('Input.dispatchTouchEvent', { type, touchPoints: points.map(([px, py], id) => ({ x: px, y: py, id, radiusX: 2, radiusY: 2, force: 1 })) });
        // Where FT-101's endpoint is on the screen once the view is still:
        // a snap taken selects its object, and the page glides to bring it
        // clear of the card, so each hold finds the point again.
        const still = async () => {
          const until = Date.now() + 5000;
          for (let calm = 0, last = ''; calm < 3; ) {
            assert(Date.now() < until, 'the view comes to rest');
            await sleep(100);
            const now = JSON.stringify(await page.evaluate(() => window.agentdrafterViewer.view()));
            calm = now === last ? calm + 1 : 0;
            last = now;
          }
          return onPage(page, end);
        };
        await touch('touchStart', [[x + 4, y + 3]]);
        await sleep(500);
        const loupe = await page.evaluate(() => {
          const el = document.querySelector('.loupe');
          const r = el.getBoundingClientRect();
          return { display: getComputedStyle(el).display, box: [r.left, r.top, r.right, r.bottom] };
        });
        const tag = await page.textContent('.snaptag');
        await page.screenshot({ path: join(out, 'snaps-phone-magnifier.png') });
        assert(loupe.display === 'block' && loupe.box[3] < y, `the magnifier comes up above the finger: ${JSON.stringify({ loupe, y })}`);
        assert(tag === 'Endpoint · FT-101', `it shows the snap the finger is on: ${tag}`);
        await touch('touchEnd', []);
        await page.waitForFunction(() => document.querySelector('.cmdbar').textContent.includes('Endpoint of FT-101'));
        assert((await page.evaluate(() => getComputedStyle(document.querySelector('.loupe')).display)) === 'none', 'lifting puts the magnifier away');
        await page.screenshot({ path: join(out, 'snaps-phone-taken.png') });
        // A change lands while the finger is held: the snap lifted is found
        // on the sheet as it is now, its object and its point agreeing,
        // though an object before FT-101's is gone and every place after it
        // has moved down one.
        {
          const objects = agentIn(folder, 'display-list', 'demo-plant', '--sheet', 'DP-LD-101').result.objects;
          const earlier = objects.slice(0, objects.findIndex((o) => o.id === 'DP-LD-101/device')).find((o) => o.id);
          assert(earlier, 'an object comes before FT-101');
          await page.tap('.opt[data-opt="clear"]');
          const [hx, hy] = await still();
          await touch('touchStart', [[hx + 4, hy + 3]]);
          await sleep(500);
          assert(sendChange(folder, { operations: [{ op: 'delete', target: { id: earlier.id } }] }).ok, `${earlier.id} is deleted`);
          await page.waitForFunction((gone) => !window.agentdrafterViewer.summary.objects.some((o) => o.id === gone), earlier.id, { timeout: 15000 });
          await touch('touchEnd', []);
          await page.waitForFunction(() => document.querySelector('.cmdbar').textContent.includes('Endpoint of'));
          const bar = await page.textContent('.cmdbar');
          assert(bar.includes(`Endpoint of FT-101: ${end[0].toFixed(4)}, ${end[1].toFixed(4)} in`), `the snap lifted is FT-101's, where it is: ${bar.slice(0, 120)}`);
        }
        // Object snap turned off from a keyboard while the finger is held:
        // the lift takes no snap.
        {
          await page.tap('.opt[data-opt="clear"]');
          const [hx, hy] = await still();
          await touch('touchStart', [[hx + 4, hy + 3]]);
          await sleep(500);
          // Held on FT-101's endpoint, before Object snap goes off.
          assert((await page.textContent('.snaptag')) === 'Endpoint · FT-101', 'the finger is held on FT-101');
          await page.keyboard.press('F3');
          await sleep(100);
          await touch('touchEnd', []);
          await sleep(300);
          const bar = await page.textContent('.cmdbar');
          assert(!bar.includes('Endpoint of'), `with Object snap off, the lift takes no snap: ${bar.slice(0, 120)}`);
          await page.keyboard.press('F3');
        }
        // The menu is in Tools, under Status.
        await page.tap('.toolsbtn');
        await page.tap('[data-snaps]');
        await page.waitForSelector('.osmenu.card');
        await page.screenshot({ path: join(out, 'snaps-phone-menu.png') });
        seen.phone = { magnifier: loupe.box, tag };
        assert(problems.length === 0, problems.join('; '));
        await context.close();
      }
      return { ...seen, screenshots: ['snaps-laptop-endpoint.png', 'snaps-laptop-taken.png', 'snaps-laptop-menu.png', 'snaps-phone-magnifier.png', 'snaps-phone-taken.png', 'snaps-phone-menu.png'] };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  /** Screenshots at phone and laptop sizes. */
  async proposals() {
    const b = await browser();
    const SEVEN = ['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302'];
    const frames = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    // An agent's command through the HTTP API, as the page's own are not.
    const http = async (base, command, input) => {
      const r = await fetch(`${base}/commands/${command}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(input) });
      return r.json();
    };
    // From the command line, another process, with no `by`.
    const proposeIn = (folder, changeSet) => {
      const file = join(folder, 'proposal.json');
      writeFileSync(file, JSON.stringify(changeSet));
      return agentIn(folder, 'propose', 'demo-plant', file);
    };
    const journalOf = (folder) => agentIn(folder, 'journal', 'demo-plant').result?.entries ?? [];
    const waiting = (folder) => (agentIn(folder, 'proposals', 'demo-plant').result?.proposals ?? []).map((p) => p.proposal);
    const grows = async (folder, from, what) => {
      const until = Date.now() + 30000;
      while (!grownAndDone(journalOf(folder), from)) {
        assert(Date.now() < until, `${what}: the journal did not grow`);
        await sleep(100);
      }
      return journalOf(folder).at(-1);
    };
    const cardText = (page, n) => page.locator(`[data-pcard="${n}"]`).textContent();
    const says = (page, n, words) =>
      page.waitForFunction(([k, w]) => document.querySelector(`[data-pcard="${k}"]`)?.textContent.includes(w), [n, words], { timeout: 30000 });
    const band = (page) => page.textContent('[data-preview]');
    const bar = (page) => page.evaluate(() => document.querySelector('.cmdbar').textContent);
    const counts = (page) =>
      page.evaluate(() => Object.fromEntries([...document.querySelectorAll('.sheettabs .tab')].map((t) => [t.querySelector('b').textContent, t.querySelector('.pcount')?.textContent ?? ''])));
    const journalLine = (page, entry) => page.textContent(`.va-journal [data-entry="${entry}"]`);
    // The service text moved into the zone line, which fails text_on_line on three sheets; CHECKED set on the title block; the junction box moved.
    const moveService = { operations: [{ op: 'move', target: { id: 'DP-LD-101/service' }, by: [2.2, 0, 0] }] };
    const checked = { operations: [{ op: 'set_attribute', target: { id: 'DP-LD-101/title-block' }, tag: 'CHECKED', value: 'JK' }] };
    const moveBox = { operations: [{ op: 'move', target: { id: 'DP-LD-101/junction-box' }, by: [0.3, 0, 0] }] };
    const lock = { operations: [{ op: 'change_layer', sheet: 'DP-LD-101', name: 'enclosure', set: { locked: true } }] };
    const shots = [];
    try {
      {
        const folder = freshPlant('proposals-laptop');
        const set = join(folder, 'demo-plant');
        const server = await serve('127.0.0.1:0', folder);
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        await journalTab(page);
        const files = filesIn(set);

        // An agent proposes through the HTTP API while the page is open: a
        // toast names it, its card comes at the change, joined by a leader,
        // it is drawn dashed, and each sheet it reaches counts it.
        const p1 = await http(server.base, 'propose', { set: 'demo-plant', change_set: moveService, by: 'Assistant' });
        assert(p1.ok && p1.result.proposal === 1, `proposed: ${JSON.stringify(p1.error)}`);
        await page.waitForSelector('[data-pcard="1"]', { timeout: 30000 });
        await says(page, 1, 'would fail a check');
        let said = await cardText(page, 1);
        assert(said.includes('Assistant') && said.includes('Move service') && said.includes('7 sheets'), `the card: ${said}`);
        assert(said.includes('All 7 would take it, but DP-LD-202, DP-LD-301 and DP-LD-302 would fail a check.'), `the card says how it stands: ${said}`);
        const toast = await page.textContent('.va-toast');
        assert(toast.includes('Assistant proposes: Move service') && toast.includes('Show'), `a toast names it: ${toast}`);
        await frames(page);
        assert((await page.locator('[data-pleaders] line').count()) === 1, 'a leader joins the card to its change');
        const ghost = await page.evaluate(() => window.agentdrafterViewer.ghostExtent(1));
        assert(ghost.length === 4 && ghost[0] > 2.5 && ghost[0] < 3.5, `drawn dashed where the service text would go: ${JSON.stringify(ghost)}`);
        const tabs = await counts(page);
        assert(
          JSON.stringify(tabs) === JSON.stringify({ 'DP-LD-101': '1', 'DP-LD-102': '1', 'DP-LD-103': '', 'DP-LD-104': '1', 'DP-LD-201': '1', 'DP-LD-202': '1', 'DP-LD-301': '1', 'DP-LD-302': '1' }),
          `each tab counts the proposals reaching it: ${JSON.stringify(tabs)}`,
        );
        const proposed = filesIn(set);
        const added = Object.keys(proposed).filter((f) => !(f in files));
        assert(
          added.every((f) => f.replaceAll('\\', '/').includes('.agentdrafter/proposals/')) && Object.keys(files).every((f) => proposed[f] === files[f]),
          `nothing is written but the proposal: ${JSON.stringify(added)}`,
        );
        assert(journalOf(folder).length === 0, 'nothing goes in the journal');

        // A second from the command line, and a third through the HTTP API, each with no `by`: its interface.
        const p2 = proposeIn(folder, checked);
        assert(p2.ok && p2.result.by === 'agent (command line)', `the second: ${JSON.stringify(p2)}`);
        const p3 = await http(server.base, 'propose', { set: 'demo-plant', change_set: moveBox });
        assert(p3.ok && p3.result.by === 'agent (HTTP API)', `the third: ${JSON.stringify(p3)}`);
        await says(page, 2, 'All 7 would take it.');
        await says(page, 3, 'All 7 would take it.');
        assert((await counts(page))['DP-LD-101'] === '3', 'three wait on DP-LD-101');
        await frames(page);
        await page.screenshot({ path: join(out, 'proposals-laptop-cards.png') });
        shots.push('proposals-laptop-cards.png');

        // The set changes under them: the enclosure layer locked. The second
        // was made before it; the third is refused now, in the core's words.
        const locked = sendChange(folder, lock);
        assert(locked.ok, `locked: ${JSON.stringify(locked.error)}`);
        await says(page, 3, 'Refused now');
        said = await cardText(page, 3);
        assert(said.includes('locked') && said.includes('Can’t keep'), `the third is refused now: ${said}`);
        assert(await page.locator('[data-pcard="3"] [data-pc="keep"]').isDisabled(), 'Keep is off');
        await says(page, 2, 'Set changed since: change set 1 landed after it.');

        // Opened: the band's PROPOSAL form, over the drawing as it would leave the sheet.
        await page.click('[data-pcard="2"] [data-pc="show"]');
        await page.waitForSelector('[data-proposal="2"] [data-pv-summary]', { timeout: 30000 });
        said = await band(page);
        assert(said.includes('Proposal 2') && said.includes('Set CHECKED = JK on title-block') && said.includes('on 7 sheets'), `the band names it: ${said}`);
        assert(said.includes('Proposed by agent (command line) at') && said.includes('Nothing lands until you keep it.'), `who and when: ${said}`);
        assert(said.includes('The set has changed since it was proposed: change set 1 landed after it.'), `what landed since: ${said}`);
        assert(said.includes('Keep') && said.includes('Discard'), `Keep and Discard: ${said}`);
        assert((await bar(page)).startsWith('>_PROPOSALNothing has landed yet.') && (await bar(page)).includes('Esc closes it, and it keeps waiting'), `the command bar: ${await bar(page)}`);
        assert((await page.textContent('[data-pvhead]')).includes('as proposal 2 would leave it'), 'the drawing says what it shows');
        assert((await page.locator('[data-pcard]').count()) === 0, 'no card over the band');
        await page.screenshot({ path: join(out, 'proposals-laptop-opened.png') });
        shots.push('proposals-laptop-opened.png');
        // Closed by ×, and by Esc: it keeps waiting.
        await page.click('[data-pv="close"]');
        await page.waitForSelector('[data-pcard="2"]', { timeout: 30000 });
        assert((await page.locator('[data-preview]').count()) === 0, '× closes it');
        await page.click('[data-pcard="2"] [data-pc="show"]');
        await page.waitForSelector('[data-proposal="2"]');
        await page.keyboard.press('Escape');
        await page.waitForSelector('[data-pcard="2"]', { timeout: 30000 });
        assert(JSON.stringify(waiting(folder)) === '[1,2,3]', `closing ends nothing: ${waiting(folder)}`);
        assert(journalOf(folder).length === 1, 'closing writes nothing');

        // A refused one opened: Keep reads Nothing can land; Discard still works, and the journal says so (J1).
        await page.click('[data-pcard="3"] [data-pc="show"]');
        await page.waitForSelector('[data-proposal="3"] [data-pv-summary]', { timeout: 30000 });
        const keep3 = page.locator('[data-pv="keep"]');
        assert((await keep3.textContent()) === 'Nothing can land' && (await keep3.isDisabled()), `Keep: ${await keep3.textContent()}`);
        await page.click('[data-pv="discard"]');
        const discarded = await grows(folder, 1, 'Discard');
        assert(discarded.kind === 'discard' && discarded.proposal === 3 && discarded.discarded_by === 'drafter' && discarded.proposed_by === 'agent (HTTP API)', `the discard: ${JSON.stringify(discarded)}`);
        await page.waitForFunction(() => !document.querySelector('[data-pcard="3"]'), null, { timeout: 30000 });
        await page.waitForSelector(`.va-journal [data-entry="${discarded.entry}"]`);
        said = await journalLine(page, discarded.entry);
        assert(said.includes('Discarded proposal 3') && said.includes('by you · proposed by agent (HTTP API)') && said.includes('Discarded'), `the journal's line: ${said}`);

        // Kept from its card, though it fails a check: one entry, which says who proposed it and who kept it.
        await page.click('[data-pcard="1"] [data-pc="keep"]');
        const kept = await grows(folder, 2, 'Keep');
        assert(kept.kind === 'change_set' && kept.proposal === 1 && kept.proposed_by === 'Assistant' && kept.kept_by === 'drafter', `kept: ${JSON.stringify(kept)}`);
        assert(JSON.stringify(kept.landed_on) === JSON.stringify(SEVEN), `it lands on all seven: ${JSON.stringify(kept.landed_on)}`);
        await page.waitForSelector(`.va-journal [data-entry="${kept.entry}"]`);
        said = await journalLine(page, kept.entry);
        assert(said.includes('Move service') && said.includes('proposed by Assistant · kept by you'), `J1: ${said}`);
        await page.waitForFunction(() => !document.querySelector('[data-pcard="1"]'), null, { timeout: 30000 });

        // A keep the set refuses: DP-LD-202's DWG made read-only, as one
        // open in AutoCAD, after the card said it would land. Nothing lands,
        // the journal labels it an attempt, and the card asks again and says
        // it is refused now.
        const dwg202 = join(set, 'dwg', 'DP-LD-202.dwg');
        // Its card, asked for again on the set as it now is, offers Keep.
        await page.waitForFunction(() => !document.querySelector('[data-pcard="2"] [data-pc="keep"]')?.disabled, null, { timeout: 30000 });
        chmodSync(dwg202, 0o444);
        // The title block's card can sit under the laptop's command bar
        // (#212): clicked on the button itself.
        await page.locator('[data-pcard="2"] [data-pc="keep"]').dispatchEvent('click');
        const attempt = await grows(folder, 3, 'A refused keep');
        assert(attempt.kind === 'failed_attempt' && attempt.proposal === 2 && attempt.kept_by === 'drafter', `the attempt: ${JSON.stringify(attempt)}`);
        assert(!attempt.landed_on, `nothing landed: ${JSON.stringify(attempt.landed_on)}`);
        await says(page, 2, 'Refused now');
        assert(await page.locator('[data-pcard="2"] [data-pc="keep"]').isDisabled(), 'Keep is off once it is refused');
        await page.waitForSelector(`.va-journal [data-entry="${attempt.entry}"]`);
        said = await journalLine(page, attempt.entry);
        assert(said.includes('keep attempted by you · refused') && !said.includes('kept by'), `the attempt's line: ${said}`);
        chmodSync(dwg202, 0o644);

        // The one made before the set changed, kept from its band: applied to the set as it stands.
        await page.click('[data-pcard="2"] [data-pc="show"]');
        await page.waitForSelector('[data-proposal="2"] [data-pv-summary]', { timeout: 30000 });
        await page.waitForFunction(() => !document.querySelector('[data-pv="keep"]')?.disabled, null, { timeout: 30000 });
        await page.click('[data-pv="keep"]');
        const second = await grows(folder, 4, 'Keep from the band');
        assert(second.kind === 'change_set' && second.proposal === 2 && second.kept_by === 'drafter', `kept from the band: ${JSON.stringify(second)}`);
        await page.waitForSelector(`.va-journal [data-entry="${second.entry}"]`);
        await page.waitForFunction(() => document.querySelectorAll('[data-pcard]').length === 0, null, { timeout: 30000 });
        assert(Object.values(await counts(page)).every((c) => c === ''), 'no tab counts one');
        await page.waitForFunction(() => !document.body.textContent.includes('Drawing DP-LD'), null, { timeout: 30000 });
        await sleep(400);
        await page.screenshot({ path: join(out, 'proposals-laptop-journal.png') });
        shots.push('proposals-laptop-journal.png');

        // Undone as any change set, as one, and the proposal does not come back.
        await page.click('[data-act="undo"]');
        const undone = await grows(folder, 5, 'Undo last');
        assert(undone.kind === 'undo' && undone.state === 'done' && JSON.stringify(undone.undid) === JSON.stringify([second.entry]), `undone: ${JSON.stringify(undone)}`);
        await sleep(500);
        assert(waiting(folder).length === 0 && (await page.locator('[data-pcard]').count()) === 0, 'the proposal does not come back');

        // Show from a toast while the set is shown as it was: back to live,
        // and the band draws it.
        const ring = sendChange(folder, {
          operations: [{ op: 'add_circle', sheet: 'DP-LD-101', center: [2, 8, 0], radius: 0.2, id: 'spare-ring', name: 'spare-ring' }],
        });
        assert(ring.ok, `the ring: ${JSON.stringify(ring.error)}`);
        await page.waitForSelector(`.va-journal [data-entry="${ring.result.journal_entry}"]`);
        await page.click(`.va-journal [data-entry="${ring.result.journal_entry}"] .je-head`);
        await page.waitForSelector('.va-vp.past', { timeout: 30000 });
        const p4 = await http(server.base, 'propose', {
          set: 'demo-plant',
          change_set: { operations: [{ op: 'move', target: { name: 'spare-ring' }, by: [0.2, 0, 0] }] },
          by: 'Assistant',
        });
        assert(p4.ok && p4.result.proposal === 4, `the fourth: ${JSON.stringify(p4.error)}`);
        await page.waitForSelector('[data-toast="show-proposal"]', { timeout: 30000 });
        await page.click('[data-toast="show-proposal"]');
        await page.waitForSelector('[data-proposal="4"] [data-pv-summary]', { timeout: 30000 });
        assert((await page.locator('.va-vp.past').count()) === 0, 'back to live');
        await page.click('[data-pv="close"]');
        await page.waitForSelector('[data-pcard="4"]', { timeout: 30000 });

        // A proposal the core now refuses whole, the circle it names
        // deleted: its card and its band say why, Keep is off, Discard ends it.
        const gone = sendChange(folder, { operations: [{ op: 'delete', target: { id: 'DP-LD-101/spare-ring' } }] });
        assert(gone.ok, `deleted: ${JSON.stringify(gone.error)}`);
        await says(page, 4, 'Refused now');
        said = await cardText(page, 4);
        assert(said.includes('no object on any sheet of the set has the name spare-ring'), `the card says why, in the core's words: ${said}`);
        assert(await page.locator('[data-pcard="4"] [data-pc="keep"]').isDisabled(), 'its Keep is off');
        await page.click('[data-pcard="4"] [data-pc="show"]');
        await page.waitForSelector('[data-proposal="4"] [data-pv-refused]', { timeout: 30000 });
        said = await band(page);
        assert(said.includes('Refused now.') && said.includes('spare-ring') && said.includes('Discard'), `the band stays open and says why: ${said}`);
        const keep4 = page.locator('[data-pv="keep"]');
        assert((await keep4.textContent()) === 'Nothing can land' && (await keep4.isDisabled()), `Keep: ${await keep4.textContent()}`);
        await page.waitForFunction(() => !document.body.textContent.includes('Drawing DP-LD'), null, { timeout: 30000 });
        await sleep(300);
        await page.screenshot({ path: join(out, 'proposals-laptop-refused-band.png') });
        shots.push('proposals-laptop-refused-band.png');
        const entries = journalOf(folder).length;
        await page.click('[data-pv="discard"]');
        const ended = await grows(folder, entries, 'Discard of a refused one');
        assert(ended.kind === 'discard' && ended.proposal === 4, `discarded: ${JSON.stringify(ended)}`);
        await page.waitForFunction(() => !document.querySelector('[data-preview]') && !document.querySelector('[data-pcard]'), null, { timeout: 30000 });
        // The refusals this check makes are the 422s the browser logs: the
        // refused keep, and the fourth's drawing asked for by its card and its band.
        assert(problems.filter((p) => !p.includes('422')).length === 0, problems.join('; '));
        await context.close();
        await server.stop();
      }
      {
        // A phone: each waiting proposal a chip, which a tap opens into its card.
        const folder = freshPlant('proposals-phone');
        const server = await serve('127.0.0.1:0', folder);
        const context = await b.newContext({ ...PHONE, viewport: { width: 375, height: 812 } });
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        for (const [changeSet, by] of [
          [moveService, 'Assistant'],
          [checked, 'agent (MCP)'],
        ]) {
          const p = await http(server.base, 'propose', { set: 'demo-plant', change_set: changeSet, by });
          assert(p.ok, `proposed: ${JSON.stringify(p.error)}`);
        }
        await page.waitForFunction(() => document.querySelectorAll('[data-pchip]').length === 2 && [...document.querySelectorAll('[data-pchip] .vd')].length === 2, null, {
          timeout: 30000,
        });
        await page.waitForSelector('[data-act="proposals"]');
        assert((await page.textContent('[data-act="proposals"]')).includes('2 proposals on this sheet, 2 in all'), 'the peek counts them');
        await sleep(300);
        await page.screenshot({ path: join(out, 'proposals-phone-chips.png') });
        shots.push('proposals-phone-chips.png');
        await page.tap('[data-pchip][data-pcard="1"]');
        await page.waitForSelector('.pcard.compact[data-pcard="1"]');
        const said = await cardText(page, 1);
        assert(said.includes('Move service') && said.includes('Fails a check') && said.includes('Keep') && said.includes('Discard'), `the card: ${said}`);
        await frames(page);
        await page.screenshot({ path: join(out, 'proposals-phone-card.png') });
        shots.push('proposals-phone-card.png');
        await page.tap('[data-pcard="1"] [data-pc="show"]');
        await page.waitForSelector('[data-proposal="1"] [data-pv-summary]', { timeout: 30000 });
        assert((await band(page)).includes('Proposed by Assistant'), 'the band says who');
        await sleep(400);
        await page.screenshot({ path: join(out, 'proposals-phone-opened.png') });
        shots.push('proposals-phone-opened.png');
        await page.tap('[data-pv="keep"]');
        const kept = await grows(folder, 0, 'Keep on a phone');
        assert(kept.proposal === 1 && kept.kept_by === 'drafter', `kept: ${JSON.stringify(kept)}`);
        // The other, opened from the peek, discarded from its band.
        await page.waitForFunction(() => document.querySelector('[data-act="proposals"]')?.textContent.includes('1 proposal on this sheet'), null, { timeout: 30000 });
        await page.tap('[data-act="proposals"]');
        await page.waitForSelector('[data-proposal="2"] [data-pv-summary]', { timeout: 30000 });
        await page.tap('[data-pv="discard"]');
        const gone = await grows(folder, 1, 'Discard on a phone');
        assert(gone.kind === 'discard' && gone.proposal === 2, `discarded: ${JSON.stringify(gone)}`);
        await page.waitForFunction(() => !document.querySelector('[data-act="proposals"]') && !document.querySelector('[data-pcard]'), null, { timeout: 30000 });
        // The journal, raised: the kept proposal's entry and the discard's line.
        const history = () => page.evaluate(() => document.querySelector('.cmd')?.textContent ?? '');
        assert(!(await page.textContent('body')).includes('Showing entry'), `still live before the journal is raised: ${await history()}`);
        // Dragged up, as a finger raises it.
        // The grip is found once the sheet is at rest: the peek just lost
        // its proposals' line, and the sheet follows its height. A finger
        // there may be steered to the peek's Ask button, which drags it too.
        // A drag that did not take on a slow runner is made once more.
        const cdp = await context.newCDPSession(page);
        for (let attempt = 1; ; attempt += 1) {
          assert(await sheetAtRest(page, false), 'the sheet comes to rest at its peek');
          const [gx, gy] = await page.evaluate(() => {
            const r = document.querySelector('.va-grab').getBoundingClientRect();
            return [r.left + r.width / 2, r.top + r.height / 2];
          });
          await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: gx, y: gy }] });
          for (let i = 1; i <= 10; i += 1) {
            await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: gx, y: gy - i * 30 }] });
            await sleep(16);
          }
          await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
          // Raised and at rest, so the tab is tapped where it stays.
          if (await sheetAtRest(page, true, attempt >= 2 ? 30000 : 5000)) break;
          assert(attempt < 2, 'dragged up twice, the sheet is raised');
        }
        await sideTab(page, 'journal', 'tap');
        await page.waitForSelector(`.va-journal [data-entry="${gone.entry}"]`, { timeout: 30000 });
        assert(!(await page.textContent('body')).includes('Showing entry'), `still live with the journal raised: ${await history()}`);
        // Tapping the grab lowers and raises the sheet without its click
        // landing on the journal list sliding up under the finger (#204).
        await page.tap('.va-grab');
        assert(await sheetAtRest(page, false), 'tapping the grab lowers the sheet to its peek');
        const [tapX, tapY] = await page.evaluate(() => {
          const r = document.querySelector('.va-grab').getBoundingClientRect();
          return [r.left + r.width / 2, r.top + r.height / 2];
        });
        await page.touchscreen.tap(tapX, tapY);
        assert(await sheetAtRest(page, true), 'tapping the grab raises the sheet to half');
        await sleep(300);
        assert(
          !(await page.textContent('body')).includes('Showing entry') && (await page.evaluate(() => window.agentdrafterViewer.summary.entry === null)),
          `tapping the grab opened no journal entry: ${await history()}`,
        );
        assert((await journalLine(page, kept.entry)).includes('proposed by Assistant · kept by you'), 'J1 on a phone');
        assert((await journalLine(page, gone.entry)).includes('Discarded proposal 2'), 'the discard on a phone');
        await sleep(500);
        await page.screenshot({ path: join(out, 'proposals-phone-journal.png') });
        shots.push('proposals-phone-journal.png');
        assert(problems.length === 0, problems.join('; '));
        await context.close();
        await server.stop();
      }
      return { screenshots: shots };
    } finally {
      await b.close();
    }
  },
  async assistant() {
    assert(process.platform !== 'win32', 'the assistant check runs on Linux or macOS: its agents are shell scripts');
    // The fake agent, started by a shim named as each real agent is, first
    // on serve's PATH, so a real agent further along it is never started.
    const fake = join(root, 'crates', 'agentdrafter-cli', 'tests', 'fake-agent', 'fake_agent.py');
    assert(existsSync(fake), `no fake agent at ${fake}`);
    const shims = join(scratch, 'agents');
    mkdirSync(shims, { recursive: true });
    for (const agent of ['claude', 'codex']) {
      const shim = join(shims, agent);
      writeFileSync(shim, `#!/bin/sh\nexec python3 ${fake} ${agent} "$@"\n`);
      chmodSync(shim, 0o755);
    }
    // Each step of the fake agent waits a little, so its turn is seen working.
    const withAgents = { ...process.env, PATH: `${shims}:${process.env.PATH ?? ''}`, FAKE_AGENT_DELAY: '0.3' };
    // Neither shim, and no real agent: only the system's own folders.
    const bare = join(scratch, 'no-agents');
    mkdirSync(bare, { recursive: true });
    const noAgents = { ...process.env, PATH: `${bare}:/usr/bin:/bin`, FAKE_AGENT_DELAY: '0.3' };
    for (const dir of ['/usr/bin', '/bin']) {
      for (const agent of ['claude', 'codex']) assert(!existsSync(join(dir, agent)), `a real ${agent} in ${dir} would be found: the no-agent case cannot be shown here`);
    }
    const b = await browser();
    const shots = [];
    const shot = async (page, name) => {
      await sleep(300);
      await page.screenshot({ path: join(out, name) });
      shots.push(name);
    };
    const http = async (base, command, input) => {
      const r = await fetch(`${base}/commands/${command}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(input) });
      return r.json();
    };
    const journalOf = (folder) => agentIn(folder, 'journal', 'demo-plant').result?.entries ?? [];
    const waiting = (folder) => (agentIn(folder, 'proposals', 'demo-plant').result?.proposals ?? []).map((p) => p.proposal);
    const grows = async (folder, from, what) => {
      const until = Date.now() + 30000;
      while (!grownAndDone(journalOf(folder), from)) {
        assert(Date.now() < until, `${what}: the journal did not grow`);
        await sleep(100);
      }
      return journalOf(folder).at(-1);
    };
    // A point on the page that picks the object `id`, at the view shown.
    const pointOn = (page, id) =>
      page.evaluate((wanted) => {
        const probe = window.agentdrafterViewer;
        const place = probe.summary.objects.findIndex((o) => o.id === wanted);
        const [x0, y0, x1, y1] = probe.bounds([place]);
        const v = probe.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        for (let j = 1; j < 8; j++) {
          for (let i = 1; i < 12; i++) {
            const x = x0 + ((x1 - x0) * i) / 12;
            const y = y0 + ((y1 - y0) * j) / 8;
            if (probe.pickAt(x, y, 16) === place) return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
          }
        }
        return null;
      }, id);
    // The newest of the chat's items of a kind, and its text.
    const newest = (page, kind) => page.evaluate((k) => [...document.querySelectorAll(`[data-chat] [data-kind="${k}"]`)].at(-1)?.textContent ?? '', kind);
    const count = (page, kind) => page.evaluate((k) => document.querySelectorAll(`[data-chat] [data-kind="${k}"]`).length, kind);
    // Waits for the assistant's `n`th reply to finish.
    const replied = (page, n) =>
      page.waitForFunction(
        (k) => {
          const items = document.querySelectorAll('[data-chat] [data-kind="assistant"]');
          return items.length >= k && !items[k - 1].querySelector('[data-working]') && !document.querySelector('[data-chat] [data-working]');
        },
        n,
        { timeout: 60000 },
      );
    // The command lines of the assistant's `n`th reply: each command, its line, and whether it was refused.
    const lines = (page, n) =>
      page.evaluate(
        (k) =>
          [...([...document.querySelectorAll('[data-chat] [data-kind="assistant"]')][k - 1]?.querySelectorAll('.tl') ?? [])].map((t) => ({
            command: t.getAttribute('data-tl'),
            text: t.querySelector('.tl-h').textContent,
            refused: !!t.querySelector('.tl-c.bad'),
          })),
        n,
      );
    try {
      {
        // A laptop, with both agents found.
        const folder = freshPlant('assistant-laptop');
        const server = await serve('127.0.0.1:0', folder, withAgents);
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);

        // The panel: Assistant, the picker with both agents, Chat first, the intro and the chip.
        await page.waitForSelector('.agent select[data-act="agent"]', { timeout: 30000 });
        const options = await page.evaluate(() => [...document.querySelectorAll('.agent select option')].map((o) => o.textContent));
        assert(JSON.stringify(options) === JSON.stringify(['Claude Code', 'Codex']), `the picker offers both: ${JSON.stringify(options)}`);
        assert((await page.inputValue('.agent select')) === 'claude', 'it starts on Claude Code');
        assert((await page.textContent('.ahsub')).includes('Your own agent, started by AgentDrafter, on agentdrafter mcp'), 'the header says whose agent it is');
        assert(await page.isVisible('.stab[data-side="chat"].on'), 'Chat is shown first');
        await page.waitForSelector('[data-intro]', { timeout: 30000 });
        assert((await page.textContent('[data-intro]')).includes('What it changes waits, dashed on the drawing, until you keep it.'), 'the intro');
        await page.waitForFunction(() => document.querySelector('[data-chip]')?.textContent.includes('Sheet DP-LD-101 · nothing selected'), null, { timeout: 15000 });
        await shot(page, 'assistant-laptop-intro.png');

        // FT-101 selected: the chip says so, and the core holds it, as `looking-at` gives it through the HTTP API.
        const at = await pointOn(page, 'DP-LD-101/device');
        assert(at, 'a point picks FT-101');
        await page.mouse.click(at[0], at[1]);
        await page.waitForSelector('.qp');
        await page.waitForFunction(() => document.querySelector('[data-chip]')?.textContent.includes('Sheet DP-LD-101 · 1 selected: FT-101'), null, { timeout: 15000 });
        let held = null;
        for (const until = Date.now() + 15000; Date.now() < until; await sleep(200)) {
          held = await http(server.base, 'looking-at', { set: 'demo-plant' });
          if (held.ok && JSON.stringify(held.result.selected) === '["DP-LD-101/device"]') break;
        }
        assert(held?.ok && held.result.sheet === 'DP-LD-101' && JSON.stringify(held.result.selected) === '["DP-LD-101/device"]', `looking-at gives what the page recorded: ${JSON.stringify(held)}`);
        assert(held.result.at, 'and when');

        // A message from the chat, carrying what the drafter looks at.
        await page.fill('[data-chatin]', 'move this 1 to the right');
        await page.press('[data-chatin]', 'Enter');
        await page.waitForSelector('[data-chat] [data-kind="drafter"]', { timeout: 15000 });
        let said = await newest(page, 'drafter');
        assert(said.includes('move this 1 to the right') && said.includes('with Sheet DP-LD-101 · 1 selected: FT-101'), `the drafter's message: ${said}`);
        assert(!said.includes('from the command bar'), 'from the chat');
        assert((await page.inputValue('[data-chatin]')) === '', 'the input is emptied');
        await page.waitForSelector('[data-chat] [data-working]', { timeout: 15000 });
        assert((await page.textContent('[data-chat] [data-working]')).includes('Claude Code is working'), 'the turn is seen working');
        await page.waitForSelector('[data-pcc="1"]', { timeout: 60000 });
        await replied(page, 1);
        const note = await page.textContent('[data-chat] [data-kind="note"]');
        assert(note.includes('On Claude Code') && note.includes('agentdrafter mcp'), `the note says which agent: ${note}`);
        const first = await lines(page, 1);
        assert(
          JSON.stringify(first.map((l) => l.command)) === JSON.stringify(['looking-at', 'query', 'propose']),
          `a line for each command: ${JSON.stringify(first)}`,
        );
        assert(first[0].text.includes('Sheet DP-LD-101 · 1 selected'), `looking-at's line: ${first[0].text}`);
        assert(first[1].text.includes('find DP-LD-101/device'), `query's line: ${first[1].text}`);
        assert(first[2].text.includes('move DP-LD-101/device → proposal 1'), `propose's line: ${first[2].text}`);
        assert(first.every((l) => !l.refused), 'none refused');
        // A line opened shows its input and its reply.
        await page.click('[data-chat] .tl[data-tl="propose"] .tl-h');
        await page.waitForSelector('[data-chat] [data-tl-reply]');
        const input = await page.textContent('[data-chat] [data-tl-input]');
        const reply = await page.textContent('[data-chat] [data-tl-reply]');
        assert(input.includes('DP-LD-101/device') && reply.includes('"proposal": 1'), `the line opened: ${input} / ${reply}`);
        said = await newest(page, 'assistant');
        assert(said.includes('Proposed: move') && said.includes('proposal 1'), `its words: ${said}`);
        // Its proposal: a card in the chat, and build 7's card on the drawing.
        await page.waitForFunction(() => !document.querySelector('[data-pcc="1"] [data-pc="keep"]')?.disabled, null, { timeout: 30000 });
        said = await page.textContent('[data-pcc="1"]');
        assert(said.includes('Proposal 1, dashed on the drawing. Nothing changes until you keep it.') && said.includes('DP-LD-101 ›'), `the chat's card: ${said}`);
        // Moved an inch, FT-101 lands, and the sheet then fails a check: its card says so, Keep in amber.
        assert(said.includes('It would land') && said.includes('Keep') && said.includes('Discard'), `how it stands: ${said}`);
        await page.waitForSelector('[data-pcard="1"]', { timeout: 30000 });
        assert(waiting(folder).length === 1 && journalOf(folder).length === 0, 'it waits, and nothing landed');
        await shot(page, 'assistant-laptop-proposed.png');

        // Kept from the chat: one change set, proposed by Assistant, kept by you; the card says so.
        await page.click('[data-pcc="1"] [data-pc="keep"]');
        const kept = await grows(folder, 0, 'Keep from the chat');
        assert(kept.kind === 'change_set' && kept.proposal === 1 && kept.proposed_by === 'Assistant' && kept.kept_by === 'drafter', `kept: ${JSON.stringify(kept)}`);
        await page.waitForSelector('[data-pcc="1"][data-ended="kept"]', { timeout: 30000 });
        said = await page.textContent('[data-pcc="1"]');
        assert(said.includes('Kept') && said.includes(`Kept by you: change set ${kept.entry}, in the journal.`), `the card once kept: ${said}`);
        await page.waitForFunction(() => !document.querySelector('[data-pcard="1"]'), null, { timeout: 30000 });
        await sideTab(page, 'journal');
        await page.waitForSelector(`.va-journal [data-entry="${kept.entry}"]`, { timeout: 30000 });
        said = await page.textContent(`.va-journal [data-entry="${kept.entry}"]`);
        assert(said.includes('proposed by Assistant · kept by you'), `the journal's line: ${said}`);
        await sideTab(page, 'chat');

        // The command bar: a tool's whole name waits for Enter, so a sentence can follow; other words go to the assistant.
        await page.click('.cmdin');
        await page.keyboard.type('move');
        await page.waitForSelector('[data-askhint]');
        said = await page.textContent('[data-askhint]');
        assert(said === 'Enter: Move · keep typing to ask the assistant', `the hint for a tool's name: ${said}`);
        await page.keyboard.press('Space');
        assert((await page.inputValue('.cmdin')) === 'move ', 'Space after a whole word is a space');
        await page.keyboard.press('Escape');
        assert(!(await page.$('[data-askhint]')), 'Esc empties it, and the hint goes');
        await page.keyboard.type('m');
        assert(!(await page.$('[data-askhint]')), 'no hint for a short alias');
        await page.keyboard.press('Escape');
        await page.keyboard.type('what is this');
        await page.waitForSelector('[data-askhint]');
        assert((await page.textContent('[data-askhint]')) === 'Enter: ask the assistant', 'the hint for other words');
        await shot(page, 'assistant-laptop-command-bar.png');
        await page.keyboard.press('Enter');
        await page.waitForFunction(() => document.querySelectorAll('[data-chat] [data-kind="drafter"]').length === 2, null, { timeout: 15000 });
        said = await newest(page, 'drafter');
        assert(said.includes('what is this') && said.includes('from the command bar') && said.includes('with Sheet DP-LD-101 · 1 selected: FT-101'), `from the command bar: ${said}`);
        assert(!(await page.$('[data-askhint]')) && (await page.inputValue('.cmdin')) === '', 'the bar is emptied');
        await replied(page, 2);
        const second = await lines(page, 2);
        assert(second.some((l) => l.command === 'looking-at') && second.some((l) => l.command === 'query'), `what is this: ${JSON.stringify(second)}`);
        assert((await page.evaluate(() => [...document.querySelectorAll('[data-chat] [data-kind="assistant"]')].at(-1).querySelector('[data-words]')?.textContent ?? '')).length > 0, 'it says what it is');

        // × leaves what the drafter looks at off one message.
        await page.click('[data-act="ctxoff"]');
        said = await page.textContent('[data-chip-off]');
        assert(said.includes('This message won’t say what you’re looking at.') && said.includes('Put it back'), `the chip off: ${said}`);
        await page.fill('[data-chatin]', 'hello there');
        await page.press('[data-chatin]', 'Enter');
        await page.waitForFunction(() => document.querySelectorAll('[data-chat] [data-kind="drafter"]').length === 3, null, { timeout: 15000 });
        said = await newest(page, 'drafter');
        assert(said.includes('hello there') && said.includes('without what you’re looking at'), `without it: ${said}`);
        await page.waitForSelector('[data-chip]', { timeout: 15000 });
        await replied(page, 3);
        assert((await newest(page, 'assistant')).includes('(fake agent) You said: hello there'), 'the fake agent heard it');

        // A change it cannot make: the server withholds `change`, so nothing lands.
        const before = journalOf(folder).length;
        await page.fill('[data-chatin]', 'change it directly');
        await page.press('[data-chatin]', 'Enter');
        await replied(page, 4);
        said = await newest(page, 'assistant');
        assert(/can(no|’|')t/i.test(said), `it says it cannot: ${said}`);
        assert(journalOf(folder).length === before && waiting(folder).length === 0, 'nothing landed, and nothing was proposed');
        await shot(page, 'assistant-laptop-lines.png');

        // A guide given by the assistant: card in the chat, Start the guide,
        // advance a step, Stop from the card, Start it again, then Stop & Close (#263).
        await page.fill('[data-chatin]', 'guide me');
        await page.press('[data-chatin]', 'Enter');
        await page.waitForSelector('[data-guide-card="1"][data-gcc="offer"]', { timeout: 60000 });
        await replied(page, 5);
        const fifth = await lines(page, 5);
        assert(
          JSON.stringify(fifth.map((l) => l.command)) === JSON.stringify(['looking-at', 'guide']),
          `guide reply lines: ${JSON.stringify(fifth)}`,
        );
        assert(fifth.every((l) => !l.refused), `guide lines not refused: ${JSON.stringify(fifth)}`);
        said = await page.textContent('[data-guide-card="1"]');
        assert(
          said.includes('Draw a note line') &&
            said.includes('2 steps · DP-LD-101') &&
            said.includes('Make notes current') &&
            said.includes('Start the guide'),
          `the chat's guide card: ${said}`,
        );
        await shot(page, 'assistant-laptop-guide-card.png');
        await page.click('[data-guide-card="1"] [data-g="start"]');
        await page.waitForSelector('[data-guide="run"][data-guide-step="1"]', { timeout: 15000 });
        await page.waitForSelector('[data-guide-card="1"][data-gcc="run"]', { timeout: 15000 });
        said = await page.textContent('[data-guide-card="1"]');
        assert(said.includes('Running: step 1 of 2.') && said.includes('Stop the guide'), `card while running: ${said}`);
        // Advance step 1 via Do it for me and verify the card tracks step 2.
        await page.click('[data-g="forme"]');
        await page.waitForSelector('[data-guide="run"][data-guide-step="2"]', { timeout: 15000 });
        said = await page.textContent('[data-guide-card="1"]');
        assert(said.includes('Running: step 2 of 2.'), `card tracks step 2: ${said}`);
        // Stop right from the chat card, then Start it again from the card.
        await page.click('[data-guide-card="1"] [data-g="cardstop"]');
        await page.waitForSelector('[data-guide-card="1"][data-gcc="end"]', { timeout: 15000 });
        await page.waitForSelector('[data-guide="end"]', { timeout: 15000 });
        said = await page.textContent('[data-guide-card="1"]');
        assert(said.includes('Stopped.') && said.includes('Start it again'), `card once stopped: ${said}`);
        await page.click('[data-guide-card="1"] [data-g="start"]');
        await page.waitForSelector('[data-guide="run"][data-guide-step="1"]', { timeout: 15000 });
        await page.click('[data-g="stop"]');
        await page.waitForSelector('[data-guide="end"]', { timeout: 15000 });
        await page.click('[data-g="close"]');
        await page.waitForFunction(() => !document.querySelector('.gbanner'), null, { timeout: 15000 });

        // The picker: a new conversation on Codex.
        await page.selectOption('.agent select', 'codex');
        await page.waitForFunction(() => [...document.querySelectorAll('[data-chat] [data-kind="note"]')].some((n) => n.textContent.includes('Now on Codex')), null, { timeout: 30000 });
        assert((await page.inputValue('.agent select')) === 'codex', 'the picker shows Codex');
        assert(problems.length === 0, problems.join('; '));
        await context.close();
        await server.stop();
      }
      {
        // No agent found, at laptop and phone sizes.
        const folder = freshPlant('assistant-none');
        const server = await serve('127.0.0.1:0', folder, noAgents);
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        await page.waitForSelector('[data-noagent]', { timeout: 30000 });
        let said = await page.textContent('[data-noagent]');
        assert(said.includes('No agent found') && said.includes('Claude Code (claude) and Codex (codex), and found neither'), `the box: ${said}`);
        assert(said.includes('npm install -g @anthropic-ai/claude-code') && said.includes('npm install -g @openai/codex') && said.includes('Look again'), `the install lines: ${said}`);
        assert((await page.textContent('.agent.none')).includes('No agent found'), 'the header says so');
        assert(await page.isDisabled('[data-chatin]'), 'the input waits');
        assert((await page.getAttribute('[data-chatin]', 'placeholder')) === 'No agent yet: install one first', 'and says why');
        await page.click('[data-act="lookagain"]');
        await page.waitForFunction(() => !document.querySelector('[data-act="lookagain"]')?.disabled, null, { timeout: 15000 });
        assert(await page.isVisible('[data-noagent]'), 'still none after looking again');
        await shot(page, 'assistant-laptop-no-agent.png');
        assert(problems.length === 0, problems.join('; '));
        await context.close();
        const phone = await b.newContext(PHONE);
        const { page: p2, problems: p2problems } = await open(phone, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await p2.waitForFunction(() => document.querySelector('[data-act="ask"]')?.textContent.includes('No agent found'), null, { timeout: 30000 });
        await p2.tap('[data-act="ask"]');
        await p2.waitForSelector('.va-sheet[data-snap="half"] [data-noagent]', { timeout: 15000 });
        await shot(p2, 'assistant-phone-no-agent.png');
        assert(p2problems.length === 0, p2problems.join('; '));
        await phone.close();
        await server.stop();
      }
      {
        // A phone: the peek, the sheet raised on the chat, a message, its card, and Keep.
        const folder = freshPlant('assistant-phone');
        const server = await serve('127.0.0.1:0', folder, withAgents);
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        await page.waitForFunction(() => document.querySelector('[data-act="ask"]')?.textContent.includes('Claude Code'), null, { timeout: 30000 });
        let said = await page.textContent('[data-act="ask"]');
        assert(said.includes('Ask the assistant…'), `the peek: ${said}`);
        assert(await page.isVisible('.va-peek [data-act="undo"]') && await page.isVisible('.va-peek [data-act="accept"]'), 'Undo last and Accept stay in the peek');
        await shot(page, 'assistant-phone-peek.png');
        const at = await pointOn(page, 'DP-LD-101/device');
        assert(at, 'a point picks FT-101');
        await page.touchscreen.tap(at[0], at[1]);
        await page.waitForSelector('.qp');
        await page.tap('[data-act="ask"]');
        await page.waitForSelector('.va-sheet[data-snap="half"] [data-chat]', { timeout: 15000 });
        assert(await page.isVisible('.stab[data-side="chat"].on'), 'raised on the chat');
        await page.waitForFunction(() => document.querySelector('[data-chip]')?.textContent.includes('1 selected: FT-101'), null, { timeout: 15000 });
        await page.tap('[data-chatin]');
        await page.waitForSelector('.va-sheet[data-snap="full"]', { timeout: 15000 });
        await page.fill('[data-chatin]', 'move this 1 to the right');
        await page.press('[data-chatin]', 'Enter');
        // Once the server takes it, the input lets go, so the keyboard goes
        // down, and the sheet comes down to half: the reply and the drawing show.
        await page.waitForSelector('.va-sheet[data-snap="half"]', { timeout: 15000 });
        assert(await page.evaluate(() => !document.activeElement?.matches?.('[data-chatin]')), 'the input lets go once the message is sent');
        await shot(page, 'assistant-phone-sent-half.png');
        await page.waitForSelector('[data-pcc="1"]', { timeout: 60000 });
        await replied(page, 1);
        await page.waitForFunction(() => !document.querySelector('[data-pcc="1"] [data-pc="keep"]')?.disabled, null, { timeout: 30000 });
        await page.locator('[data-pcc="1"]').scrollIntoViewIfNeeded();
        await shot(page, 'assistant-phone-proposed.png');
        await page.tap('[data-pcc="1"] [data-pc="keep"]');
        const kept = await grows(folder, 0, 'Keep on a phone');
        assert(kept.proposal === 1 && kept.kept_by === 'drafter' && kept.proposed_by === 'Assistant', `kept: ${JSON.stringify(kept)}`);
        await page.waitForSelector('[data-pcc="1"][data-ended="kept"]', { timeout: 30000 });
        said = await page.textContent('[data-pcc="1"]');
        assert(said.includes(`Kept by you: change set ${kept.entry}, in the journal.`), `the card once kept: ${said}`);
        await shot(page, 'assistant-phone-kept.png');
        // Words from the command bar, by ⌨ Type: once the server takes
        // them, the bar's field lets go, so the keyboard goes down.
        if (await page.isVisible('.qp')) await page.tap('.qp [data-close]');
        await page.tap('.va-grab');
        await page.waitForSelector('.va-sheet[data-snap="peek"]', { timeout: 15000 });
        await page.tap('[data-type]');
        await page.waitForFunction(() => document.activeElement?.matches?.('.cmdin'), null, { timeout: 15000 });
        await page.keyboard.type('hello there');
        await page.keyboard.press('Enter');
        await replied(page, 2);
        assert(await page.evaluate(() => !document.activeElement?.matches?.('.cmdin')), 'the bar’s field lets go once its words are sent');
        assert((await newest(page, 'drafter')).includes('from the command bar'), 'sent from the command bar');
        await shot(page, 'assistant-phone-command-bar-sent.png');
        assert(problems.length === 0, problems.join('; '));
        await context.close();
        await server.stop();
      }
      return { screenshots: shots };
    } finally {
      await b.close();
    }
  },
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
  /**
   * Guides (#157), at laptop and phone sizes: a guide given through the
   * HTTP API, as an agent gives one, named by a toast with Start; the
   * banner with each step's words, the tool's button pulsing (Tools on a
   * phone), the ring on the next point with its tag; each step done by
   * hand, the setting step by the Layer box; another layer, another tool, a
   * wrong point, each with its gentle note and what takes it back; Esc with
   * nothing drawn, the step waiting; Do it for me; Skip, which skips the
   * steps that need it, and Stop; the journal marking each step's change
   * set; and the summary's one Undo of the guide's steps.
   */
  async guides() {
    const folder = freshPlant('guides');
    // DP-LD-101 starts without its + field wire, as if the drafter had not
    // drawn it yet: the guide teaches drawing it.
    const cut = sendChange(folder, { operations: [{ op: 'delete', target: { id: 'DP-LD-101/field-wire-1' } }] });
    assert(cut.ok, `the + wire deleted: ${JSON.stringify(cut.error)}`);
    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    const shots = [];
    const seen = {};
    const journal = () => agentIn(folder, 'journal', 'demo-plant').result;
    const listed = () => agentIn(folder, 'guides', 'demo-plant').result.guides;
    const frames = (page) => page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    const shot = async (page, name) => {
      await frames(page);
      await sleep(150);
      await page.screenshot({ path: join(out, name) });
      shots.push(name);
    };
    const onPage = (page, at) =>
      page.evaluate(([x, y]) => {
        const v = window.agentdrafterViewer.view();
        const r = document.querySelector('.stage').getBoundingClientRect();
        return [r.left + (x - v.left) * v.scale, r.top + (v.top - y) * v.scale];
      }, at);
    const guide = {
      title: 'Draw FT-101’s + wire to TB1-1',
      sheet: 'DP-LD-101',
      steps: [
        { say: 'Make wire the current layer. What you draw goes on the current layer, and wires go on wire.', title: 'Make wire the current layer', setting: { current_layer: 'wire' } },
        {
          say: 'Draw the wire with Line, from FT-101’s + terminal to TB1-1 in JB-100.',
          title: 'Draw the wire, FT-101’s + to TB1-1',
          tool: 'line',
          points: [
            { at: { snap: { id: 'DP-LD-101/device', snap: 'center', which: 'objects[5]' } }, label: 'FT-101’s + terminal' },
            { at: { snap: { id: 'DP-LD-101/terminal-1', snap: 'midpoint', which: 'objects[0].segments[3]' } }, label: 'TB1-1' },
          ],
          completes: {
            op: 'add_line',
            sheet: 'DP-LD-101',
            layer: 'wire',
            from: { snap: { id: 'DP-LD-101/device', snap: 'center', which: 'objects[5]' } },
            to: { snap: { id: 'DP-LD-101/terminal-1', snap: 'midpoint', which: 'objects[0].segments[3]' } },
          },
        },
        {
          say: 'Number the wire with Text: 101+, at the wire’s middle.',
          title: 'Number it 101+',
          tool: 'text',
          points: [{ at: { snap: { made_by: 2, snap: 'midpoint' } }, label: 'the wire’s middle' }],
          completes: { op: 'add_text', sheet: 'DP-LD-101', contents: '101+', height: 0.08, position: { snap: { made_by: 2, snap: 'midpoint' } } },
        },
        {
          say: 'Move the number 0.4 in left and 0.04 in up, clear of the wire and the zone line.',
          title: 'Move the number above the wire',
          tool: 'move',
          points: [
            { object: { made_by: 3 }, label: '101+' },
            { at: { snap: { made_by: 2, snap: 'midpoint' } }, label: 'base point' },
          ],
          completes: { op: 'move', target: { made_by: 3 }, by: [-0.4, 0.04, 0] },
        },
      ],
    };
    // Given through the HTTP API, as an agent does, by the assistant.
    const give = async (given = guide) => {
      const r = await fetch(`${server.base}/commands/guide`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ set: 'demo-plant', guide: given, by: 'Assistant' }),
      });
      const reply = await r.json();
      assert(r.status === 200 && reply.ok, `the guide is given: ${JSON.stringify(reply)}`);
      return reply.result.guide;
    };
    const banner = (page) => page.evaluate(() => document.querySelector('.gbanner')?.textContent ?? '');
    const stepShown = (page) => page.evaluate(() => Number(document.querySelector('[data-guide-step]')?.dataset.guideStep ?? 0));
    const untilStep = async (page, n, what) => {
      await page.waitForFunction((wanted) => Number(document.querySelector('[data-guide-step]')?.dataset.guideStep ?? 0) === wanted, n, { timeout: 30000 }).catch(async () => {
        throw new Error(`${what}: the banner is not at step ${n}: ${await banner(page)}`);
      });
    };
    const note = async (page, wanted, what) => {
      await page.waitForFunction((w) => (document.querySelector('[data-gnote]')?.textContent ?? '').includes(w), wanted, { timeout: 15000 }).catch(async () => {
        throw new Error(`${what}: no note saying ${JSON.stringify(wanted)}: ${await banner(page)}`);
      });
    };
    const noNote = (page) => page.waitForFunction(() => !document.querySelector('[data-gnote="warn"]'), null, { timeout: 15000 });
    const pulse = (page) => page.evaluate(() => document.querySelector('.va')?.dataset.gpulse ?? null);
    // The ring's middle on the page, once it is there, and its tag.
    const ring = async (page, kind = 'point') => {
      await page.waitForSelector(`[data-gring="${kind}"]`, { timeout: 15000 });
      await frames(page);
      return page.evaluate(() => {
        const r = document.querySelector('[data-gring]').getBoundingClientRect();
        return { at: [r.left + r.width / 2, r.top + r.height / 2], tag: document.querySelector('[data-gtag]')?.textContent ?? '' };
      });
    };
    // The point a step's ring marks, as the core works it out on the set as it stands.
    const ringAt = (guideNo, step, point) => {
      const at = listed().find((g) => g.guide === guideNo).steps[step - 1].points[point].at;
      assert(at, `step ${step}'s point ${point} is worked out`);
      return at;
    };
    // The ring on the page is where the core worked the step's point out
    // to be, to within a pixel and a half, with its tag.
    // The view may still be gliding to the guide's points, so the ring and
    // the view are read on one frame, until the view is at rest.
    const ringOn = async (page, guideNo, step, point, label) => {
      const want = ringAt(guideNo, step, point);
      let shown;
      let at;
      for (let i = 0; i < 40; i++) {
        shown = await ring(page);
        at = await onPage(page, want);
        if (Math.hypot(shown.at[0] - at[0], shown.at[1] - at[1]) <= 1.5) break;
        await sleep(100);
      }
      assert(Math.hypot(shown.at[0] - at[0], shown.at[1] - at[1]) <= 1.5, `step ${step}'s ring at (${shown.at}), the core's point at (${at})`);
      assert(shown.tag.includes(label), `the ring's tag names ${label}: ${shown.tag}`);
      return shown;
    };
    // Points the cursor at `at`, in drawing units, and waits until it holds
    // the snap there, exactly: a click then takes that point.
    const hold = async (page, at) => {
      const [x, y] = await onPage(page, at);
      for (let i = 0; i < 40; i++) {
        await page.mouse.move(x + (i % 2 ? 0.3 : 0), y);
        await frames(page);
        const h = await page.evaluate(() => window.agentdrafterViewer.hovered());
        if (h && Math.abs(h.at[0] - at[0]) < 1e-9 && Math.abs(h.at[1] - at[1]) < 1e-9) return [x, y];
      }
      throw new Error(`the cursor holds no snap at ${at}`);
    };
    const clickAt = async (page, at) => {
      const [x, y] = await hold(page, at);
      await page.mouse.click(x, y);
      await frames(page);
    };
    // The newest journal entry, once there is one past `before`.
    const landedAfter = async (before, what) => {
      const until = Date.now() + 30000;
      let j = journal();
      while (j.entries.length <= before) {
        assert(Date.now() < until, `${what}: nothing landed`);
        await sleep(100);
        j = journal();
      }
      return j.entries.at(-1);
    };
    const fitAll = async (page) => {
      await page.click('[aria-label="Fit the sheet"]');
      await sleep(600);
    };
    try {
      // A laptop: each step by hand.
      {
        const context = await b.newContext(LAPTOP);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        const n = await give();
        assert(n === 1, `guide 1: ${n}`);
        await page.waitForSelector('[data-toast="start-guide"]', { timeout: 15000 });
        const toast = await page.textContent('.va-toast');
        assert(toast.includes('Assistant gives a guide') && toast.includes('4 steps'), `the toast names the guide: ${toast}`);
        await shot(page, 'guides-laptop-1-offer.png');
        await page.click('[data-toast="start-guide"]');
        await untilStep(page, 1, 'Start');
        let words = await banner(page);
        assert(words.includes('Step 1 of 4') && words.includes('Make wire the current layer') && words.includes('Pick wire in the ribbon'), `step 1's banner: ${words}`);
        assert((await pulse(page)) === 'layer', `the Layer box pulses: ${await pulse(page)}`);
        // Another layer: a note, and the step waits; then wire.
        await page.selectOption('.layerbox select', 'cable');
        await note(page, 'That made cable current; the step wants wire.', 'another layer');
        await shot(page, 'guides-laptop-2-step1-layer.png');
        await page.selectOption('.layerbox select', 'wire');
        await untilStep(page, 2, 'wire made current');
        words = await banner(page);
        assert(words.includes('Step 1 done') && words.includes('Start Line: type L and Enter'), `step 2's banner: ${words}`);
        assert((await pulse(page)) === 'line', `Line pulses: ${await pulse(page)}`);
        const first = await ringOn(page, 1, 2, 0, 'FT-101’s + terminal');
        assert(first.tag.includes('2'), `the ring's tag: ${first.tag}`);
        await shot(page, 'guides-laptop-3-step2-ring.png');
        // Another tool: its note, and End it.
        await page.click('.rb[data-tool="rectangle"]');
        await note(page, 'That started Rectangle; this step wants Line. End it?', 'another tool');
        await shot(page, 'guides-laptop-4-other-tool.png');
        await page.click('[data-g="noteact"]');
        await page.waitForFunction(() => !document.querySelector('.stage')?.dataset.tool, null, { timeout: 15000 });
        // Line, then Esc with nothing drawn: the step waits.
        await page.click('.rb[data-tool="line"]');
        await noNote(page);
        await page.keyboard.press('Escape');
        await note(page, 'Line ended with nothing drawn, so the step waits.', 'Esc');
        await shot(page, 'guides-laptop-5-esc.png');
        assert((await stepShown(page)) === 2, 'the step waits');
        // By hand: L, the ring, a wrong point and Undo it, the ring, Enter.
        await page.mouse.move(700, 500);
        await page.keyboard.type('L');
        await page.keyboard.press('Enter');
        await frames(page);
        await clickAt(page, ringAt(1, 2, 0));
        await ringOn(page, 1, 2, 1, 'TB1-1');
        await shot(page, 'guides-laptop-6-step2-second-point.png');
        await clickAt(page, [7.2, 6.25]);
        await note(page, 'That went to the endpoint of field-wire-2; the step wants TB1-1. Undo it?', 'a wrong point');
        await shot(page, 'guides-laptop-7-wrong-point.png');
        await page.click('[data-g="noteact"]');
        await noNote(page);
        await ringOn(page, 1, 2, 1, 'TB1-1');
        let before = journal().entries.length;
        await clickAt(page, ringAt(1, 2, 1));
        assert((await pulse(page)) === 'done', `Done pulses once the line has its points: ${await pulse(page)}`);
        await page.keyboard.press('Enter');
        let entry = await landedAfter(before, 'the wire');
        assert(JSON.stringify(entry.guide) === '{"guide":1,"step":2,"of":4,"completes":true}', `the journal marks step 2, completed: ${JSON.stringify(entry)}`);
        await untilStep(page, 3, 'the wire landed');
        words = await banner(page);
        assert(words.includes(`Step 2 done · change set ${entry.entry}`), `step 2 done: ${words}`);
        // Text, by hand, at the ring on the wire it drew.
        const middle = ringAt(1, 3, 0);
        assert(Math.abs(middle[0] - 5.2) < 1e-9 && Math.abs(middle[1] - 6.75) < 1e-9, `the wire's middle: ${middle}`);
        await ringOn(page, 1, 3, 0, 'the wire’s middle');
        await shot(page, 'guides-laptop-8-step3-ticked.png');
        await page.keyboard.type('T');
        await page.keyboard.press('Enter');
        await frames(page);
        await clickAt(page, middle);
        await page.waitForFunction(() => (document.querySelector('[data-g-now]')?.textContent ?? '').includes('Type 101+'), null, { timeout: 15000 });
        before = journal().entries.length;
        await page.keyboard.type('101+');
        await page.keyboard.press('Enter');
        await page.keyboard.press('Enter');
        entry = await landedAfter(before, 'the number');
        assert(entry.guide?.step === 3, `step 3 marked: ${JSON.stringify(entry.guide)}`);
        await untilStep(page, 4, 'the number landed');
        // Move, by hand: the number picked in its ring, the base point, @0,0.04.
        const box = await ring(page, 'object');
        assert(box.tag.includes('101+'), `the ring is round 101+: ${box.tag}`);
        await shot(page, 'guides-laptop-9-step4-pick.png');
        await page.keyboard.type('M');
        await page.keyboard.press('Enter');
        await frames(page);
        const text = listed()[0].steps[2].made;
        const { place, bounds } = await objectOf(page, text);
        assert(place >= 0, `the number ${text} is on the sheet`);
        let picked = false;
        for (let i = 1; i < 10 && !picked; i++) {
          const at = [bounds[0] + ((bounds[2] - bounds[0]) * i) / 10, (bounds[1] + bounds[3]) / 2];
          if ((await page.evaluate(([x, y]) => window.agentdrafterViewer.pickAt(x, y, 16), at)) !== place) continue;
          await page.mouse.click(...(await onPage(page, at)));
          picked = true;
        }
        assert(picked, 'a click picks the number');
        await frames(page);
        await page.keyboard.press('Enter');
        await ringOn(page, 1, 4, 1, 'base point');
        await clickAt(page, ringAt(1, 4, 1));
        await page.waitForFunction(() => (document.querySelector('[data-g-now]')?.textContent ?? '').includes('@-0.4,0.04'), null, { timeout: 15000 });
        await shot(page, 'guides-laptop-10-step4-type.png');
        before = journal().entries.length;
        await page.keyboard.type('@-0.4,0.04');
        await page.keyboard.press('Enter');
        entry = await landedAfter(before, 'the move');
        assert(entry.guide?.step === 4, `step 4 marked: ${JSON.stringify(entry.guide)}`);
        const checked = agentIn(folder, 'check', 'demo-plant', '--sheet', 'DP-LD-101');
        assert(checked.ok && checked.result.passed, `the completed guide passes every check: ${JSON.stringify(checked)}`);
        // The end: every step done, what landed, and one Undo.
        await page.waitForSelector('[data-guide="end"]', { timeout: 15000 });
        words = await banner(page);
        assert(words.includes('Finished: 4 of 4 steps done'), `the summary: ${words}`);
        assert(words.includes('Undo the guide’s steps'), `one Undo: ${words}`);
        await shot(page, 'guides-laptop-11-done.png');
        // The journal: each step's change set marked.
        const marks = journal().entries.filter((e) => e.guide).map((e) => `${e.entry}:${e.guide.step}`);
        await sideTab(page, 'journal');
        await page.waitForFunction(() => document.body.textContent.includes('guide step 4 of 4'), null, { timeout: 15000 });
        await shot(page, 'guides-laptop-12-journal.png');
        await sideTab(page, 'chat');
        const guided = journal().entries.filter((e) => e.guide && e.undo_state === 'kept').map((e) => e.entry);
        const others = journal().entries.filter((e) => !e.guide && e.kind === 'change_set' && e.undo_state === 'kept').map((e) => e.entry);
        assert(guided.length === 3 && others.length === 1, `three steps landed, after the wire's cut: ${guided} and ${others}`);
        before = journal().entries.length;
        await page.click('[data-g="undoall"]');
        const undone = await landedAfter(before, 'Undo the guide’s steps');
        const sorted = (list) => JSON.stringify([...list].sort((a, b) => a - b));
        assert(undone.kind === 'undo' && sorted(undone.undid) === sorted(guided), `one undo took back exactly ${guided}: ${JSON.stringify(undone)}`);
        const kept = journal().entries.filter((e) => others.includes(e.entry)).every((e) => e.undo_state === 'kept');
        assert(kept, `the wire's cut, before the guide, is kept: ${JSON.stringify(journal().entries.map((e) => [e.entry, e.undo_state]))}`);
        await page.waitForFunction(() => (document.querySelector('[data-g-line]')?.textContent ?? '').startsWith('Undone'), null, { timeout: 15000 });
        await shot(page, 'guides-laptop-13-undone.png');
        await page.click('[data-g="close"]');
        await page.waitForFunction(() => !document.querySelector('.gbanner'), null, { timeout: 15000 });
        const dropUntil = Date.now() + 10000;
        while (listed().some((g) => g.guide === 1)) {
          assert(Date.now() < dropUntil, `guide 1 was not ended on Close: ${JSON.stringify(listed())}`);
          await sleep(100);
        }
        // Template-object guide step (#264): Move on a template object goes
        // through Change all 7 sheets carrying the guide mark and ticks the step.
        const tplGuide = await give({
          title: 'Move TB1-2 down 0.1 in',
          sheet: 'DP-LD-101',
          steps: [
            {
              say: 'Move TB1-2 down 0.1 in with Move.',
              title: 'Move TB1-2 down',
              tool: 'move',
              points: [
                { object: { id: 'DP-LD-101/terminal-2' }, label: 'TB1-2' },
                { at: { snap: { id: 'DP-LD-101/terminal-2', snap: 'midpoint', which: 'objects[0].segments[3]' } }, label: 'base point' },
              ],
              completes: { op: 'move', target: { id: 'DP-LD-101/terminal-2' }, by: [0, -0.1, 0] },
            },
          ],
        });
        await page.waitForSelector('[data-toast="start-guide"]', { timeout: 15000 });
        await page.click('[data-toast="start-guide"]');
        await untilStep(page, 1, 'template guide start');
        await ring(page, 'object');
        for (let same = 0, last = ''; same < 4; ) {
          await frames(page);
          const cur = JSON.stringify(await page.evaluate(() => window.agentdrafterViewer.view()));
          same = cur === last ? same + 1 : 0;
          last = cur;
        }
        await page.keyboard.type('M');
        await page.keyboard.press('Enter');
        await frames(page);
        const term2 = await objectOf(page, 'DP-LD-101/terminal-2');
        let pickedTerm = false;
        for (let i = 1; i < 10 && !pickedTerm; i++) {
          for (let j = 1; j < 8 && !pickedTerm; j++) {
            const at = [
              term2.bounds[0] + ((term2.bounds[2] - term2.bounds[0]) * i) / 10,
              term2.bounds[1] + ((term2.bounds[3] - term2.bounds[1]) * j) / 8,
            ];
            if ((await page.evaluate(([x, y]) => window.agentdrafterViewer.pickAt(x, y, 16), at)) !== term2.place) continue;
            await page.mouse.click(...(await onPage(page, at)));
            pickedTerm = true;
          }
        }
        assert(pickedTerm, 'a click picks terminal-2');
        await frames(page);
        await page.keyboard.press('Enter');
        await ringOn(page, tplGuide, 1, 1, 'base point');
        await clickAt(page, ringAt(tplGuide, 1, 1));
        await page.keyboard.type('@0,-0.1');
        await page.keyboard.press('Enter');
        await page.waitForSelector('[data-preview] [data-pv-summary]', { timeout: 30000 });
        before = journal().entries.length;
        await page.click('[data-pv="go"]');
        const tplEntry = await landedAfter(before, 'template guide move');
        assert(
          JSON.stringify(tplEntry.guide) === `{"guide":${tplGuide},"step":1,"of":1,"completes":true}`,
          `template Change all kept the guide mark: ${JSON.stringify(tplEntry)}`,
        );
        await page.waitForSelector('[data-guide="end"]', { timeout: 15000 });
        words = await banner(page);
        assert(words.includes('Finished: 1 of 1 steps done'), `template guide ticked: ${words}`);
        before = journal().entries.length;
        await page.click('[data-g="undoall"]');
        await landedAfter(before, 'undo template guide');
        await page.click('[data-g="close"]');
        await page.waitForFunction(() => !document.querySelector('.gbanner'), null, { timeout: 15000 });
        const tplDropUntil = Date.now() + 10000;
        while (listed().some((g) => g.guide === tplGuide)) {
          assert(Date.now() < tplDropUntil, `guide ${tplGuide} was not ended on Close`);
          await sleep(100);
        }
        seen.laptop = { marks, undone: undone.undid, tplGuide, problems };
        assert(!problems.length, `no page errors: ${problems.join('; ')}`);
        await context.close();
      }
      // A phone: Tools pulses, taps, Do it for me, Skip and Stop.
      {
        const context = await b.newContext(PHONE);
        const { page, problems } = await open(context, `${server.base}/?source=demo-plant&sheet=DP-LD-101`, 'DP-LD-101');
        await live(page);
        const n = await give();
        await page.waitForSelector('[data-toast="start-guide"]', { timeout: 15000 });
        await page.tap('[data-toast="start-guide"]');
        await untilStep(page, 1, 'Start');
        await sleep(600);
        const narrowBanner = await banner(page);
        assert(narrowBanner.includes('Open Tools, and pick wire in the Layer box.') && narrowBanner.includes('Skip') && narrowBanner.includes('Stop'), `the phone's banner: ${narrowBanner}`);
        assert(await page.evaluate(() => getComputedStyle(document.querySelector('.toolsbtn')).animationName === 'gpulse'), 'Tools pulses');
        await shot(page, 'guides-phone-1-step1-layer.png');
        // Do it for me: the layer.
        await page.tap('[data-g="forme"]');
        await untilStep(page, 2, 'Do it for me, the layer');
        // Line by taps: the ring, a wrong point taken back, the ring, Done.
        await page.tap('.toolsbtn');
        await page.tap('.rb[data-tool="line"]', { force: true });
        await frames(page);
        const tapAt = async (at) => page.touchscreen.tap(...(await onPage(page, at)));
        await ring(page);
        await shot(page, 'guides-phone-2-step2-ring.png');
        await tapAt(ringAt(n, 2, 0));
        await frames(page);
        assert((await ring(page)).tag.includes('TB1-1'), 'the tap took the ring’s point');
        await tapAt([7.2, 6.25]);
        await note(page, 'the step wants TB1-1. Undo it?', 'a wrong tap');
        await shot(page, 'guides-phone-3-wrong-point.png');
        await page.tap('[data-g="noteact"]');
        await noNote(page);
        await tapAt(ringAt(n, 2, 1));
        let before = journal().entries.length;
        await page.tap('.opt[data-opt="tool:done"]');
        const entry = await landedAfter(before, 'the wire, by taps');
        assert(entry.guide?.guide === n && entry.guide.step === 2, `the tapped wire marked: ${JSON.stringify(entry.guide)}`);
        await untilStep(page, 3, 'the wire tapped');
        await shot(page, 'guides-phone-4-step3.png');
        // Do it for me: the number, as the drafter would land it.
        before = journal().entries.length;
        await page.tap('[data-g="forme"]');
        const forMe = await landedAfter(before, 'Do it for me');
        assert(forMe.guide?.step === 3 && forMe.change_set.operations[0].op === 'add_text', `done for you: ${JSON.stringify(forMe)}`);
        await untilStep(page, 4, 'Do it for me, the number');
        // Skip the last: the end, with what was done for you and skipped.
        await page.tap('[data-g="skip"]');
        await page.waitForSelector('[data-guide="end"]', { timeout: 15000 });
        let words = await banner(page);
        assert(words.includes('Finished: 3 of 4 steps done') && words.includes('skipped') && words.includes('done for you'), `the phone's summary: ${words}`);
        await shot(page, 'guides-phone-5-done.png');
        await page.tap('[data-g="close"]');
        // Another guide, skipping the wire: the steps that need it skip too, and say why.
        const m = await give();
        await page.waitForSelector('[data-toast="start-guide"]', { timeout: 15000 });
        await page.tap('[data-toast="start-guide"]');
        await untilStep(page, 1, 'the second start');
        await page.tap('[data-g="skip"]');
        await untilStep(page, 2, 'step 1 skipped');
        await shot(page, 'guides-phone-6-step1-skipped.png');
        await page.tap('[data-g="skip"]');
        await page.waitForSelector('[data-guide="end"]', { timeout: 15000 });
        words = await banner(page);
        assert(words.includes('skipped: it needs what step 2 makes') && words.includes('Nothing landed.'), `skipped with the steps that need it: ${words}`);
        await shot(page, 'guides-phone-7-skipped-rest-follow.png');
        await page.tap('[data-g="close"]');
        // A third, stopped at its first step.
        await give();
        await page.waitForSelector('[data-toast="start-guide"]', { timeout: 15000 });
        await page.tap('[data-toast="start-guide"]');
        await untilStep(page, 1, 'the third start');
        await page.tap('[data-g="stop"]');
        await page.waitForSelector('[data-guide="end"]', { timeout: 15000 });
        words = await banner(page);
        assert(words.includes('Stopped at step 1 of 4') && words.includes('not reached'), `stopped: ${words}`);
        await shot(page, 'guides-phone-8-stopped.png');
        seen.phone = { guides: [n, m], problems };
        assert(!problems.length, `no page errors: ${problems.join('; ')}`);
        await context.close();
      }
      return { seen, shots };
    } finally {
      await b.close();
      await server.stop();
    }
  },

  desktop: async () => {
    const folder = freshPlant('desktop');
    const setPath = join(folder, 'demo-plant');
    const dottedSetPath = join(folder, 'project.yaml');
    cpSync(setPath, dottedSetPath, { recursive: true });
    const cleanDwg = join(folder, 'clean-orig.dwg');
    const lossDwg = join(folder, 'loss-orig.dwg');
    const damagedDwg = join(folder, 'damaged-orig.dwg');
    const singleYaml = join(folder, 'geometry.yaml');
    cpSync(join(root, 'examples', 'geometry.yaml'), singleYaml);
    const builtClean = agentIn(folder, 'build', singleYaml, '--output', cleanDwg, '--overwrite');
    assert(builtClean.ok, `clean DWG built: ${JSON.stringify(builtClean.error)}`);
    // Real made-up synthetic fixtures (ADR-0008): NO mocking of `read`!
    const customFixtureA = join(root, 'tests', 'fixtures', 'desktop-custom-undecoded-a.dwg');
    const customFixtureB = join(root, 'tests', 'fixtures', 'desktop-custom-undecoded-b.dwg');
    const damagedFixture = join(root, 'tests', 'fixtures', 'desktop-damaged-line.dwg');
    cpSync(customFixtureA, lossDwg);
    cpSync(damagedFixture, damagedDwg);
    const origCleanSha = createHash('sha256').update(readFileSync(cleanDwg)).digest('hex');
    const origLossShaA = createHash('sha256').update(readFileSync(lossDwg)).digest('hex');
    const origLossShaB = createHash('sha256').update(readFileSync(customFixtureB)).digest('hex');
    assert(origLossShaA !== origLossShaB, 'replacement undecoded fixture has distinct SHA-256');

    const server = await serve('127.0.0.1:0', folder);
    const b = await browser();
    const shots = [];
    const shot = async (page, file) => {
      await page.screenshot({ path: join(out, file) });
      shots.push(file);
    };
    const seen = {};
    const installDesktopBridge = async (context, initialRecents = []) => {
      await context.addInitScript(
        ({ setPath: sp, cleanDwg: cd, singleYaml: sy, initialRecents: ir }) => {
          const recents = ir.slice();
          const newWindows = [];
          const titles = [];
          window.__desktopTest = {
            nextFolderError: null,
            nextFolder: sp,
            nextDwg: cd,
            recents,
            newWindows,
            titles,
          };
          window.__AGENTDRAFTER_DESKTOP__ = {
            async pickDrawingSetFolder() {
              if (window.__desktopTest.nextFolderError) {
                const msg = window.__desktopTest.nextFolderError;
                window.__desktopTest.nextFolderError = null;
                throw new Error(msg);
              }
              const p = window.__desktopTest.nextFolder;
              const name = p.split(/[\\/]/).pop() || p;
              return { kind: 'set', path: p, name, detail: 'Drawing set folder · set.yaml' };
            },
            async pickDrawingFile(filter) {
              if (filter === 'yaml') {
                return { kind: 'sheet', path: sy, name: 'geometry.yaml', detail: 'Single sheet description · YAML' };
              }
              const p = window.__desktopTest.nextDwg;
              const name = p.split(/[\\/]/).pop();
              return { kind: 'dwg', path: p, name, detail: 'Plant DWG · original read-only' };
            },
            async inspectPath(path) {
              const name = path.split(/[\\/]/).pop() || path;
              const kind =
                path === window.__desktopTest.nextFolder || path === sp
                  ? 'set'
                  : path.endsWith('.dwg')
                    ? 'dwg'
                    : path.endsWith('.yaml')
                      ? 'sheet'
                      : 'set';
              return { kind, path, name, detail: kind };
            },
            async listRecents() {
              return recents.slice();
            },
            async recordRecent(entry) {
              const idx = recents.findIndex((r) => r.path === entry.path);
              if (idx >= 0) recents.splice(idx, 1);
              recents.unshift({ ...entry, opened_at_ms: Date.now() });
              return recents.slice();
            },
            async removeRecent(path) {
              const idx = recents.findIndex((r) => r.path === path);
              if (idx >= 0) recents.splice(idx, 1);
              return recents.slice();
            },
            async openInNewWindow(source, title, kind) {
              newWindows.push({ source, title, kind });
              return `win-${newWindows.length + 1}`;
            },
            async setWindowTitle(title) {
              titles.push(title);
            },
          };
        },
        { setPath, cleanDwg, singleYaml, initialRecents },
      );
    };

    try {
      // 1. Laptop (1440x900): Start screen, invalid folder error, drawing set, dotted set folder,
      //    switcher, New window (folder & file), clean plant DWG successive accumulation + undo +
      //    cache-key invalidation + accept, REAL decoded:false custom-object DWG (view-only,
      //    decode modal, allow_loss, view-only undo lock, SHA-256 consent revocation),
      //    REAL damaged-line DWG rendering via read_past_damage, single YAML, and plain serve isolation.
      {
        const context = await b.newContext(LAPTOP);
        await installDesktopBridge(context);
        const page = await context.newPage();
        const problems = [];
        const http4xx = [];
        page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
        page.on('console', (m) => m.type() === 'error' && problems.push(`console: ${m.text()}`));
        page.on('response', async (r) => {
          if (r.status() >= 400) {
            http4xx.push(`${r.status()} ${r.url()} ${r.request().postData() ?? ''} => ${await r.text().catch(() => '')}`);
          }
        });

        // Observe real `change` payloads without mocking `read` or `change`!
        let sawAllowLossOnChange = false;
        let sawExpectedShaOnChange = false;
        let lossChangeCount = 0;
        await page.route('**/commands/change', async (route) => {
          const post = route.request().postDataJSON();
          if (post?.set === lossDwg) {
            lossChangeCount += 1;
            if (post?.allow_loss === true) sawAllowLossOnChange = true;
            if (post?.expected_original_sha256 === origLossShaA) sawExpectedShaOnChange = true;
          }
          await route.continue();
        });

        await page.goto(`${server.base}/?desktop=1&start=1`, { waitUntil: 'domcontentloaded' });
        await page.waitForSelector('[data-desktop-start]', { timeout: 15000 });
        await shot(page, 'desktop-laptop-1-start.png');

        // Invalid folder error on Start screen.
        await page.evaluate(() => {
          window.__desktopTest.nextFolderError = 'Not a drawing set: `misc-notes` has no `set.yaml`.';
        });
        await page.click('[data-pick="set"]');
        await page.waitForSelector('[data-desktop-error]', { timeout: 5000 });
        const errText = await page.textContent('[data-desktop-error]');
        assert(errText.includes('set.yaml'), `invalid folder error shown: ${errText}`);

        // Pick valid drawing set folder -> opens demo-plant.
        await page.click('[data-pick="set"]');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-101', null, { timeout: 20000 });
        await live(page);
        assert((await page.textContent('[data-act="desktop-switcher"]')).includes('demo-plant'), 'switcher shows demo-plant');
        await shot(page, 'desktop-laptop-2-set.png');

        // Open switcher, test New window (recent + DWG/YAML file picker button), then open dotted set folder `project.yaml/`.
        await page.click('[data-act="desktop-switcher"]');
        await page.waitForSelector('[data-desktop-switcher]', { timeout: 5000 });
        await page.click('[data-desktop-switcher] [data-recent-newwin]');
        await page.click('[data-act="desktop-switcher"]');
        await page.waitForSelector('[data-desktop-switcher]', { timeout: 5000 });
        await page.click('[data-sw-pick="file-newwin"]');
        const newWins = await page.evaluate(() => window.__desktopTest.newWindows.slice());
        assert(
          newWins.length === 2 && newWins[0].kind === 'set' && newWins[1].kind === 'dwg',
          `New window invoked for both set and file: ${JSON.stringify(newWins)}`,
        );

        // Dotted set folder `project.yaml/` opens as a drawing set (`kind=set`), not a single YAML sheet (Blocker #11).
        await page.evaluate((dp) => {
          window.__desktopTest.nextFolder = dp;
        }, dottedSetPath);
        await page.click('[data-act="desktop-switcher"]');
        await page.waitForSelector('[data-desktop-switcher]', { timeout: 5000 });
        await page.click('[data-sw-pick="set"]');
        await page.waitForFunction(
          () =>
            window.agentdrafterViewer?.summary?.sheet === 'DP-LD-101' &&
            document.querySelector('[data-act="desktop-switcher"]')?.textContent?.includes('project.yaml'),
          null,
          { timeout: 20000 },
        );
        assert(!(await page.$('[data-dwg-banner="sheet"]')), 'dotted set folder project.yaml/ is NOT misclassified as a single sheet');
        assert(await page.evaluate(() => location.search.includes('kind=set')), 'URL carries kind=set');

        // Go back to Start screen and confirm recents.
        await page.click('[data-act="desktop-switcher"]');
        await page.waitForSelector('[data-desktop-switcher]', { timeout: 5000 });
        await page.click('[data-sw-start]');
        await page.waitForSelector('[data-desktop-start]', { timeout: 5000 });
        assert(await page.isVisible(`[data-recent="${setPath.replace(/\\/g, '\\\\')}"]`), 'demo-plant listed in Recents');

        // Open clean plant DWG -> verify read-only badge, successive edits accumulate (Blocker #4),
        // stack undo, cache-key invalidation on reused rev1.dwg (Blocker #5), and accept.
        await page.click('[data-pick="dwg"]');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'clean-orig', null, { timeout: 20000 });
        await page.waitForSelector('[data-dwg-banner="clean"]', { timeout: 10000 });
        assert((await page.textContent('[data-dwg-version]')).includes('AC1032'), 'shows DWG version pill');
        assert(!(await page.isDisabled('.rb[data-tool="line"]')), 'drawing tools enabled on clean DWG');
        const origObjCount = await page.evaluate(() => window.agentdrafterViewer.summary.objects.length);
        await shot(page, 'desktop-laptop-3-dwg-clean.png');

        // Draw Line A (1,1 -> 4,1) on clean-orig.dwg -> clean-orig.rev1.dwg.
        await page.click('.rb[data-tool="line"]');
        await page.fill('.cmdin', '1,1');
        await page.keyboard.press('Enter');
        await page.fill('.cmdin', '4,1');
        await page.keyboard.press('Enter');
        await page.keyboard.press('Enter');
        await page.waitForFunction(
          (expectedCount) =>
            document.querySelector('.titlebar button[data-act="undo"]')?.textContent?.includes('(#1)') &&
            window.agentdrafterViewer?.summary?.sheet === 'clean-orig.rev1' &&
            window.agentdrafterViewer?.summary?.objects?.length === expectedCount &&
            document.querySelector('[data-dwg-banner="clean"]')?.textContent?.includes('clean-orig.rev1.dwg'),
          origObjCount + 1,
          { timeout: 20000 },
        );

        // Draw Line B (1,2 -> 4,2) -> clean-orig.rev2.dwg MUST contain BOTH Line A and Line B (Blocker #4).
        await page.click('.rb[data-tool="line"]');
        await page.fill('.cmdin', '1,2');
        await page.keyboard.press('Enter');
        await page.fill('.cmdin', '4,2');
        await page.keyboard.press('Enter');
        await page.keyboard.press('Enter');
        await page.waitForFunction(
          (expectedCount) =>
            document.querySelector('.titlebar button[data-act="undo"]')?.textContent?.includes('(#2)') &&
            window.agentdrafterViewer?.summary?.sheet === 'clean-orig.rev2' &&
            window.agentdrafterViewer?.summary?.objects?.length === expectedCount &&
            document.querySelector('[data-dwg-banner="clean"]')?.textContent?.includes('clean-orig.rev2.dwg'),
          origObjCount + 2,
          { timeout: 20000 },
        );
        await sleep(250);
        const rev1Path = join(folder, 'clean-orig.rev1.dwg');
        const rev2Path = join(folder, 'clean-orig.rev2.dwg');
        assert(existsSync(rev1Path) && existsSync(rev2Path), 'rev1 and rev2 DWGs written beside original');
        assert(
          createHash('sha256').update(readFileSync(cleanDwg)).digest('hex') === origCleanSha,
          'original DWG untouched after two successive revisions',
        );

        // Undo #2 -> back to rev1 with origObjCount + 1.
        await page.click('.titlebar button[data-act="undo"]');
        await page.waitForFunction(
          (expectedCount) =>
            document.querySelector('.titlebar button[data-act="undo"]')?.textContent?.includes('(#1)') &&
            window.agentdrafterViewer?.summary?.sheet === 'clean-orig.rev1' &&
            window.agentdrafterViewer?.summary?.objects?.length === expectedCount,
          origObjCount + 1,
          { timeout: 20000 },
        );
        assert(!existsSync(rev2Path), 'undo #2 removed clean-orig.rev2.dwg');
        await sleep(250);

        // Undo #1 -> back to original with origObjCount.
        await page.click('.titlebar button[data-act="undo"]');
        await page.waitForFunction(
          (expectedCount) => {
            const b = document.querySelector('.titlebar button[data-act="undo"]');
            return (
              b &&
              b.disabled &&
              !b.textContent?.includes('(#1)') &&
              window.agentdrafterViewer?.summary?.sheet === 'clean-orig' &&
              window.agentdrafterViewer?.summary?.objects?.length === expectedCount
            );
          },
          origObjCount,
          { timeout: 20000 },
        );
        assert(!existsSync(rev1Path), 'undo #1 removed clean-orig.rev1.dwg');

        // Draw Line C (1,5 -> 6,5), which reuses clean-orig.rev1.dwg -> rendered bounds MUST reflect y=5 (NOT stale Line A y=1, Blocker #5)!
        await page.click('.rb[data-tool="line"]');
        await page.fill('.cmdin', '1,5');
        await page.keyboard.press('Enter');
        await page.fill('.cmdin', '6,5');
        await page.keyboard.press('Enter');
        await page.keyboard.press('Enter');
        await page.waitForFunction(
          (expectedCount) => {
            const probe = window.agentdrafterViewer;
            if (!probe || probe.summary?.sheet !== 'clean-orig.rev1' || probe.summary.objects.length !== expectedCount) return false;
            const lastPlace = probe.summary.objects.length - 1;
            const [, y0, , y1] = probe.bounds([lastPlace]);
            return Math.abs(y0 - 5) < 0.05 && Math.abs(y1 - 5) < 0.05;
          },
          origObjCount + 1,
          { timeout: 20000 },
        );

        // Accept the working revision.
        await page.click('.titlebar button[data-act="accept"]');
        await page.waitForSelector('.va-modal [data-ok]', { timeout: 10000 });
        await page.click('.va-modal [data-ok]');
        await page.waitForFunction(
          () => document.querySelector('.va-status')?.textContent?.includes('Issued'),
          null,
          { timeout: 20000 },
        );
        assert(
          createHash('sha256').update(readFileSync(cleanDwg)).digest('hex') === origCleanSha,
          'original DWG untouched after accept',
        );

        // Open REAL undecoded custom-object DWG (desktop-custom-undecoded-a.dwg, NO mocked `read`!).
        await page.evaluate((ld) => {
          window.__desktopTest.nextDwg = ld;
        }, lossDwg);
        await page.click('[data-act="desktop-switcher"]');
        await page.waitForSelector('[data-desktop-switcher]', { timeout: 5000 });
        await page.click('[data-sw-pick="file"]');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'loss-orig', null, { timeout: 20000 });
        await page.waitForSelector('[data-dwg-banner="view-only"]', { timeout: 15000 });
        assert(await page.isDisabled('.rb[data-tool="line"]'), 'drawing tools locked while view-only');
        assert(await page.isDisabled('.titlebar button[data-act="accept"]'), 'Accept locked while view-only');
        await shot(page, 'desktop-laptop-4-dwg-viewonly.png');

        await page.click('[data-act="inspect-loss"]');
        await page.waitForSelector('[data-decode-modal]', { timeout: 5000 });
        const modalText = await page.textContent('[data-decode-modal]');
        assert(modalText.includes('MADE_UP_VALVE'), `real decode modal lists MADE_UP_VALVE: ${modalText}`);
        assert(await page.isDisabled('[data-allow-loss-confirm]'), 'Allow button disabled before ticking checkbox');
        await shot(page, 'desktop-laptop-5-decode-modal.png');

        await page.check('[data-allow-loss-check]');
        assert(!(await page.isDisabled('[data-allow-loss-confirm]')), 'Allow button enabled after ticking checkbox');
        await page.click('[data-allow-loss-confirm]');
        await page.waitForSelector('[data-dwg-banner="allowed-loss"]', { timeout: 5000 });
        assert(!(await page.isDisabled('.rb[data-tool="line"]')), 'drawing tools unlocked after Allow Loss');

        // Draw a line on real undecoded loss-orig.dwg -> sends `allow_loss: true`, writes rev1 beside untouched original.
        await page.click('.rb[data-tool="line"]');
        await page.fill('.cmdin', '10,10');
        await page.keyboard.press('Enter');
        await page.fill('.cmdin', '40,10');
        await page.keyboard.press('Enter');
        await page.keyboard.press('Enter');
        await page.waitForFunction(
          () =>
            document.querySelector('.titlebar button[data-act="undo"]')?.disabled === false &&
            window.agentdrafterViewer?.summary?.sheet === 'loss-orig.rev1',
          null,
          { timeout: 20000 },
        );
        assert(
          sawAllowLossOnChange && sawExpectedShaOnChange && lossChangeCount === 1,
          `real change request carried allow_loss + expected_original_sha256: allow=${sawAllowLossOnChange} sha=${sawExpectedShaOnChange} count=${lossChangeCount}`,
        );
        assert(
          createHash('sha256').update(readFileSync(lossDwg)).digest('hex') === origLossShaA,
          'loss-orig.dwg original untouched',
        );

        // Blocker #6 + Re-review #4: Replace loss-orig.dwg on disk with a DIFFERENT undecoded DWG (distinct SHA-256)
        // while still in the unlocked session -> next edit MUST revoke consent, refuse without calling change,
        // re-engage view-only, AND display B (`loss-orig`) directly instead of A's `loss-orig.rev1`!
        cpSync(customFixtureB, lossDwg);
        const changesBeforeReplace = lossChangeCount;
        await page.click('.rb[data-tool="line"]');
        await page.fill('.cmdin', '10,20');
        await page.keyboard.press('Enter');
        await page.fill('.cmdin', '40,20');
        await page.keyboard.press('Enter');
        await page.keyboard.press('Enter');
        await page.waitForSelector('[data-dwg-banner="view-only"]', { timeout: 15000 });
        await page.waitForFunction(
          () => window.agentdrafterViewer?.summary?.sheet === 'loss-orig',
          null,
          { timeout: 15000 },
        );
        assert(
          lossChangeCount === changesBeforeReplace,
          `replaced DWG did not send change with stale allow_loss: ${lossChangeCount} vs ${changesBeforeReplace}`,
        );
        assert(await page.isDisabled('.rb[data-tool="line"]'), 'drawing tools re-locked after file replacement');
        // Restore original A so the unissued rev1 matches loss-orig.dwg for the view-only Undo lock test.
        cpSync(customFixtureA, lossDwg);

        // Blocker #7: Fresh page load on undecoded loss-orig.dwg (which has an unissued rev1 in its journal)
        // opens view-only -> BOTH titlebar Undo last AND journal entry Undo button MUST be disabled!
        await page.goto(`${server.base}/?desktop=1&source=${encodeURIComponent(lossDwg)}&kind=dwg`, { waitUntil: 'domcontentloaded' });
        await page.waitForSelector('[data-dwg-banner="view-only"]', { timeout: 20000 });
        await journalTab(page);
        await page.waitForSelector('li[data-entry="1"]', { timeout: 15000 });
        assert(await page.isDisabled('.titlebar button[data-act="undo"]'), 'titlebar Undo disabled while view-only (Blocker #7)');
        await page.click('li[data-entry="1"] .je-head');
        await page.waitForSelector('[data-je-undo="1"]', { timeout: 5000 });
        assert(await page.isDisabled('[data-je-undo="1"]'), 'journal entry Undo disabled while view-only (Blocker #7)');

        // Fix #2 (A -> B -> A alignment + SSE `read --no-mirror` refresh):
        // Return to live view (clicking entry 1 above stepped back), unlock allow_loss on A,
        // then externally swap disk A -> B and revise B via HTTP API ->
        // SSE MUST re-run `read --no-mirror`, revoke A's stale allow_loss (back to view-only),
        // and display B's `loss-orig.rev2`; then swapping disk back B -> A MUST display `loss-orig`
        // directly (NEVER skipping past B's entry 2 back to A's older entry 1 `loss-orig.rev1`)!
        if (await page.$('button[data-act="live"]')) {
          await page.click('button[data-act="live"]');
        }
        await page.click('[data-act="inspect-loss"]');
        await page.waitForSelector('[data-decode-modal]', { timeout: 5000 });
        await page.check('[data-allow-loss-check]');
        await page.click('[data-allow-loss-confirm]');
        await page.waitForSelector('[data-dwg-banner="allowed-loss"]', { timeout: 5000 });
        cpSync(customFixtureB, lossDwg);
        const extRes = await fetch(`${server.base}/commands/change`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({
            set: lossDwg,
            allow_loss: true,
            no_pdf: true,
            change_set: { operations: [{ op: 'add_line', from: [12, 12, 0], to: [24, 12, 0] }] },
          }),
        });
        const extJson = await extRes.json();
        assert(extJson.ok, `external revision on swapped B succeeded: ${JSON.stringify(extJson)}`);
        await page.waitForSelector('[data-dwg-banner="view-only"]', { timeout: 15000 });
        await page.waitForFunction(
          () => window.agentdrafterViewer?.summary?.sheet === 'loss-orig.rev2',
          null,
          { timeout: 15000 },
        );
        cpSync(customFixtureA, lossDwg);
        await page.goto(`${server.base}/?desktop=1&source=${encodeURIComponent(lossDwg)}&kind=dwg`, { waitUntil: 'domcontentloaded' });
        await page.waitForSelector('[data-dwg-banner="view-only"]', { timeout: 20000 });
        await page.waitForFunction(
          () => window.agentdrafterViewer?.summary?.sheet === 'loss-orig',
          null,
          { timeout: 15000 },
        );

        // Blocker #9: Open REAL damaged-line DWG (`desktop-damaged-line.dwg`) -> MUST render its
        // recoverable objects (circle + polyline, >= 2 objects) read-only via `display-list --read-past-damage`!
        await page.evaluate((dd) => {
          window.__desktopTest.nextDwg = dd;
        }, damagedDwg);
        await page.click('[data-act="desktop-switcher"]');
        await page.waitForSelector('[data-desktop-switcher]', { timeout: 5000 });
        await page.click('[data-sw-pick="file"]');
        await page.waitForFunction(
          () =>
            window.agentdrafterViewer?.summary?.sheet === 'damaged-orig' &&
            window.agentdrafterViewer?.summary?.objects?.length >= 2,
          null,
          { timeout: 20000 },
        );
        await page.waitForSelector('[data-dwg-banner="view-only"]', { timeout: 15000 });
        await page.click('[data-act="inspect-loss"]');
        await page.waitForSelector('[data-decode-modal]', { timeout: 5000 });
        const damagedModal = await page.textContent('[data-decode-modal]');
        assert(
          damagedModal.includes('LINE') && damagedModal.includes('Read past damage'),
          `damaged-line decode modal names LINE and Read past damage: ${damagedModal}`,
        );
        await page.click('[data-keep-viewonly]');

        // Open single YAML sheet -> renders cleanly, read-only banner, drawing tools disabled.
        await page.click('[data-act="desktop-switcher"]');
        await page.waitForSelector('[data-desktop-switcher]', { timeout: 5000 });
        await page.click('[data-sw-start]');
        await page.waitForSelector('[data-desktop-start]', { timeout: 5000 });
        await page.click('[data-pick="sheet"]');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'geometry', null, { timeout: 20000 });
        await page.waitForSelector('[data-dwg-banner="sheet"]', { timeout: 5000 });
        assert(await page.isDisabled('.rb[data-tool="line"]'), 'drawing tools disabled on single YAML sheet');
        await shot(page, 'desktop-laptop-6-single-sheet.png');

        seen.laptop = { newWindows: newWins.length, sawAllowLossOnChange, problems };
        assert(!problems.length, `laptop desktop errors: ${problems.join('; ')} | http4xx: ${http4xx.join(' || ')}`);
        await context.close();
      }

      // 2. Blocker #10: Plain browser `agentdrafter serve` opening a DWG (no desktop bridge, no ?desktop=1)
      //    keeps existing behavior: NO desktop switcher, NO DWG banner, and WASM does NOT enable DWG tools.
      {
        const context = await b.newContext(LAPTOP);
        const page = await context.newPage();
        const problems = [];
        page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
        page.on('console', (m) => m.type() === 'error' && problems.push(`console: ${m.text()}`));

        await page.goto(`${server.base}/?source=${encodeURIComponent(cleanDwg)}`, { waitUntil: 'domcontentloaded' });
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'clean-orig', null, { timeout: 20000 });
        assert(!(await page.$('[data-act="desktop-switcher"]')), 'plain serve has no desktop switcher');
        assert(!(await page.$('[data-dwg-banner]')), 'plain serve has no desktop DWG banner');
        // Clicking Line in plain browser mode on a DWG must NOT start the Line tool (desktop_mode is false).
        await page.click('.rb[data-tool="line"]');
        await sleep(200);
        const activeTool = await page.evaluate(() => document.querySelector('.rb[data-tool="line"]')?.classList.contains('on') ?? false);
        assert(!activeTool, 'plain serve does not unlock desktop DWG drafting tools');
        seen.plainServeDwg = { activeTool, problems };
        assert(!problems.length, `plain serve errors: ${problems.join('; ')}`);
        await context.close();
      }

      // 3. Phone (390x844): Start screen, drawing set title bar (zero overlap), switcher, single sheet.
      {
        const context = await b.newContext(PHONE);
        await installDesktopBridge(context);
        const page = await context.newPage();
        const problems = [];
        page.on('pageerror', (e) => problems.push(`pageerror: ${e.message}`));
        page.on('console', (m) => m.type() === 'error' && problems.push(`console: ${m.text()}`));

        await page.goto(`${server.base}/?desktop=1&start=1`, { waitUntil: 'domcontentloaded' });
        await page.waitForSelector('[data-desktop-start]', { timeout: 15000 });
        const startOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth + 1);
        assert(!startOverflow, 'phone Start screen has no horizontal overflow');
        await shot(page, 'desktop-phone-1-start.png');

        await page.tap('[data-pick="set"]');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'DP-LD-101', null, { timeout: 20000 });
        await live(page);
        const barFit = await page.evaluate(() => {
          const items = [...document.querySelectorAll('.titlebar > *')].filter((el) => getComputedStyle(el).display !== 'none');
          const rects = items.map((el) => el.getBoundingClientRect());
          let overlap = false;
          for (let i = 1; i < rects.length; i += 1) {
            if (rects[i].left < rects[i - 1].right - 1) overlap = true;
          }
          const rightMost = Math.max(...rects.map((r) => r.right));
          return { overlap, rightMost, innerWidth: window.innerWidth };
        });
        assert(!barFit.overlap && barFit.rightMost <= barFit.innerWidth + 1, `phone titlebar clean: ${JSON.stringify(barFit)}`);
        await shot(page, 'desktop-phone-2-set.png');

        await page.tap('[data-act="desktop-switcher"]');
        await page.waitForSelector('[data-desktop-switcher]', { timeout: 5000 });
        await page.tap('[data-sw-start]');
        await page.waitForSelector('[data-desktop-start]', { timeout: 5000 });
        await page.tap('[data-pick="sheet"]');
        await page.waitForFunction(() => window.agentdrafterViewer?.summary?.sheet === 'geometry', null, { timeout: 20000 });
        await page.waitForSelector('[data-dwg-banner="sheet"]', { timeout: 5000 });
        await shot(page, 'desktop-phone-3-sheet.png');

        seen.phone = { barFit, problems };
        assert(!problems.length, `phone desktop errors: ${problems.join('; ')}`);
        await context.close();
      }
      return { seen, shots };
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
