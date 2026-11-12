import assert from 'node:assert/strict';
import { test } from 'node:test';

import { between, fit, keepInSight, limits, panBy, pinch, toDrawing, toScreen, zoomAt, type Paper } from '../src/view.ts';

const SHEET: Paper = [0, 0, 17, 11];
const close = (a: number, b: number, why: string) => assert.ok(Math.abs(a - b) < 1e-9, `${why}: ${a} against ${b}`);

test('a view puts the drawing point at the top left at the screen origin, y down', () => {
  const view = { left: 2, top: 10, scale: 50 };
  assert.deepEqual(toScreen(view, 2, 10), [0, 0]);
  assert.deepEqual(toScreen(view, 3, 9), [50, 50]);
  assert.deepEqual(toDrawing(view, 50, 50), [3, 9]);
});

test('fit shows the whole paper, centred in the area, inside its padding', () => {
  const view = fit(SHEET, { x: 0, y: 0, width: 1000, height: 800 }, 10);
  close(view.scale, 980 / 17, 'the width limits a landscape sheet');
  const [x0, y0] = toScreen(view, 0, 11);
  const [x1, y1] = toScreen(view, 17, 0);
  close(x0, 10, 'left edge');
  close(x1, 990, 'right edge');
  close((y0 + y1) / 2, 400, 'centred down');
  // A phone's area, above the bottom sheet's peek.
  const phone = fit(SHEET, { x: 0, y: 0, width: 390, height: 500 }, 10);
  const [, top] = toScreen(phone, 0, 11);
  const [, bottom] = toScreen(phone, 0, 0);
  close((top + bottom) / 2, 250, 'centred in the area above the peek');
});

test('zooming keeps the drawing point under the pointer, within the limits', () => {
  const fitted = fit(SHEET, { x: 0, y: 0, width: 1000, height: 800 });
  const bounds = limits(fitted);
  const under = toDrawing(fitted, 300, 200);
  const zoomed = zoomAt(fitted, 300, 200, fitted.scale * 3, bounds);
  close(zoomed.scale, fitted.scale * 3, 'zoomed three times');
  const [sx, sy] = toScreen(zoomed, under[0], under[1]);
  close(sx, 300, 'x stays under the pointer');
  close(sy, 200, 'y stays under the pointer');
  assert.equal(zoomAt(fitted, 0, 0, fitted.scale * 1000, bounds).scale, bounds.max);
  assert.equal(zoomAt(fitted, 0, 0, fitted.scale / 1000, bounds).scale, bounds.min);
});

test('a pan moves the drawing with the pointer', () => {
  const view = { left: 0, top: 11, scale: 40 };
  const moved = panBy(view, 80, -40);
  const [x, y] = toScreen(moved, 0, 11);
  assert.deepEqual([x, y], [80, -40]);
});

test('a pinch zooms by the fingers spread and keeps the point between them under them', () => {
  const view = fit(SHEET, { x: 0, y: 0, width: 390, height: 700 });
  const bounds = limits(view);
  const start = { view, a: [150, 300] as [number, number], b: [230, 300] as [number, number] };
  const under = toDrawing(view, 190, 300);
  // The fingers spread to three times as far apart, their middle moving left.
  const next = pinch(start, [70, 320], [310, 320], bounds);
  close(next.scale, view.scale * 3, 'three times as far apart');
  const [sx, sy] = toScreen(next, under[0], under[1]);
  close(sx, 190, 'the point stays between the fingers across');
  close(sy, 320, 'and down');
});

test('a view flung away is brought back to keep the paper in sight', () => {
  const size = { width: 400, height: 300 };
  const view = { left: 0, top: 11, scale: 20 };
  assert.deepEqual(keepInSight(view, SHEET, size), view, 'a view in sight is left alone');
  const away = panBy(view, -5000, 4000);
  const kept = keepInSight(away, SHEET, size, 60);
  const [right] = toScreen(kept, 17, 0);
  const [, top] = toScreen(kept, 0, 11);
  close(right, 60, 'the right edge 60 px in');
  close(top, 240, 'the top edge 60 px above the bottom');
});

test('a glide between views moves the zoom by equal ratios', () => {
  const a = { left: 0, top: 11, scale: 10 };
  const b = { left: 5, top: 6, scale: 40 };
  close(between(a, b, 0.5, [100, 100]).scale, 20, 'half way is the geometric middle');
  const end = between(a, b, 1, [100, 100]);
  close(end.left, b.left, 'ends at the target');
  close(end.top, b.top, 'ends at the target');
});
