import assert from 'node:assert/strict';
import { test } from 'node:test';

import { finishSheetGesture, SheetClickGuard, SWALLOW_MS } from '../src/sheet.ts';

test('tapping the grab toggles the sheet and swallows its trailing click (#204)', () => {
  const res = finishSheetGesture(false, true, false);
  assert.deepEqual(res, { swallowClick: true, toggle: true });

  const guard = new SheetClickGuard();
  guard.arm(1000);
  assert.equal(guard.take(1020), true, 'trailing click during slide is swallowed');
  assert.equal(guard.take(1030), false, 'only that one trailing click is swallowed');
});

test('a moved drag swallows its trailing click, and a new pointerdown clears it if no click arrived', () => {
  assert.deepEqual(finishSheetGesture(true, true, false), { swallowClick: true, toggle: false });
  assert.deepEqual(finishSheetGesture(true, false, false), { swallowClick: true, toggle: false });

  const guard = new SheetClickGuard();
  guard.arm(2000);
  // Long touch drag synthesized no click; user next taps a tab 100ms later.
  guard.clear();
  assert.equal(guard.take(2100), false, 'subsequent real tap is never swallowed');
});

test('tapping a peek button without dragging lets its click through', () => {
  assert.deepEqual(finishSheetGesture(false, false, false), { swallowClick: false, toggle: false });
});

test('a cancelled grab touch neither toggles the sheet nor swallows clicks', () => {
  assert.deepEqual(finishSheetGesture(false, true, true), { swallowClick: false, toggle: false });
});

test('an armed guard expires after the swallow window', () => {
  const guard = new SheetClickGuard();
  guard.arm(500);
  assert.equal(guard.take(500 + SWALLOW_MS), false);
});
