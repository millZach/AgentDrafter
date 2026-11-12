// Screenshots of a running viewer at any address, such as the one
// `agentdrafter serve` printed on another machine of the tailnet: at phone
// size with touch, fitted, pinched in on the title block, on another sheet
// and with the journal's sheet dragged up; and at laptop size, fitted and
// zoomed with the wheel. For proofs by hand; it changes nothing.
//
//   node e2e/look.mjs <viewer-address> <folder> [--chrome <chromium>] [--gl gpu|software]

import { existsSync, mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { chromium } from 'playwright-core';

const [address, folder] = process.argv.slice(2);
if (!address || !folder) throw new Error('usage: node e2e/look.mjs <viewer-address> <folder>');
const argv = process.argv.slice(4);
const option = (name) => {
  const at = argv.indexOf(name);
  return at >= 0 ? argv[at + 1] : undefined;
};
const chrome =
  option('--chrome') ??
  process.env.CHROME ??
  ['/usr/bin/chromium', '/usr/bin/google-chrome', '/usr/bin/google-chrome-stable'].find(existsSync);
const gl =
  option('--gl') === 'gpu'
    ? ['--use-angle=vulkan', '--enable-features=Vulkan', '--ignore-gpu-blocklist']
    : ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'];
const out = resolve(folder);
mkdirSync(out, { recursive: true });

const browser = await chromium.launch({ executablePath: chrome, args: ['--no-sandbox', ...gl] });
const seen = {};
try {
  const drawn = (page, sheet) =>
    page.waitForFunction((s) => window.agentdrafterViewer?.summary?.sheet === s || (!s && window.agentdrafterViewer?.summary), sheet, {
      timeout: 30000,
    });
  const settle = (page) => page.evaluate(() => new Promise((done) => setTimeout(() => requestAnimationFrame(() => done()), 600)));

  // A phone.
  const phone = await browser.newContext({
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 3,
    isMobile: true,
    hasTouch: true,
  });
  const page = await phone.newPage();
  const problems = [];
  page.on('pageerror', (e) => problems.push(e.message));
  await page.goto(address);
  await drawn(page, null);
  await settle(page);
  await page.screenshot({ path: join(out, 'phone-1-fitted.png') });
  seen.phone = await page.evaluate(() => ({
    address: location.href.replace(/token=[0-9a-f]+/, 'token=<token>'),
    token_kept: !!sessionStorage.getItem('agentdrafter-token'),
    summary: window.agentdrafterViewer.summary,
  }));
  const cdp = await phone.newCDPSession(page);
  const box = await page.evaluate(() => {
    const r = document.querySelector('.stage').getBoundingClientRect();
    return { x: r.left, y: r.top, width: r.width, height: r.height };
  });
  // Pinch in on the title block, at the paper's lower right.
  const view = await page.evaluate(() => window.agentdrafterViewer.view());
  const paper = seen.phone.summary.paper;
  const tx = box.x + (paper[2] - 3 - view.left) * view.scale;
  const ty = box.y + (view.top - (paper[1] + 1)) * view.scale;
  const touch = (type, points) =>
    cdp.send('Input.dispatchTouchEvent', { type, touchPoints: points.map(([x, y], id) => ({ x, y, id })) });
  await touch('touchStart', [[tx - 20, ty], [tx + 20, ty]]);
  for (let i = 1; i <= 40; i++) {
    await touch('touchMove', [[tx - 20 - i * 4, ty], [tx + 20 + i * 4, ty]]);
    await sleep(16);
  }
  await touch('touchEnd', []);
  // And drag the pinched point to the middle of the screen.
  await touch('touchStart', [[tx, ty]]);
  for (let i = 1; i <= 20; i++) {
    await touch('touchMove', [[tx + ((box.x + box.width / 2 - tx) * i) / 20, ty + ((box.y + box.height * 0.4 - ty) * i) / 20]]);
    await sleep(16);
  }
  await touch('touchEnd', []);
  await settle(page);
  await page.screenshot({ path: join(out, 'phone-2-pinched-title-block.png') });
  // Another sheet.
  await page.tap('.tab:nth-child(5)');
  const fifth = seen.phone.summary.sheets[4];
  await drawn(page, fifth);
  await settle(page);
  await page.screenshot({ path: join(out, 'phone-3-another-sheet.png') });
  // The journal's sheet dragged up.
  const grab = await page.evaluate(() => {
    const r = document.querySelector('.va-grab').getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  });
  await touch('touchStart', [[grab.x, grab.y]]);
  for (let i = 1; i <= 20; i++) {
    await touch('touchMove', [[grab.x, grab.y - i * 20]]);
    await sleep(16);
  }
  await touch('touchEnd', []);
  await settle(page);
  await page.screenshot({ path: join(out, 'phone-4-journal-dragged-up.png') });
  seen.phone.problems = problems;
  await phone.close();

  // A laptop.
  const laptop = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  const wide = await laptop.newPage();
  await wide.goto(address);
  await drawn(wide, null);
  await settle(wide);
  await wide.screenshot({ path: join(out, 'laptop-1-fitted.png') });
  await wide.mouse.move(820, 680);
  for (let i = 0; i < 6; i++) {
    await wide.mouse.wheel(0, -120);
    await sleep(30);
  }
  await settle(wide);
  await wide.screenshot({ path: join(out, 'laptop-2-wheel-zoomed.png') });
  seen.laptop = await wide.evaluate(() => ({ view: window.agentdrafterViewer.view() }));
  await laptop.close();
} finally {
  await browser.close();
}
console.log(JSON.stringify(seen, null, 2));
